# M8 render-path spike (throwaway, branch `spike/m08-rdp-canvas`, never merged)

Question: can the canvas path (IronRDP framebuffer in Rust, region messages on a Tauri channel,
`putImageData` in the webview) reach **at least 30 painted frames per second while scrolling and
dragging, on every OS**, and does typing feel fast enough? (Main spec 5.6; gate in the M8 plan,
Task 8.)

## Part 1: the webview path, on every OS (Windows 11 PC, macOS laptop, CachyOS laptop)

No RDP host needed. The page paints synthetic 1920x1080 region messages that Rust copies and sends
on a channel, one acknowledged message at a time (and optionally two or three in flight).

1. Build: `pnpm install` then `pnpm tauri build --no-bundle` on that machine (Windows needs the
   WebView2 runtime, `winget install Microsoft.EdgeWebView2Runtime`).
2. Start the binary (`target/release/fernjo` or `fernjo.exe`) with a scratch data folder
   (`FERNJO_DATA_DIR=<empty folder>`) and press **Ctrl+Shift+F12** in the window: it switches to the
   spike page (`#spike`). In a `pnpm tauri dev` run the same key works.
3. Press "Run all three modes" with 1 in flight, write down the table (painted fps per mode, paint
   ms per message, ms between messages, MB/s). Repeat with 2 and 3 in flight if a mode is under 30.
4. Note the OS, the machine, the webview version (the page prints the user agent).

## Part 2: what IronRDP delivers, from one machine against one Windows host (NLA on)

```
FERNJO_TEST_RDP_ADDR=<windows host>:3389 FERNJO_TEST_RDP_USER=<user> \
FERNJO_TEST_RDP_PASSWORD=<password> [FERNJO_TEST_RDP_DOMAIN=<domain>] \
cargo run -p fernjo-rdp --release --example spike_bench
```

It scripts its own load (Notepad with scrolling text, window snapping with Win+arrows, Win+D) for
10 s per phase (`FERNJO_BENCH_SECONDS`) and prints messages per second, mean and p95 message size,
MB/s and the Rust-side copy cost per message. Run it once at 1920x1080. (The host's desktop will
flicker; use a test session.)

## Latency judgment

Keystroke latency cannot be measured by the script: connect with the OS's own RDP client to the
same host and type, then compare with the typing feel of the finished tab once it exists. For the
gate the owner's call on the **message rate and size** numbers is what counts; the final latency
check is in the milestone's manual checks.
