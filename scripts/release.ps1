<#
.SYNOPSIS
    Build DJLED as something that lives on this machine, and optionally install it.

.DESCRIPTION
    Three things happen here and the order is load-bearing:

      1. the editor is built with vite into ui/dist
      2. the engine is built, and `engine/build.rs` bakes ui/dist into the exe
      3. the result is staged in dist/, and with -Install copied somewhere
         permanent and wired to start with the machine

    Step 2 reads what step 1 wrote, so an engine built against a stale ui/dist
    serves a stale editor with no sign that it has. That is the entire reason
    this is one script rather than two commands in a README.

    Two binaries come out of it, and they are the same program:

      djled.exe   console. Every diagnostic flag, the terminal bar display,
                  ctrl-c to stop. What you run while working on a show.
      djledw.exe  no console at all. Puts an icon in the notification area,
                  serves the editor on a port the OS picks, and writes what it
                  would have printed to %LOCALAPPDATA%\DJLED\djled.log. What
                  gets installed.

.PARAMETER Install
    Copy the build to %LOCALAPPDATA%\Programs\DJLED, add a Start Menu shortcut,
    and register it to start at logon. Replaces whatever was there, stopping a
    running copy first — a running exe cannot be overwritten on Windows.

.PARAMETER Uninstall
    Undo all of that: stop it, drop the autostart entry, remove the shortcut and
    the folder. Leaves the presets and the log alone, because they are yours and
    an uninstall is not a request to lose twelve shows.

.PARAMETER Port
    The serial port to drive, e.g. COM6. This exists rather than leaving it to
    -EngineArgs because `powershell -File` does not parse PowerShell syntax:
    -EngineArgs '--port','COM6' arrives as the single literal string
    "--port,COM6", the engine rejects it, and an installed copy started that way
    exits the instant it launches. A single -Port value cannot be mangled.

.PARAMETER EngineArgs
    Anything else the installed copy should be started with, e.g. --bands 64.
    Baked into the autostart entry and the shortcut alongside -Port. Commas and
    spaces both separate arguments here, so that the -File mangling above cannot
    produce a broken entry; an argument that has to contain a space (a presets
    path, say) needs the engine started directly rather than through this.

    Omit both and it runs with no hardware — a perfectly good way to check the
    editor comes up, and exactly what the strip staying dark means.

.PARAMETER NoAutostart
    Install, but do not start it at logon. The Start Menu shortcut still works.

.PARAMETER Start
    Start the installed copy when the install finishes, rather than at next
    logon.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\release.ps1
    Build only. The result is in dist\.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\release.ps1 -Install -Start -EngineArgs '--port','COM3'
    Build, install, start it now and at every logon, driving the strip on COM3.
#>

[CmdletBinding()]
param(
    [switch]$Install,
    [switch]$Uninstall,
    [switch]$NoAutostart,
    [switch]$Start,
    [string]$Port = '',
    [string[]]$EngineArgs = @()
)

$ErrorActionPreference = 'Stop'

$Root      = Split-Path -Parent $PSScriptRoot
$Dist      = Join-Path $Root 'dist'
$Target    = Join-Path $env:LOCALAPPDATA 'Programs\DJLED'
$RunKey    = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$RunName   = 'DJLED'
$StartMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\DJLED.lnk'

function Write-Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }
function Write-Note($text) { Write-Host "    $text" -ForegroundColor DarkGray }
function Write-Warn($text) { Write-Host "    $text" -ForegroundColor Yellow }

$LogFile = Join-Path $env:LOCALAPPDATA 'DJLED\djled.log'

# Everything the installed copy is started with, as separate arguments.
#
# Split on commas as well as spaces because `powershell -File` does not parse
# PowerShell: -EngineArgs '--port','COM6' arrives here as one literal string
# with a comma in it, the engine rejects the whole thing, and what you get is a
# service that exits the moment it starts with nothing on screen to say so.
# That failure already happened once; this is why -Port exists.
function Resolve-EngineArgs {
    $resolved = @()
    if ($Port -ne '') { $resolved += @('--port', $Port) }
    foreach ($item in $EngineArgs) {
        if ($null -eq $item) { continue }
        foreach ($piece in ($item -split '[,\s]+')) {
            if ($piece -ne '') { $resolved += $piece }
        }
    }
    return ,$resolved
}

