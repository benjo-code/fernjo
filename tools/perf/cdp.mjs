// CDP helper for tools/measure-performance.ps1 (Node 22+, global WebSocket).
// usage: node cdp.mjs usable <port> <startEpochMs> <timeoutMs>   -> prints ms from start to "tree area populated"
//        node cdp.mjs filter <port> <updates>                    -> prints JSON {firstMs, alternatingMs[]}
const [mode, portArg, a3, a4] = process.argv.slice(2);
const port = Number(portArg);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function connect(deadline) {
  for (;;) {
    if (Date.now() > deadline) throw new Error("no DevTools page before timeout");
    try {
      const list = await (await fetch(`http://127.0.0.1:${port}/json`)).json();
      const page = list.find((t) => t.type === "page" && t.webSocketDebuggerUrl);
      if (page) {
        const ws = new WebSocket(page.webSocketDebuggerUrl);
        await new Promise((res, rej) => {
          ws.onopen = res;
          ws.onerror = () => rej(new Error("ws"));
        });
        return ws;
      }
    } catch {
      /* app not up yet */
    }
    await sleep(10);
  }
}

function client(ws) {
  let id = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    }
  };
  return (method, params = {}) =>
    new Promise((res) => {
      const i = ++id;
      pending.set(i, res);
      ws.send(JSON.stringify({ id: i, method, params }));
    });
}

const evaluate = async (send, expression) => {
  const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails));
  return r.result?.result?.value;
};

// Tree pane only: rows inside the tree, or the empty-tree offer. The generic ".empty" class also
// marks placeholders of the main area and the properties panel, which exist as soon as the
// shell mounts, before the snapshot has loaded.
const POPULATED = `(() => {
  const rows = document.querySelectorAll('[role="tree"] [role="treeitem"]').length;
  const offer = [...document.querySelectorAll('p.empty')].some((p) => p.textContent.trim() === 'No connections yet');
  return rows > 0 || offer;
})()`;

if (mode === "usable") {
  const start = Number(a3);
  const deadline = start + Number(a4);
  const ws = await connect(deadline);
  const send = client(ws);
  for (;;) {
    if (Date.now() > deadline) throw new Error("tree area not populated before timeout");
    const ok = await evaluate(send, POPULATED).catch(() => false);
    if (ok) {
      console.log(String(Date.now() - start));
      break;
    }
    await sleep(5);
  }
  ws.close();
} else if (mode === "filter") {
  const updates = Number(a3);
  const ws = await connect(Date.now() + 10000);
  const send = client(ws);
  // Sets the filter text like a keystroke would and resolves with the time to the
  // second painted frame (M2's method).
  const expr = (q) => `new Promise((resolve) => {
    const input = document.querySelector('input[aria-label="Filter connections"]');
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    const t0 = performance.now();
    setter.call(input, ${JSON.stringify(q)});
    input.dispatchEvent(new Event('input', { bubbles: true }));
    requestAnimationFrame(() => requestAnimationFrame(() => resolve(performance.now() - t0)));
  })`;
  const first = await evaluate(send, expr("host"));
  const alt = [];
  for (let i = 0; i < updates; i++) {
    alt.push(await evaluate(send, expr(i % 2 === 0 ? "host-1" : "host")));
    await sleep(50);
  }
  await evaluate(send, expr(""));
  console.log(JSON.stringify({ firstMs: first, alternatingMs: alt }));
  ws.close();
} else {
  throw new Error("unknown mode");
}
