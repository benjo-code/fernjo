<#
.SYNOPSIS
  Measures the spec section 6 performance targets of a fernjo binary on Windows.
.DESCRIPTION
  Prints one JSON object: coldStartMs (median, max, runs[]), idleMemoryMiB
  (fernjoOnly, withWebView) and filter500Ms (median, max, firstMs). Definitions are in
  Docs/performance.md. Uses a scratch FERNJO_DATA_DIR (never the real data folder), drives
  the page over the WebView2 DevTools port (no OS mouse or keyboard) and stops every
  process it starts. Needs Node 22+ (global WebSocket) and, if -TreeDir is not given,
  cargo (to build the 500-connection sample tree).
.EXAMPLE
  pwsh tools/measure-performance.ps1 -Exe target/release/fernjo.exe -Runs 10
#>
param(
  [Parameter(Mandatory)][string]$Exe,
  [int]$Runs = 10,
  [int]$Port = 9222,
  [int]$IdleSeconds = 30,
  [int]$FilterUpdates = 20,
  [string]$TreeDir
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$Exe = (Resolve-Path $Exe).Path
$helper = Join-Path $PSScriptRoot 'perf/cdp.mjs'

function Stop-Fernjo {
  Get-Process fernjo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 800
}
if (Get-Process fernjo -ErrorAction SilentlyContinue) { throw 'fernjo.exe is already running; close it first' }

if (-not $TreeDir) {
  $TreeDir = Join-Path ([IO.Path]::GetTempPath()) ("fernjo-perf-" + [guid]::NewGuid().ToString('N'))
  Push-Location $repo
  try { cargo run -q -p fernjo-core --example generate_tree -- $TreeDir 500 | Out-Null } finally { Pop-Location }
}
$tree = (Resolve-Path $TreeDir).Path

function Get-Descendants([int]$RootPid) {
  $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId
  $ids = [System.Collections.Generic.HashSet[int]]::new()
  [void]$ids.Add($RootPid)
  do {
    $before = $ids.Count
    foreach ($p in $all) { if ($ids.Contains([int]$p.ParentProcessId)) { [void]$ids.Add([int]$p.ProcessId) } }
  } while ($ids.Count -ne $before)
  $ids
}

function Get-PrivateMiB($ids) {
  $sum = 0.0
  foreach ($p in Get-CimInstance Win32_PerfFormattedData_PerfProc_Process) {
    if ($ids.Contains([int]$p.IDProcess)) { $sum += $p.WorkingSetPrivate / 1MB }
  }
  [math]::Round($sum, 1)
}

function Get-Median([double[]]$v) {
  $s = $v | Sort-Object
  $n = $s.Count
  if ($n % 2) { $s[($n - 1) / 2] } else { ($s[$n / 2 - 1] + $s[$n / 2]) / 2 }
}

$env:FERNJO_DATA_DIR = $tree
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"

$cold = @()
$idle = $null
$filter = $null
try {
  for ($i = 1; $i -le $Runs; $i++) {
    $start = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    $app = Start-Process -FilePath $Exe -PassThru
    $out = node $helper usable $Port $start 20000
    if ($LASTEXITCODE -ne 0) { throw "run ${i}: app never became usable" }
    $cold += [double]$out
    if ($i -eq $Runs) {
      Start-Sleep -Seconds $IdleSeconds
      $ids = Get-Descendants $app.Id
      $only = Get-PrivateMiB ([System.Collections.Generic.HashSet[int]]::new([int[]]@($app.Id)))
      $with = Get-PrivateMiB $ids
      $idle = [ordered]@{ fernjoOnly = $only; withWebView = $with; processes = $ids.Count }
      $f = node $helper filter $Port $FilterUpdates | ConvertFrom-Json
      $alt = [double[]]$f.alternatingMs
      $filter = [ordered]@{
        median = [math]::Round((Get-Median $alt), 1)
        max = [math]::Round(($alt | Measure-Object -Maximum).Maximum, 1)
        firstMs = [math]::Round($f.firstMs, 1)
        updates = $alt.Count
      }
    }
    Stop-Fernjo
  }
} finally {
  Stop-Fernjo
  Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS -ErrorAction SilentlyContinue
  Remove-Item Env:FERNJO_DATA_DIR -ErrorAction SilentlyContinue
}

[ordered]@{
  coldStartMs = [ordered]@{
    median = [math]::Round((Get-Median $cold), 0)
    max = ($cold | Measure-Object -Maximum).Maximum
    runs = $cold
  }
  idleMemoryMiB = $idle
  filter500Ms = $filter
} | ConvertTo-Json -Depth 4
