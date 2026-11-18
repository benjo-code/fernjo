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

# The HKCU uninstall entries of fernjo (empty once the uninstaller has cleaned up).
function Get-FernjoUninstallKeys {
    $found = @()
    $keys = Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' -ErrorAction SilentlyContinue
    foreach ($k in $keys) {
        $p = Get-ItemProperty -Path $k.PSPath -ErrorAction SilentlyContinue
        if ($null -ne $p -and $p.DisplayName -and $p.DisplayName -like '*fernjo*') { $found += $k.PSChildName }
    }
    return $found
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

$scratch = $null
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

    $inst = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
    if ($inst.ExitCode -ne 0) { Fail "installer exited with code $($inst.ExitCode)" }
    Ok "silent install ran (exit code 0): $(Split-Path -Leaf $setup)"

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
        Fail "no fernjo process with MainWindowTitle 'fernjo' within 30 s of starting (15 s wait, then polling for up to 15 s; seen: $($seen -join '; '))"
    }
    Ok "main window 'fernjo' present after the 15 s wait (found within 30 s at most)"

    Get-Process -Name fernjo -ErrorAction SilentlyContinue | Stop-Process -Force
    $app.WaitForExit(15000) | Out-Null

    $uninstaller = Join-Path $installDir 'uninstall.exe'
    if (-not (Test-Path $uninstaller)) { Fail "uninstall.exe missing in $installDir" }
    $un = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
    # The NSIS uninstaller re-launches itself from a temp copy and exits at once, so its exit
    # code says nothing about the real uninstall: print it, do not judge it.
    Write-Host "uninstaller stub exit code: $($un.ExitCode)"
    # Wait for the end state, not for the first sign of progress: the exe goes first, the rest
    # of the files, the registry entry and any data deletion come after.
    $left = @()
    for ($i = 0; $i -lt 90; $i++) {
        $left = @()
        if (Test-Path $exe) { $left += "file $exe" }
        if (Test-Path $uninstaller) { $left += "file $uninstaller" }
        # An empty leftover directory is fine; remaining files are not.
        if ((Test-Path $installDir) -and @(Get-ChildItem -Path $installDir -Recurse -Force -File -ErrorAction SilentlyContinue).Count -gt 0) { $left += "files under $installDir" }
        if (Get-FernjoUninstallKeys) { $left += 'HKCU uninstall registry entry' }
        if (Get-Process -Name fernjo -ErrorAction SilentlyContinue) { $left += 'fernjo process' }
        if ($left.Count -eq 0) { break }
        Start-Sleep -Seconds 1
    }
    if ($left.Count -ne 0) { Fail "still present 90 s after the silent uninstall: $($left -join ', ')" }
    Ok 'silent uninstall finished: install directory, registry entry and fernjo process are gone'

    if (-not (Test-Path $sentinel)) { Fail 'sentinel vanished after uninstall' }
    if ((Get-FileHash -Path $sentinel -Algorithm SHA256).Hash -ne $sentinelHash) { Fail 'sentinel changed after uninstall' }
    Ok 'sentinel unchanged after uninstall'
}
catch {
    Fail "unexpected error: $($_.Exception.Message)"
}
finally {
    # Also runs on the exit inside Fail. The sentinel stays by design (throw-away runner).
    Get-Process -Name fernjo -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item Env:\FERNJO_DATA_DIR -ErrorAction SilentlyContinue
    if ($scratch) { Remove-Item -Recurse -Force -Path $scratch -ErrorAction SilentlyContinue }
}
