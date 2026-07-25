<#
.SYNOPSIS
    Starts the Serena MCP server for a coding session.

.DESCRIPTION
    Launches 'serena start-mcp-server' with Claude Code modes (interactive,
    editing, planning, onboarding, query-projects).

    For one-time project setup (create project, index, install hooks), use
    Initialize-Serena.ps1 instead.

    Prerequisite: serena must be installed as a uv tool. One-time install:

        uv tool install serena-agent

    (upgrade later with: uv tool upgrade serena-agent)

    Two transport modes are supported:

      stdio — registers the server command with Claude Code so it
        spawns and manages the process itself via stdin/stdout.

      http (default) — starts a background job listening on a port and registers
        the URL with Claude Code. You control the server lifecycle.

    Both modes automatically update the Claude Code MCP configuration.

    Pass -Debug to show diagnostic messages (Write-Debug) throughout execution,
    including fully constructed command arguments. Useful for diagnosing startup
    failures or argument-passing issues. Debug output also propagates into the
    background job.

.PARAMETER Mode
    Transport mode for the MCP server. Valid values: stdio, http.
    Defaults to http.

      stdio  — Registers command; Claude Code manages the process.
      http   — Starts background server on -Port, registers URL.

.PARAMETER Port
    Port number for the MCP server (http mode only). Defaults to 9999.

.PARAMETER DisableDashboard
    When present, passes --open-web-dashboard false to Serena.

.PARAMETER Yes
    Skips all confirmation prompts and proceeds automatically.
    Also skips the "keep waiting" prompt if the server takes longer than expected to start.

.PARAMETER Help
    Displays this help message.

.EXAMPLE
    .\Start-Serena.ps1                             # Start HTTP server + register (default)
    .\Start-Serena.ps1 -Mode stdio                 # Register stdio with Claude Code
    .\Start-Serena.ps1 -Port 8888 -DisableDashboard # Custom port, no dashboard
    .\Start-Serena.ps1 -Yes                        # Skip confirmation prompt
    .\Start-Serena.ps1 -Debug                      # Full execution tracing
#>
#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet("stdio", "http")]
    [string]$Mode = "http",
    [int]$Port = 9999,
    [switch]$DisableDashboard,
    [switch]$Yes,
    [switch]$Help
)

# ── Help ──────────────────────────────────────────────────────────────────────
if ($Help) {
    Get-Help $PSCommandPath -Full
    exit 0
}

# ── Debug flag ────────────────────────────────────────────────────────────────
# -Debug is a built-in common parameter (from CmdletBinding) that sets
# $DebugPreference = 'Continue', making Write-Debug messages visible.
# We capture it once so helper functions and the background job can check it.
$IsDebug = $PSBoundParameters.ContainsKey('Debug')
if ($IsDebug) {
    Write-Debug "Debug mode enabled."
}

# Capture invocation directory immediately — before anything else can change $PWD.
$WorkingDir = (Get-Location).Path
$JobName    = "SerenaMCP"

# ── Prerequisite: serena must be in PATH (all modes) ─────────────────────────
# Serena is installed as a uv tool (see script docstring). Once installed,
# 'serena' is available on PATH as a system binary — no wrapper needed.
if (-not (Get-Command serena -ErrorAction SilentlyContinue)) {
    Write-Host ""
    Write-Host "Error: serena is not installed or not in PATH." -ForegroundColor Red
    Write-Host "       Install uv first:  scoop install uv   (Windows)" -ForegroundColor Red
    Write-Host "                          brew install uv    (macOS/Linux)" -ForegroundColor Red
    Write-Host "       Then install serena as a uv tool:" -ForegroundColor Red
    Write-Host "         uv tool install serena-agent" -ForegroundColor Red
    Write-Host ""
    exit 1
}

# ── Debug helper ──────────────────────────────────────────────────────────────
function Write-DebugCommand ([string]$Executable, [string[]]$Arguments) {
    if (-not $IsDebug) { return }
    $flat = ($Arguments | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }) -join ' '
    Write-Debug "Executing: $Executable $flat"
}

# ── Confirmation helper ───────────────────────────────────────────────────────
function Confirm-Proceed ([string]$SettingsBlock, [bool]$AutoYes = $false) {
    Write-Host ""
    Write-Host $SettingsBlock
    Write-Host ""
    if ($AutoYes) { return }
    while ($true) {
        $answer = (Read-Host "Proceed? [Y/n]").Trim()
        if ($answer -eq '' -or $answer -eq 'y' -or $answer -eq 'Y') { return }
        if ($answer -eq 'n' -or $answer -eq 'N') {
            Write-Host "Aborted." -ForegroundColor Yellow
            exit 0
        }
        Write-Host "Please enter Y or N." -ForegroundColor Yellow
    }
}