# The last run's own paragraph of the log. The file is appended to across runs,
# so the interesting part is always the tail after the final banner.
function Get-LastRun {
    if (-not (Test-Path $LogFile)) { return @() }
    # UTF-8 said out loud: Windows PowerShell reads as ANSI by default, and
    # the engine's output is full of em dashes that come back as mojibake —
    # which reads as corruption in the one output somebody turns to when
    # something is already wrong.
    $lines = @(Get-Content $LogFile -Encoding UTF8)
    $banner = $lines | Select-String -SimpleMatch '=== djled started ===' | Select-Object -Last 1
    if ($banner) { return $lines[($banner.LineNumber - 1)..($lines.Count - 1)] }
    return $lines
}

# A running copy holds its own exe open, so every path that writes into the
# install directory goes through this first. Both names, because a release is
# two binaries and either one could be the one running.
function Stop-DJLED {
    $running = Get-Process -Name 'djled', 'djledw' -ErrorAction SilentlyContinue
    if ($running) {
        Write-Step 'Stopping the running copy'
        $running | Stop-Process -Force
        # The tray icon and the ports go with the process, but not instantly.
        Start-Sleep -Milliseconds 700
    }
}

function Invoke-Uninstall {
    Stop-DJLED

    if (Get-ItemProperty -Path $RunKey -Name $RunName -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -Path $RunKey -Name $RunName
        Write-Note 'autostart entry removed'
    }
    if (Test-Path $StartMenu) {
        Remove-Item $StartMenu -Force
        Write-Note 'Start Menu shortcut removed'
    }
    if (Test-Path $Target) {
        Remove-Item $Target -Recurse -Force
        Write-Note "removed $Target"
    }

    Write-Host ''
    Write-Host 'DJLED is uninstalled.' -ForegroundColor Green
    Write-Note "presets are still at $env:APPDATA\djled\presets.json"
    Write-Note "the log is still at $env:LOCALAPPDATA\DJLED\djled.log"
}

if ($Uninstall) {
    Invoke-Uninstall
    exit 0
}

# ---------------------------------------------------------------- build the UI

Write-Step 'Building the editor'
Push-Location (Join-Path $Root 'ui')
try {
    if (-not (Test-Path 'node_modules')) {
        Write-Note 'node_modules is missing; installing'
        npm install --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw 'npm install failed' }
    }
    # `npm run build` is tsc --noEmit and then vite build, so a type error stops
    # the release here rather than shipping a bundle that fails at runtime.
    npm run build
    if ($LASTEXITCODE -ne 0) { throw 'the editor did not build' }
} finally {
    Pop-Location
}

$indexFile = Join-Path $Root 'ui\dist\index.html'
if (-not (Test-Path $indexFile)) { throw "ui\dist\index.html is missing after the build" }

# ------------------------------------------------------------ build the engine

Write-Step 'Building the engine'
Write-Note 'ui\dist is baked into the exe by engine\build.rs'
cargo build --manifest-path (Join-Path $Root 'engine\Cargo.toml') --release
if ($LASTEXITCODE -ne 0) { throw 'the engine did not build' }

$built = Join-Path $Root 'engine\target\release'
$exes = @('djled.exe', 'djledw.exe')
foreach ($exe in $exes) {
    if (-not (Test-Path (Join-Path $built $exe))) { throw "$exe is missing after the build" }
}

# ------------------------------------------------------------------- stage it

Write-Step "Staging in $Dist"
if (Test-Path $Dist) { Remove-Item $Dist -Recurse -Force }
New-Item -ItemType Directory -Path $Dist | Out-Null
foreach ($exe in $exes) { Copy-Item (Join-Path $built $exe) $Dist }
Copy-Item (Join-Path $Root 'docs\wiring.md') $Dist -ErrorAction SilentlyContinue

foreach ($exe in $exes) {
    $size = [math]::Round((Get-Item (Join-Path $Dist $exe)).Length / 1MB, 1)
    Write-Note "$exe  ${size} MB"
}

if (-not $Install) {
    Write-Host ''
    Write-Host 'Built.' -ForegroundColor Green
    Write-Note "run dist\djledw.exe for the tray, or dist\djled.exe for a terminal"
    Write-Note 'add -Install to put it in %LOCALAPPDATA% and start it at logon'
    exit 0
}

# --------------------------------------------------------------------- install

Stop-DJLED

