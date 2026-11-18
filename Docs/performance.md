# Performance measurements (spec section 6)

Targets: cold start to a usable window under 2 s, idle memory with no session under about 150 MB, a tree of 500 connections filters without visible lag. All three **met** on Windows; Linux and macOS are open owner checks (commands below).

## Definitions

- **Cold start:** from `Start-Process fernjo.exe` to the first moment the DevTools protocol reports the tree area populated (a `[role=tree]` with at least one `[role=treeitem]`, or the empty-tree offer) on a scratch `FERNJO_DATA_DIR` holding the 500-connection sample tree (`cargo run -p fernjo-core --example generate_tree -- <dir> 500`). Polled every 5 ms. The first start of a session can be slower on a never-run binary (disk cache, antivirus scan); these runs follow a build and a smoke run, so the binary is warm on disk.
- **Idle memory:** sum of private working sets (`Win32_PerfFormattedData_PerfProc_Process.WorkingSetPrivate`) 30 s after usable, no session open: `fernjoOnly` = `fernjo.exe` alone, `withWebView` = the whole process tree including the WebView2 children (7 processes). Private working set excludes shared pages (DLLs, shared WebView2 code), so it understates what Task Manager's "Working set" total shows; both readings are far below the target either way.
- **Filter:** M2's method: set the filter input text (native setter plus an `input` event), time to the second painted frame (two nested `requestAnimationFrame`), `firstMs` is the cold first update, `median` and `max` are over 20 alternating worst-case updates ("host-1" and "host", 50 ms apart).
- Script: `tools/measure-performance.ps1 -Exe target/release/fernjo.exe -Runs 10` (PowerShell, Windows; helper `tools/perf/cdp.mjs`, Node 22+). Idle memory and filter are measured in the last of the runs.

## Result (Windows, 3 sessions of 10 runs)

- Machine: AMD Ryzen 7 3800X 8-Core, 15.9 GB RAM, Windows 11 Pro build 10.0.26200.
- Binary: `pnpm tauri build --no-bundle` (release profile: strip, lto thin, codegen-units 1) of commit `eb5404e` plus the measurement script.
- Date: 2026-11-18.

| Session | Cold start median / max (ms) | Idle fernjo only / with WebView (MiB) | Filter median / max (ms), first |
|---|---|---|---|
| 1 | 880 / 950 | 3.6 / 67.3 | 29.4 / 49.2, 120.1 |
| 2 | 858 / 916 | 3.5 / 67.4 | 28.3 / 53.4, 123.4 |
| 3 | 852 / 924 | 3.6 / 69.7 | 32.2 / 49.5, 115.6 |

Medians: cold start about 858 ms (target under 2000 ms: met), idle memory 3.6 MiB fernjo only and about 67 MiB with WebView (target about 150: met), filter median about 29 ms, worst 53 ms (no visible lag: met; M2 measured median 61 ms on the dev build).

### Raw output

Session 1:

```json
{
    "coldStartMs":  {
                        "median":  880,
                        "max":  950,
                        "runs":  [
                                     950,
                                     897,
                                     845,
                                     888,
                                     881,
                                     853,
                                     879,
                                     836,
                                     856,
                                     881
                                 ]
                    },
    "idleMemoryMiB":  {
                          "fernjoOnly":  3.6,
                          "withWebView":  67.3,
                          "processes":  7
                      },
    "filter500Ms":  {
                        "median":  29.4,
                        "max":  49.2,
                        "firstMs":  120.1,
                        "updates":  20
                    }
}
```

Session 2:

```json
{
    "coldStartMs":  {
                        "median":  858,
                        "max":  916,
                        "runs":  [
                                     916,
                                     877,
                                     848,
                                     838,
                                     868,
                                     857,
                                     860,
                                     831,
                                     838,
                                     901
                                 ]
                    },
    "idleMemoryMiB":  {
                          "fernjoOnly":  3.5,
                          "withWebView":  67.4,
                          "processes":  7
                      },
    "filter500Ms":  {
                        "median":  28.3,
                        "max":  53.4,
                        "firstMs":  123.4,
                        "updates":  20
                    }
}
```

Session 3:

```json
{
    "coldStartMs":  {
                        "median":  852,
                        "max":  924,
                        "runs":  [
                                     924,
                                     902,
                                     846,
                                     865,
                                     867,
                                     835,
                                     835,
                                     856,
                                     849,
                                     849
                                 ]
                    },
    "idleMemoryMiB":  {
                          "fernjoOnly":  3.6,
                          "withWebView":  69.7,
                          "processes":  7
                      },
    "filter500Ms":  {
                        "median":  32.2,
                        "max":  49.5,
                        "firstMs":  115.6,
                        "updates":  20
                    }
}
```

## Linux and macOS (owner-run, open)

Build `pnpm tauri build --no-bundle`, use a scratch `FERNJO_DATA_DIR` with the 500-connection sample tree, no session open.

- Cold start: launch and watch for the tree to show, timing with a stopwatch or screen recording; target under 2 s. Linux also: `/usr/bin/time -v target/release/fernjo` gives wall time and peak RSS (close the window right after the tree appears).
- Idle memory, Linux: 30 s after the tree is shown, `ps -o rss= -p $(pgrep -d, -f 'fernjo|WebKit')` summed (RSS in KiB, includes shared pages, so expect more than the Windows private figure); target about 150 MB.
- Idle memory, macOS: `footprint -p fernjo` (physical footprint of the app) plus `footprint -p "$(pgrep -f 'com.apple.WebKit.WebContent' | head -1)"` for the WebKit helper.
- Filter: type into the filter box on the 500-connection tree; there must be no visible lag (the DevTools-port recipe is Windows-only; on Linux/macOS use the Web Inspector from a debug build).