# ── Common server arguments (shared by both transports) ──────────────────────
$commonServerArgs = @(
    "start-mcp-server",
    "--project-from-cwd",
    "--context", "claude-code"
)
if ($DisableDashboard) { $commonServerArgs += "--open-web-dashboard", "false" }

# ── stdio mode ────────────────────────────────────────────────────────────────
# In stdio mode Claude Code spawns and owns the process; no background job.
if ($Mode -eq "stdio") {
    $flatCmd = "serena " + (($commonServerArgs | ForEach-Object {
        if ($_ -match '\s') { "`"$_`"" } else { $_ }
    }) -join ' ')

    Confirm-Proceed @"
Settings:
  Mode:       Register MCP server (stdio)
  Command:    $flatCmd
"@ $Yes.IsPresent

    Write-Host ""
    Write-Host "Registering Serena MCP with Claude Code (stdio) ..." -ForegroundColor Cyan

    & claude mcp get serena 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Write-Host "Warning: 'serena' MCP already registered — removing it first." `
            -ForegroundColor DarkYellow
        & claude mcp remove serena 2>&1 | Out-Null
    }

    & claude mcp add serena --scope user --transport stdio -- serena @commonServerArgs

    if ($LASTEXITCODE -eq 0) {
        Write-Host "Serena MCP registered successfully (stdio)." -ForegroundColor Green
        Write-Host ""
        Write-Host "Claude Code will start and manage the Serena process automatically." `
            -ForegroundColor Cyan
        Write-Host "Restart Claude Code to activate." -ForegroundColor Cyan
    } else {
        Write-Host "Failed to register Serena MCP. Check 'claude mcp list' for details." `
            -ForegroundColor Red
    }
    Write-Host ""
    exit 0
}

# ══════════════════════════════════════════════════════════════════════════════
# http mode — background job, port-based
# ══════════════════════════════════════════════════════════════════════════════

# ── Guard: skip if already running ────────────────────────────────────────────
# NOTE: This guard runs BEFORE the port check so that re-running against a live
# server exits silently — the running server owns the port, so the port check
# would otherwise fire a false-positive error.
$existingJob = Get-Job -Name $JobName -ErrorAction SilentlyContinue
if ($existingJob -and $existingJob.State -eq 'Running') {
    Write-Host "Serena MCP server already running (Job: $JobName). Skipping start." -ForegroundColor Green
    exit 0
}
# Clean up any stale/stopped job before starting fresh
if ($existingJob) {
    Stop-Job   -Name $JobName -ErrorAction SilentlyContinue
    Remove-Job -Name $JobName -Force
}

# ── Prerequisite: port must be free (start mode only) ─────────────────────────
$listeners = [System.Net.NetworkInformation.IPGlobalProperties]::GetIPGlobalProperties().GetActiveTcpListeners()
if ($listeners | Where-Object { $_.Port -eq $Port }) {
    Write-Host ""
    Write-Host "Error: Port $Port is already in use." -ForegroundColor Red
    Write-Host "       Use -Port to specify a different port, or stop the existing process first." -ForegroundColor Red
    Write-Host ""
    exit 1
}

# ── Confirmation for start mode ───────────────────────────────────────────────
$dashboardStatus = if ($DisableDashboard) { "disabled" } else { "enabled" }
Confirm-Proceed @"
Settings:
  Mode:       Start MCP server (http)
  Port:       $Port
  Dashboard:  $dashboardStatus
"@ $Yes.IsPresent

# ── Start background job ──────────────────────────────────────────────────────
# Scalars are passed so the ScriptBlock rebuilds its own arg array, avoiding
# PowerShell's array-unwrapping behaviour in -ArgumentList.
$job = Start-Job -Name $JobName -ScriptBlock {
    param(
        [string]$dir,
        [int]$port,
        [bool]$disableDashboard,
        [bool]$debugMode
    )

    if ($debugMode) {
        $DebugPreference = 'Continue'
    }

    Set-Location $dir

    $serverArgs = @(
        "start-mcp-server",
        "--project-from-cwd",
        "--context", "claude-code",
        "--transport", "streamable-http",
        "--port", "$port"
    )

    if ($disableDashboard) { $serverArgs += "--open-web-dashboard", "false" }

    if ($debugMode) {
        $flat = ($serverArgs | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }) -join ' '
        Write-Debug "Background job executing: serena $flat"
    }

    # Invoke serena — merge stdout + stderr so Receive-Job captures both streams.
    & serena @serverArgs 2>&1

} -ArgumentList $WorkingDir, $Port, $DisableDashboard.IsPresent, $IsDebug

# ── Wait for server to become ready ──────────────────────────────────────────
# Serena has no /health endpoint — use a TCP connection check instead.
$maxWait = 30
$elapsed = 0

Write-Host ""
Write-Host "Serena MCP server starting on port $Port ..." -ForegroundColor Cyan
Write-Host "Waiting for server to become ready..." -ForegroundColor Cyan

$ready = $false
while (-not $ready) {
    while ($elapsed -lt $maxWait) {
        try {
            $tcp = New-Object System.Net.Sockets.TcpClient
            $tcp.Connect("localhost", $Port)
            $tcp.Close()
            $ready = $true
            break
        } catch {
            # Connection refused — server not listening yet
        }
        Start-Sleep -Seconds 1
        $elapsed++
        if ($elapsed % 5 -eq 0) {
            Write-Host "  ... still waiting ($elapsed / ${maxWait}s)" -ForegroundColor DarkGray
        }
    }

    if ($ready) { break }

    # Server not ready yet — show job output and let the user decide.
    Write-Host ""
    Write-Host "Server has not responded after ${elapsed}s." -ForegroundColor Yellow
    $startupOutput = Receive-Job -Name $JobName -Keep
    if ($startupOutput) {
        Write-Host "--- Startup output ---" -ForegroundColor DarkGray
        $startupOutput | ForEach-Object { Write-Host $_ }
        Write-Host "----------------------" -ForegroundColor DarkGray
    }

    if ($Yes) {
        Write-Host "Proceeding without waiting further (-Yes)." -ForegroundColor DarkGray
        break
    }
    $answer = ''
    $nextWait = $maxWait * 2
    while ($true) {
        $answer = (Read-Host "Keep waiting ($maxWait s more) or proceed? [W/p]").Trim()
        if ($answer -eq '' -or $answer -eq 'w' -or $answer -eq 'W') {
            $maxWait = $nextWait
            Write-Host "Waiting up to ${maxWait}s total ..." -ForegroundColor Cyan
            break
        }
        if ($answer -eq 'p' -or $answer -eq 'P') {
            Write-Host "Proceeding — server may still be starting." -ForegroundColor Yellow
            break
        }
        Write-Host "Please enter W or P." -ForegroundColor Yellow
    }
    if ($answer -eq 'p' -or $answer -eq 'P') { break }
}

if ($ready) {
    Write-Host "Server is ready -> http://localhost:$Port/mcp" -ForegroundColor Green
}

# ── Register MCP with Claude Code ─────────────────────────────────────────────
Write-Host ""
Write-Host "Registering Serena MCP with Claude Code (http) ..." -ForegroundColor Cyan

& claude mcp get serena 2>&1 | Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Host "Warning: 'serena' MCP already registered — removing it first." `
        -ForegroundColor DarkYellow
    & claude mcp remove serena 2>&1 | Out-Null
}

& claude mcp add serena --scope user --transport http "http://localhost:$Port/mcp"

if ($LASTEXITCODE -eq 0) {
    Write-Host "Serena MCP registered successfully (http://localhost:$Port/mcp)." `
        -ForegroundColor Green
} else {
    Write-Host "Failed to register Serena MCP. Check 'claude mcp list' for details." `
        -ForegroundColor Red
}

# ── Instructions ───────────────────────────────────────────────────────────────
Write-Host ""
Write-Host "Job '$JobName' is running in the background." -ForegroundColor Green
Write-Host ""

Write-Host "Monitor / foreground" -ForegroundColor Yellow
Write-Host "  Stream output (foreground-like, drains buffer each tick):"
Write-Host "    while (`$true) { Receive-Job -Name $JobName; Start-Sleep -Milliseconds 500 }"
Write-Host ""
Write-Host "  Read buffered output without draining:"
Write-Host "    Receive-Job -Name $JobName -Keep"
Write-Host ""
Write-Host "  Check job state:"
Write-Host "    Get-Job -Name $JobName"
Write-Host ""

if ($DisableDashboard) {
    Write-Host "Stop the server" -ForegroundColor Yellow
    Write-Host "    Stop-Job -Name $JobName; Remove-Job -Name $JobName"
} else {
    Write-Host "Stop the server" -ForegroundColor Yellow
    Write-Host "  Use the Serena web dashboard shutdown button." -ForegroundColor Cyan
    Write-Host ""
    Write-Host "  Do NOT use Stop-Job — it kills the process without a graceful shutdown" `
        -ForegroundColor Red
    Write-Host "  and may leave stale lock files or corrupt state."
    Write-Host ""
    Write-Host "  After the server shuts down cleanly, remove the finished job with:"
    Write-Host "    Remove-Job -Name $JobName"
}

if ($IsDebug) {
    Write-Debug "Debug output is also active inside the background job."
    Write-Debug "Use 'Receive-Job -Name $JobName -Keep' to see debug output."
}

Write-Host ""