Write-Step "Installing to $Target"
if (-not (Test-Path $Target)) { New-Item -ItemType Directory -Path $Target | Out-Null }
foreach ($exe in $exes) { Copy-Item (Join-Path $Dist $exe) $Target -Force }

$windowed = Join-Path $Target 'djledw.exe'
$resolved = Resolve-EngineArgs

# Said out loud, because the difference between a strip that lights and one
# that does not is entirely in this line, and it is the line nobody sees.
if ($resolved.Count -gt 0) {
    Write-Note "arguments: $($resolved -join ' ')"
} else {
    Write-Warn 'no arguments: this will run on a mock link and drive nothing.'
    Write-Warn 'pass -Port COM6 to drive the strip.'
}

# The board is asked about before anything is registered. A port that is not
# there is not fatal — it may simply be unplugged this minute — but an
# installed copy that exits at every logon because of a typo is the exact
# failure this whole check exists to catch.
if ($Port -ne '') {
    $ports = & (Join-Path $Target 'djled.exe') --list-ports 2>&1
    if ($ports -match [regex]::Escape($Port)) {
        Write-Note "$Port is there"
    } else {
        Write-Warn "$Port is not among the serial ports visible right now:"
        foreach ($line in $ports) { Write-Warn "  $line" }
    }
}

# Quoted whatever the path looks like: %LOCALAPPDATA% contains the user's name
# and a space in it would otherwise split the command in two.
$command = '"{0}"' -f $windowed
if ($resolved.Count -gt 0) { $command = '{0} {1}' -f $command, ($resolved -join ' ') }

if ($NoAutostart) {
    if (Get-ItemProperty -Path $RunKey -Name $RunName -ErrorAction SilentlyContinue) {
        Remove-ItemProperty -Path $RunKey -Name $RunName
    }
    Write-Note 'autostart not registered (-NoAutostart)'
} else {
    # The Run key rather than a scheduled task: it runs as the logged-in user in
    # their own session, which is the only thing a tray icon and WASAPI loopback
    # can be. A task set to run "whether logged on or not" would have neither.
    New-ItemProperty -Path $RunKey -Name $RunName -Value $command -PropertyType String -Force | Out-Null
    Write-Note "starts at logon: $command"
}

$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($StartMenu)
$shortcut.TargetPath = $windowed
$shortcut.Arguments = ($resolved -join ' ')
$shortcut.WorkingDirectory = $Target
$shortcut.Description = 'DJLED — audio-reactive LED wall'
$shortcut.Save()
Write-Note 'Start Menu shortcut written'

if ($Start) {
    Write-Step 'Starting it'
    if ($resolved.Count -gt 0) {
        Start-Process -FilePath $windowed -ArgumentList $resolved -WorkingDirectory $Target
    } else {
        Start-Process -FilePath $windowed -WorkingDirectory $Target
    }
    # Long enough to have opened its devices and bound its ports, so what the
    # log says below is what this run actually settled on.
    Start-Sleep -Seconds 3

    # The whole point of this block. A windowed process that rejects its
    # arguments or cannot open the port exits immediately and silently — there
    # is no console for it to complain to — so the only two things worth
    # knowing are whether it is still alive and what it says it is driving.
    if (-not (Get-Process -Name 'djledw' -ErrorAction SilentlyContinue)) {
        Write-Host ''
        Write-Host 'It started and exited. The last thing it wrote:' -ForegroundColor Red
        foreach ($line in (Get-LastRun)) { Write-Host "    $line" }
        throw 'the installed copy did not stay running'
    }

    foreach ($line in (Get-LastRun)) {
        if ($line -match '^\s+(source|output|editor)\s') { Write-Note $line.Trim() }
    }
    if ((Get-LastRun) -match 'mock link') {
        Write-Host ''
        Write-Warn 'It is running on a mock link — the editor will look perfectly'
        Write-Warn 'alive and nothing will reach the strip. Reinstall with -Port.'
    }
}

Write-Host ''
Write-Host 'DJLED is installed.' -ForegroundColor Green
Write-Note 'right-click the tray icon: Open editor, Restart, Stop'
Write-Note "log:     $env:LOCALAPPDATA\DJLED\djled.log"
Write-Note "presets: $env:APPDATA\djled\presets.json"
Write-Note 'uninstall: scripts\release.ps1 -Uninstall'
