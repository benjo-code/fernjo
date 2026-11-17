# Usage: pwsh tools/smoke-installed.ps1 -Installers <dir>
# Silently installs the NSIS installer (per user), starts the installed app, checks that its main
# window appears and that the user data in %APPDATA%\fernjo survives install and uninstall, then
# silently uninstalls. Prints one `ok:` or `FAIL:` line per check and exits 1 on the first failure.
# CI only: it installs into the current profile and writes %APPDATA%\fernjo\sentinel.txt, so it
# refuses to run outside GitHub Actions (a throw-away hosted runner).
param([Parameter(Mandatory = $true)][string]$Installers)

$ErrorActionPreference = 'Stop'

function Ok([string]$msg) { Write-Host "ok: $msg" }
function Fail([string]$msg) { Write-Host "FAIL: $msg"; exit 1 }

if ($env:GITHUB_ACTIONS -ne 'true') {
    Fail "refusing to run outside GitHub Actions (GITHUB_ACTIONS is not 'true'); this script installs into the current user profile"
}

# Where did the installer put fernjo.exe? Prefer the uninstall registry entry, fall back to the
# usual per-user folders. Prints what it finds so a wrong guess is visible in the CI log.
function Find-Install {
    $keys = Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue
    foreach ($k in $keys) {
        $p = Get-ItemProperty -Path $k.PSPath -ErrorAction SilentlyContinue
        if ($null -eq $p -or -not $p.DisplayName -or $p.DisplayName -notlike '*fernjo*') { continue }
        Write-Host "registry entry $($k.PSChildName): DisplayName='$($p.DisplayName)' InstallLocation='$($p.InstallLocation)' UninstallString='$($p.UninstallString)'"
        $dir = $null
        if ($p.InstallLocation) { $dir = ([string]$p.InstallLocation).Trim('"') }
        elseif ($p.UninstallString) { $dir = Split-Path -Parent (([string]$p.UninstallString).Trim('"')) }
        if ($dir -and (Test-Path (Join-Path $dir 'fernjo.exe'))) { return $dir }
    }
    foreach ($c in @((Join-Path $env:LOCALAPPDATA 'fernjo'), (Join-Path $env:LOCALAPPDATA 'Programs\fernjo'))) {
        if (Test-Path (Join-Path $c 'fernjo.exe')) { return $c }
    }
    return $null
}

try {
    $setups = @(Get-ChildItem -Path $Installers -Filter 'fernjo_*_x64-setup.exe' -File)
    if ($setups.Count -ne 1) { Fail "expected one fernjo_*_x64-setup.exe in $Installers, found $($setups.Count)" }
    $setup = $setups[0].FullName

    # User data that must survive install and uninstall.
    $dataDir = Join-Path $env:APPDATA 'fernjo'
    $sentinel = Join-Path $dataDir 'sentinel.txt'
    New-Item -ItemType Directory -Force -Path $dataDir | Out-Null
    Set-Content -Path $sentinel -Value 'fernjo smoke test sentinel' -Encoding ascii
    $sentinelHash = (Get-FileHash -Path $sentinel -Algorithm SHA256).Hash
    Ok "sentinel created at $sentinel"

    Start-Process -FilePath $setup -ArgumentList '/S' -Wait
    Ok "silent install ran: $(Split-Path -Leaf $setup)"

    $installDir = Find-Install
    if (-not $installDir) {
        Write-Host 'LOCALAPPDATA contents:'
        Get-ChildItem $env:LOCALAPPDATA -Directory | ForEach-Object { Write-Host "  $($_.FullName)" }
        Fail 'fernjo.exe not found after install (checked the uninstall registry key and %LOCALAPPDATA% candidates)'
    }
    Write-Host "installed to: $installDir"
    $exe = Join-Path $installDir 'fernjo.exe'
    if (-not $installDir.StartsWith($env:LOCALAPPDATA, [StringComparison]::OrdinalIgnoreCase)) {
        Fail "install dir $installDir is not under %LOCALAPPDATA% (expected a per-user install)"
    }
    Ok "fernjo.exe installed per user: $exe"

    if (-not (Test-Path $sentinel)) { Fail 'sentinel vanished after install' }
    if ((Get-FileHash -Path $sentinel -Algorithm SHA256).Hash -ne $sentinelHash) { Fail 'sentinel changed after install' }
    Ok 'sentinel unchanged after install'

    # Start the app against a scratch data folder so it never touches %APPDATA%\fernjo.
    $scratch = Join-Path ([IO.Path]::GetTempPath()) ('fernjo-smoke-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $scratch | Out-Null
    $env:FERNJO_DATA_DIR = $scratch
    $app = Start-Process -FilePath $exe -PassThru
    Start-Sleep -Seconds 15
    $title = $null
    for ($i = 0; $i -lt 15; $i++) {
        $hit = @(Get-Process -Name fernjo -ErrorAction SilentlyContinue) |
            Where-Object { $_.MainWindowTitle -eq 'fernjo' } | Select-Object -First 1
        if ($hit) { $title = $hit.MainWindowTitle; break }
        Start-Sleep -Seconds 1
    }
    if (-not $title) {
        $seen = @(Get-Process -Name fernjo -ErrorAction SilentlyContinue | ForEach-Object { "pid $($_.Id) title '$($_.MainWindowTitle)'" })
        Fail "no fernjo process with MainWindowTitle 'fernjo' after 15 s (seen: $($seen -join '; '))"
    }
    Ok "main window 'fernjo' present after 15 s"

    Get-Process -Name fernjo -ErrorAction SilentlyContinue | Stop-Process -Force
    $app.WaitForExit(15000) | Out-Null
    Remove-Item Env:\FERNJO_DATA_DIR
    Remove-Item -Recurse -Force -Path $scratch -ErrorAction SilentlyContinue

    $uninstaller = Join-Path $installDir 'uninstall.exe'
    if (-not (Test-Path $uninstaller)) { Fail "uninstall.exe missing in $installDir" }
    Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait
    # The NSIS uninstaller re-launches itself from a temp copy, so -Wait can return early.
    $gone = $false
    for ($i = 0; $i -lt 60; $i++) {
        if (-not (Test-Path $exe) -and -not (Get-Process -Name fernjo -ErrorAction SilentlyContinue)) { $gone = $true; break }
        Start-Sleep -Seconds 1
    }
    if (-not $gone) { Fail 'fernjo.exe or a fernjo process still present 60 s after the silent uninstall' }
    Ok 'silent uninstall removed fernjo.exe and left no fernjo process'

    if (-not (Test-Path $sentinel)) { Fail 'sentinel vanished after uninstall' }
    if ((Get-FileHash -Path $sentinel -Algorithm SHA256).Hash -ne $sentinelHash) { Fail 'sentinel changed after uninstall' }
    Ok 'sentinel unchanged after uninstall'
}
catch {
    Fail "unexpected error: $($_.Exception.Message)"
}
