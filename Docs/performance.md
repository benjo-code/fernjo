# Performance measurements (spec section 6)

Targets: cold start to a usable window under 2 s, idle memory with no session under about 150 MB, a tree of 500 connections filters without visible lag. All three **met** on Windows (see the verdicts for the filter's cold first update); Linux and macOS are open owner checks (commands below).

## Definitions

- **Cold start:** from `Start-Process fernjo.exe` to the first moment the DevTools protocol reports the tree area populated (a `[role=tree]` with at least one `[role=treeitem]`, or the empty-tree offer) on a scratch `FERNJO_DATA_DIR` holding the 500-connection sample tree (`cargo run -p fernjo-core --example generate_tree -- <dir> 500`). Polled every 5 ms. The first start of a session can be slower on a never-run binary (disk cache, antivirus scan); these runs follow a build and a smoke run, so the binary is warm on disk.
- **Idle memory:** sum of private working sets (`Win32_PerfFormattedData_PerfProc_Process.WorkingSetPrivate`) 30 s after usable, no session open: `fernjoOnly` = `fernjo.exe` alone, `withWebView` = the whole process tree including the WebView2 children (7 processes). Private working set excludes shared pages (DLLs, shared WebView2 code), so it understates what Task Manager's "Working set" total shows; both readings are far below the target either way.
- **Filter:** M2's method: set the filter input text (native setter plus an `input` event), time to the second painted frame (two nested `requestAnimationFrame`), `firstMs` is the cold first update, `median` and `max` are over 20 alternating worst-case updates ("host-1" and "host", 50 ms apart).
- Script: `tools/measure-performance.ps1 -Exe target/release/fernjo.exe -Runs 10` (PowerShell, Windows; helper `tools/perf/cdp.mjs`, Node 22+). Idle memory and filter are measured in the last of the runs.

## Result (Windows, 3 sessions of 10 runs)

- Machine: AMD Ryzen 7 3800X 8-Core, 15.9 GB RAM, Windows 11 Pro build 10.0.26200.
- Binary: `pnpm tauri build --no-bundle` (release profile: strip, lto thin, codegen-units 1) built from commit `2965077` (the working tree held only the corrected probe selector in `tools/perf/cdp.mjs`, no app source change).
- Date: 2026-11-18.

| Session | Cold start median / max (ms) | Idle fernjo only / with WebView (MiB) | Filter median / max (ms), first cold update |
|---|---|---|---|
| 1 | 932 / 1091 | 3.7 / 69.5 | 32.7 / 70.9, 159.0 |
| 2 | 954 / 1092 | 3.1 / 60.8 | 30.7 / 59.0, 125.5 |
| 3 | 992 / 1025 | 3.6 / 67.2 | 29.5 / 55.0, 136.2 |

Verdicts:
- Cold start: medians 932, 954 and 992 ms, worst single run 1092 ms; target under 2000 ms: **met**.
- Idle memory: 3.1-3.7 MiB fernjo only, 60.8-69.5 MiB with WebView; target about 150 MB: **met** on both readings.
- Filter: median 29.5-32.7 ms and worst 70.9 ms over 20 alternating worst-case updates, no visible lag: **met**. The very first (cold) update after start took 125-159 ms; this is accepted as in M2's record, which had a 163 ms cold first keystroke and was judged no visible lag (`Docs/ROADMAP.md`, M2 acceptance record).

### Correction of an earlier reading

The first version of the probe also accepted any element with the class `.empty`. The main area ("Open a connection from the tree") and the properties panel render such a paragraph as soon as the shell mounts, so the probe fired before the tree snapshot had loaded. Measured side by side on the same build (three starts): the shell placeholder appeared at 1011, 903 and 883 ms, the first tree row at 1040, 939 and 920 ms, so the first version read about 35 ms low (medians 880, 858, 852 ms). The probe now accepts only rows inside `[role=tree]` or the tree pane's "No connections yet" offer; the numbers below are from that probe.

### Raw output

Session 1:

```json
{
    "coldStartMs":  {
                        "median":  932,
                        "max":  1091,
                        "runs":  [
                                     1091,
                                     909,
                                     937,
                                     913,
                                     986,
                                     900,
                                     932,
                                     919,
                                     932,
                                     944
                                 ]
                    },
    "idleMemoryMiB":  {
                          "fernjoOnly":  3.7,
                          "withWebView":  69.5,
                          "processes":  7
                      },
    "filter500Ms":  {
                        "median":  32.7,
                        "max":  70.9,
                        "firstMs":  159,
                        "updates":  20
                    }
}
```

Session 2:

```json
{
    "coldStartMs":  {
                        "median":  954,
                        "max":  1092,
                        "runs":  [
                                     1092,
                                     1028,
                                     938,
                                     884,
                                     948,
                                     927,
                                     1012,
                                     1019,
                                     915,
                                     959
                                 ]
                    },
    "idleMemoryMiB":  {
                          "fernjoOnly":  3.1,
                          "withWebView":  60.8,
                          "processes":  7
                      },
    "filter500Ms":  {
                        "median":  30.7,
                        "max":  59,
                        "firstMs":  125.5,
                        "updates":  20
                    }
}
```

Session 3:

```json
{
    "coldStartMs":  {
                        "median":  992,
                        "max":  1025,
                        "runs":  [
                                     995,
                                     910,
                                     991,
                                     1025,
                                     992,
                                     961,
                                     992,
                                     1009,
                                     966,
                                     957
                                 ]
                    },
    "idleMemoryMiB":  {
                          "fernjoOnly":  3.6,
                          "withWebView":  67.2,
                          "processes":  7
                      },
    "filter500Ms":  {
                        "median":  29.5,
                        "max":  55,
                        "firstMs":  136.2,
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
