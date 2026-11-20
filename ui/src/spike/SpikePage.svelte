<script lang="ts">
  // THROWAWAY (branch spike/m08-rdp-canvas, never merged): paints synthetic RDP-like region
  // messages from the Rust side and reports the painted frames per second per mode.
  import { Channel, invoke } from "@tauri-apps/api/core";

  const W = 1920;
  const H = 1080;
  const SECONDS = 10;
  const MODES = ["full", "scroll", "drag"] as const;

  interface Row {
    mode: string;
    inFlight: number;
    fps: number;
    paintMs: number;
    arrivalMs: number;
    mbPerSecond: number;
    frames: number;
  }

  let canvas: HTMLCanvasElement | undefined = $state();
  let rows = $state<Row[]>([]);
  let running = $state(false);
  let status = $state("idle");
  let inFlight = $state(1);

  async function runMode(mode: string, flight: number): Promise<Row> {
    const ctx = canvas!.getContext("2d")!;
    let frames = 0;
    let bytes = 0;
    let paintSum = 0;
    let arrivalSum = 0;
    let lastArrival = 0;
    const channel = new Channel<ArrayBuffer>();
    channel.onmessage = (buf) => {
      const t0 = performance.now();
      if (lastArrival) arrivalSum += t0 - lastArrival;
      lastArrival = t0;
      const head = new DataView(buf);
      const x = head.getUint16(0, true);
      const y = head.getUint16(2, true);
      const w = head.getUint16(4, true);
      const h = head.getUint16(6, true);
      ctx.putImageData(new ImageData(new Uint8ClampedArray(buf, 8, w * h * 4), w, h), x, y);
      paintSum += performance.now() - t0;
      frames++;
      bytes += buf.byteLength;
      void invoke("spike_ack");
    };
    await invoke("spike_stream", {
      onMessage: channel,
      width: W,
      height: H,
      mode,
      seconds: SECONDS,
      inFlight: flight,
    });
    await new Promise((r) => setTimeout(r, 300));
    return {
      mode,
      inFlight: flight,
      fps: Math.round((frames / SECONDS) * 10) / 10,
      paintMs: Math.round((paintSum / Math.max(frames, 1)) * 100) / 100,
      arrivalMs: Math.round((arrivalSum / Math.max(frames - 1, 1)) * 100) / 100,
      mbPerSecond: Math.round(bytes / SECONDS / 1048576),
      frames,
    };
  }

  async function runAll() {
    running = true;
    rows = [];
    for (const mode of MODES) {
      status = `running ${mode} (${SECONDS} s, ${inFlight} in flight)`;
      rows = [...rows, await runMode(mode, inFlight)];
    }
    status = "done";
    running = false;
  }
</script>

<main style="padding: 12px; font-family: sans-serif">
  <h1 style="font-size: 16px">RDP canvas path spike (throwaway)</h1>
  <p>
    {W}x{H} canvas, {SECONDS} s per mode. In flight:
    <select bind:value={inFlight} disabled={running}>
      <option value={1}>1</option>
      <option value={2}>2</option>
      <option value={3}>3</option>
    </select>
    <button onclick={runAll} disabled={running}>Run all three modes</button>
    <span>{status}</span>
  </p>
  <table border="1" cellpadding="4" style="border-collapse: collapse">
    <thead>
      <tr>
        <th>mode</th><th>in flight</th><th>painted fps</th><th>paint ms/msg</th>
        <th>ms between msgs</th><th>MB/s</th><th>frames</th>
      </tr>
    </thead>
    <tbody>
      {#each rows as r (r.mode)}
        <tr>
          <td>{r.mode}</td><td>{r.inFlight}</td><td>{r.fps}</td><td>{r.paintMs}</td>
          <td>{r.arrivalMs}</td><td>{r.mbPerSecond}</td><td>{r.frames}</td>
        </tr>
      {/each}
    </tbody>
  </table>
  <pre>{JSON.stringify({ userAgent: navigator.userAgent, rows }, null, 1)}</pre>
  <canvas bind:this={canvas} width={W} height={H} style="width: 480px; border: 1px solid #888"
  ></canvas>
</main>
