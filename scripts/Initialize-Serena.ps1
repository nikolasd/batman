<#
.SYNOPSIS
    One-time project setup for Serena MCP: create project, index, or install hooks.

.DESCRIPTION
    Handles the one-time setup steps required before using Serena MCP in a project.
    These steps need to be performed once per repo clone, not every session.

    To start the Serena MCP server for a coding session, use Start-Serena.ps1 instead.

    Prerequisite: serena must be installed as a uv tool. One-time install:

        uv tool install serena-agent

    (upgrade later with: uv tool upgrade serena-agent)

    Multiple flags may be combined. Operations run in order: Create → Index → InstallHooks.
    If any operation fails the script exits immediately without running subsequent operations.

    Pass -Debug to show diagnostic messages (Write-Debug) throughout execution.

.PARAMETER Create
    Creates a new Serena project in the current directory.

.PARAMETER Index
    Indexes the Serena project in the current directory.

.PARAMETER InstallHooks
    Installs the four Serena lifecycle hooks into .claude/settings.json.
    Idempotent — skips any hook that is already present. Requires no external tools.

.PARAMETER Language
    Languages to include when creating a Serena project. Only used with -Create.
    Defaults to: python, typescript, csharp, powershell, markdown.
    Example: -Language python,typescript,markdown

.PARAMETER Yes
    Skips the confirmation prompt and proceeds automatically.

.PARAMETER Help
    Displays this help message.

.EXAMPLE
    .\Initialize-Serena.ps1 -Create                              # Create project (default languages)
    .\Initialize-Serena.ps1 -Create -Language python,typescript  # Create with custom languages
    .\Initialize-Serena.ps1 -Index                               # Index project
    .\Initialize-Serena.ps1 -InstallHooks                        # Install Serena lifecycle hooks
    .\Initialize-Serena.ps1 -Create -Index -InstallHooks         # Full first-time setup
    .\Initialize-Serena.ps1 -Yes -Create -Index -InstallHooks    # Full setup, no prompts
    .\Initialize-Serena.ps1 -Debug                               # Full execution tracing
#>
#Requires -Version 5.1
[CmdletBinding()]
param(
    [switch]$Create,
    [switch]$Index,
    [switch]$InstallHooks,
    [string[]]$Language = @("python", "typescript", "csharp", "powershell", "markdown"),
    [switch]$Yes,
    [switch]$Help
)

# ── Help ──────────────────────────────────────────────────────────────────────
if ($Help -or (-not ($Create -or $Index -or $InstallHooks))) {
    Get-Help $PSCommandPath -Full
    exit 0
}

# ── Debug flag ────────────────────────────────────────────────────────────────
# -Debug is a built-in common parameter (from CmdletBinding) that sets
# $DebugPreference = 'Continue', making Write-Debug messages visible.
$IsDebug = $PSBoundParameters.ContainsKey('Debug')
if ($IsDebug) {
    Write-Debug "Debug mode enabled."
}

# Capture invocation directory immediately — before anything else can change $PWD.
$WorkingDir = (Get-Location).Path

# ── Prerequisite: serena must be in PATH ──────────────────────────────────────
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

# ── Validate prerequisites for selected operations ────────────────────────────
$settingsPath = $null
if ($InstallHooks) {
    $settingsPath = Join-Path $WorkingDir ".claude\settings.json"
    if (-not (Test-Path $settingsPath)) {
        Write-Host ""
        Write-Host "Error: .claude/settings.json not found at $WorkingDir" -ForegroundColor Red
        Write-Host "       Run this script from the project root directory." -ForegroundColor Red
        Write-Host ""
        exit 1
    }
}

# ── Confirm all planned operations upfront ────────────────────────────────────
$opLines = @()
if ($Create)       { $opLines += "  - Create project (languages: $($Language -join ', '))" }
if ($Index)        { $opLines += "  - Index project" }
if ($InstallHooks) { $opLines += "  - Install lifecycle hooks -> $settingsPath" }

Confirm-Proceed @"
Settings:
  Directory:  $WorkingDir

Operations:
$($opLines -join "`n")
"@ $Yes.IsPresent

# ── Create project ────────────────────────────────────────────────────────────
if ($Create) {
    Write-Host ""
    Write-Host "Creating Serena project in $WorkingDir ..." -ForegroundColor Cyan
    $langArgs = @()
    foreach ($lang in $Language) { $langArgs += @("--language", $lang) }
    Write-DebugCommand "serena" (@("project", "create") + $langArgs)
    & serena project create @langArgs
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

# ── Index project ─────────────────────────────────────────────────────────────
if ($Index) {
    Write-Host ""
    Write-Host "Indexing Serena project in $WorkingDir ..." -ForegroundColor Cyan
    Write-DebugCommand "serena" @("project", "index")
    & serena project index
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

# ── Install hooks ─────────────────────────────────────────────────────────────
if ($InstallHooks) {
    # The four Serena lifecycle hooks
    $hookDefs = @(
        [PSCustomObject]@{ Event = "PreToolUse";   Matcher = "";               Command = "serena-hooks remind --client=claude-code" }
        [PSCustomObject]@{ Event = "PreToolUse";   Matcher = "mcp__serena__*"; Command = "serena-hooks auto-approve --client=claude-code" }
        [PSCustomObject]@{ Event = "SessionStart"; Matcher = "";               Command = "serena-hooks activate --client=claude-code" }
        [PSCustomObject]@{ Event = "SessionEnd";   Matcher = "";               Command = "serena-hooks cleanup --client=claude-code" }
    )

    Write-Host ""
    Write-Host "Installing Serena hooks into .claude/settings.json ..." -ForegroundColor Cyan

    $rawJson  = Get-Content $settingsPath -Raw -Encoding UTF8
    $settings = $rawJson | ConvertFrom-Json

    if (-not $settings.hooks) {
        $settings | Add-Member -NotePropertyName "hooks" -NotePropertyValue ([PSCustomObject]@{}) -Force
    }

    $added   = 0
    $skipped = 0

    foreach ($def in $hookDefs) {
        $event   = $def.Event
        $matcher = $def.Matcher
        $command = $def.Command

        if ($null -eq $settings.hooks.$event) {
            $settings.hooks | Add-Member -NotePropertyName $event -NotePropertyValue @() -Force
        }

        $exists = $false
        foreach ($entry in $settings.hooks.$event) {
            if ($null -ne $entry.hooks) {
                foreach ($hook in $entry.hooks) {
                    if ($hook.command -eq $command) { $exists = $true; break }
                }
            }
            if ($exists) { break }
        }

        if ($exists) {
            Write-Host "  [skip] $event ($matcher): $command" -ForegroundColor DarkGray
            $skipped++
        } else {
            $newEntry = [PSCustomObject]@{
                matcher = $matcher
                hooks   = @([PSCustomObject]@{ type = "command"; command = $command })
            }
            $settings.hooks.$event += $newEntry
            Write-Host "  [add]  $event ($matcher): $command" -ForegroundColor Green
            $added++
        }
    }

    $newJson = $settings | ConvertTo-Json -Depth 10
    [System.IO.File]::WriteAllText($settingsPath, $newJson + "`n", [System.Text.Encoding]::UTF8)

    Write-Host ""
    Write-Host "Done: $added added, $skipped already present." -ForegroundColor Cyan
}
