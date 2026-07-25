#!/usr/bin/env bash
#
# start-serena.sh — Start the Serena MCP server for a coding session (macOS/Linux).
#
# For one-time project setup (create project, index, install hooks), use
# init-serena.sh instead.
#
# Prerequisite: serena must be installed as a uv tool. One-time install:
#
#     uv tool install serena-agent
#
# (upgrade later with: uv tool upgrade serena-agent)
#
# Two transport modes are supported:
#
#   stdio — registers the server command with Claude Code so it
#     spawns and manages the process itself via stdin/stdout.
#
#   http (default) — starts a background process listening on a port and
#     registers the URL with Claude Code. You control the server lifecycle.
#
# Both modes automatically update the Claude Code MCP configuration.
#
# Usage:
#   ./start-serena.sh                                              # Start HTTP server + register (default)
#   ./start-serena.sh --mode stdio                                 # Register stdio with Claude Code
#   ./start-serena.sh --mode http --port 8888 --no-dashboard       # Custom port, no dashboard
#   ./start-serena.sh --debug                                      # Show diagnostic messages

set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────
MODE="http"
PORT=9999
DISABLE_DASHBOARD=false
DEBUG=false
YES=false

WORKING_DIR="$(pwd)"

PID_FILE="/tmp/serena-mcp.pid"
LOG_FILE="/tmp/serena-mcp.log"

# ── Parse arguments ──────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --mode)
            MODE="$2"
            if [[ "$MODE" != "stdio" && "$MODE" != "http" ]]; then
                echo "Error: --mode must be 'stdio' or 'http' (got '$MODE')." >&2
                exit 1
            fi
            shift 2
            ;;
        --port)
            PORT="$2"
            shift 2
            ;;
        --no-dashboard)
            DISABLE_DASHBOARD=true
            shift
            ;;
        --debug)
            DEBUG=true
            shift
            ;;
        --yes|-y)
            YES=true
            shift
            ;;
        --help|-h)
            echo "Usage: $0 [--mode stdio|http] [--port PORT] [--no-dashboard] [--yes] [--debug]"
            echo ""
            echo "Start the Serena MCP server for a coding session."
            echo "Run init-serena.sh for one-time project setup (create, index, install hooks)."
            echo ""
            echo "Options:"
            echo "  --mode MODE      Transport mode: stdio or http (default)"
            echo "                     stdio — registers command; Claude Code manages the process"
            echo "                     http  — starts background server on --port, registers URL"
            echo "  --port PORT      Port for the MCP server (http mode only; default: 9999)"
            echo "  --no-dashboard   Disable the Serena web dashboard"
            echo "  --yes, -y        Skip confirmation prompt and proceed automatically"
            echo "  --debug          Show diagnostic messages (constructed commands, etc.)"
            echo "  --help, -h       Show this help message"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            echo "Run '$0 --help' for usage." >&2
            exit 1
            ;;
    esac
done

# ── Prerequisite: serena must be in PATH (all modes) ─────────────────────────
# Serena is installed as a uv tool (see script header). Once installed,
# 'serena' is available on PATH as a system binary — no wrapper needed.
if ! command -v serena &>/dev/null; then
    echo "" >&2
    echo "Error: serena is not installed or not in PATH." >&2
    echo "       Install uv first:  brew install uv    (macOS/Linux)" >&2
    echo "       Then install serena as a uv tool:" >&2
    echo "         uv tool install serena-agent" >&2
    echo "" >&2
    exit 1
fi

# ── Debug helper ──────────────────────────────────────────────────────────────
debug_log() {
    [[ "$DEBUG" == "true" ]] && echo "[DEBUG] $*" >&2 || true
}

if [[ "$DEBUG" == "true" ]]; then
    debug_log "Debug mode enabled."
fi

# ── Confirmation helper ───────────────────────────────────────────────────────
confirm_proceed() {
    local settings_block="$1"
    echo ""
    echo "$settings_block"
    echo ""
    if [[ "$YES" == "true" ]]; then
        return
    fi
    if [[ ! -t 0 ]]; then
        echo "Note: non-interactive mode — proceeding automatically."
        return
    fi
    while true; do
        read -r -p "Proceed? [Y/n]: " answer
        case "$answer" in
            ""|y|Y)
                return
                ;;
            n|N)
                echo "Aborted."
                exit 0
                ;;
            *)
                echo "Please enter Y or N."
                ;;
        esac
    done
}

# ── Port-in-use helper ────────────────────────────────────────────────────────
# Returns 0 (true) if the port is occupied, 1 (false) if free.
_port_in_use() {
    local port="$1"
    if command -v ss &>/dev/null; then
        ss -tln 2>/dev/null | awk '{print $4}' | grep -qE ":${port}$"
    elif command -v netstat &>/dev/null; then
        # macOS netstat uses dot separators (*.9999); Linux uses colon (0.0.0.0:9999)
        netstat -tln 2>/dev/null | awk '{print $4}' | grep -qE "[:.]${port}$"
    else
        return 1  # Cannot detect — assume free
    fi
}

# ── MCP registration helper ─────────────────────────────────────────────────
# Removes existing serena MCP (if any) then adds with the given arguments.
register_mcp() {
    local transport_label="$1"
    shift
    # "$@" contains the remaining arguments for 'claude mcp add'

    if claude mcp get serena >/dev/null 2>&1; then
        echo "Warning: 'serena' MCP already registered — removing it first."
        claude mcp remove serena >/dev/null 2>&1 || true
    fi

    if claude mcp add "$@"; then
        echo "Serena MCP registered successfully ($transport_label)."
    else
        echo "Failed to register Serena MCP. Check 'claude mcp list' for details." >&2
    fi
}

# ── Common server arguments (shared by both transports) ──────────────────────
COMMON_SERVER_ARGS=(
    "start-mcp-server"
    "--project-from-cwd"
    "--context" "claude-code"
)
if [[ "$DISABLE_DASHBOARD" == "true" ]]; then
    COMMON_SERVER_ARGS+=("--open-web-dashboard" "false")
fi

# ── stdio mode ────────────────────────────────────────────────────────────────
# In stdio mode Claude Code spawns and owns the process; no background process.
if [[ "$MODE" == "stdio" ]]; then
    flat_cmd="serena ${COMMON_SERVER_ARGS[*]}"

    confirm_proceed "Settings:
  Mode:       Register MCP server (stdio)
  Command:    $flat_cmd"

    echo ""
    echo "Registering Serena MCP with Claude Code (stdio) ..."

    register_mcp "stdio" \
        serena --scope user --transport stdio -- serena "${COMMON_SERVER_ARGS[@]}"

    echo ""
    echo "Claude Code will start and manage the Serena process automatically."
    echo "Restart Claude Code to activate."
    echo ""
    exit 0
fi

# ══════════════════════════════════════════════════════════════════════════════
# http mode — background process, port-based
# ══════════════════════════════════════════════════════════════════════════════

# ── Guard: skip if already running ────────────────────────────────────────────
# NOTE: This guard runs BEFORE the port check so that re-running against a live
# server exits silently — the running server owns the port, so the port check
# would otherwise fire a false-positive error.
if [[ -f "$PID_FILE" ]]; then
    EXISTING_PID=$(cat "$PID_FILE")
    if kill -0 "$EXISTING_PID" 2>/dev/null; then
        echo "Serena MCP server already running (PID $EXISTING_PID). Skipping start."
        exit 0
    fi
    rm -f "$PID_FILE"
fi

# ── Prerequisite: port must be free (start mode only) ─────────────────────────
if _port_in_use "$PORT"; then
    echo "" >&2
    echo "Error: Port $PORT is already in use." >&2
    echo "       Use --port to specify a different port, or stop the existing process first." >&2
    echo "" >&2
    exit 1
fi

# ── Confirmation for start mode ───────────────────────────────────────────────
dashboard_status="enabled"
[[ "$DISABLE_DASHBOARD" == "true" ]] && dashboard_status="disabled"

confirm_proceed "Settings:
  Mode:       Start MCP server (http)
  Port:       $PORT
  Dashboard:  $dashboard_status"

# ── Start Serena in the background ───────────────────────────────────────────
echo ""
echo "Serena MCP server starting on port $PORT ..."

cd "$WORKING_DIR"
debug_log "Executing: serena ${COMMON_SERVER_ARGS[*]} --transport streamable-http --port $PORT"
nohup serena \
        "${COMMON_SERVER_ARGS[@]}" \
        --transport streamable-http \
        --port "$PORT" \
    > "$LOG_FILE" 2>&1 &
SERENA_PID=$!
echo "$SERENA_PID" > "$PID_FILE"

# ── Wait for server to become ready ──────────────────────────────────────────
# Polls /health every 1s for up to 30s. Any 3-digit HTTP code (not "000") means
# the server is accepting connections. "000" is what curl returns on
# connection-refused or timeout.
HEALTH_URL="http://localhost:${PORT}/health"
MAX_WAIT=30
elapsed=0
ready=false

echo "Waiting for server to become ready..."

while [[ "$ready" == "false" ]]; do
    while [[ $elapsed -lt $MAX_WAIT ]]; do
        http_code=$(curl -s -o /dev/null -w "%{http_code}" --connect-timeout 2 "$HEALTH_URL" 2>/dev/null || true)
        if [[ "$http_code" =~ ^[1-9][0-9]{2}$ ]]; then
            ready=true
            break
        fi
        sleep 1
        elapsed=$((elapsed + 1))
        if (( elapsed % 5 == 0 )); then
            echo "  ... still waiting ($elapsed / ${MAX_WAIT}s)"
        fi
    done

    if [[ "$ready" == "true" ]]; then break; fi

    # Server not ready yet — show log output and let the user decide.
    echo ""
    echo "Server has not responded after ${elapsed}s."
    if [[ -s "$LOG_FILE" ]]; then
        echo "--- Startup output ---"
        cat "$LOG_FILE"
        echo "----------------------"
    fi

    if [[ "$YES" == "true" || ! -t 0 ]]; then
        echo "Non-interactive mode — proceeding without waiting further."
        break
    fi

    while true; do
        read -r -p "Keep waiting (${MAX_WAIT}s more) or proceed? [W/p]: " answer
        case "$answer" in
            ""|w|W)
                MAX_WAIT=$((MAX_WAIT * 2))
                echo "Waiting up to ${MAX_WAIT}s total ..."
                break
                ;;
            p|P)
                echo "Proceeding — server may still be starting."
                break
                ;;
            *)
                echo "Please enter W or P."
                ;;
        esac
    done
    if [[ "$answer" == "p" || "$answer" == "P" ]]; then break; fi
done

if [[ "$ready" == "true" ]]; then
    echo "Server is ready -> http://localhost:${PORT}/mcp"
fi

# ── Register MCP with Claude Code ────────────────────────────────────────────
echo ""
echo "Registering Serena MCP with Claude Code (http) ..."

register_mcp "http://localhost:$PORT/mcp" \
    serena --scope user --transport http "http://localhost:$PORT/mcp"

# ── Instructions ──────────────────────────────────────────────────────────────
echo ""
echo "Serena MCP server is running in the background (PID $SERENA_PID)."
echo ""

echo "Monitor"
echo "  Stream log output (like tail -f):"
echo "    tail -f $LOG_FILE"
echo ""
echo "  Check if the process is running:"
echo "    kill -0 \$(cat $PID_FILE) 2>/dev/null && echo 'Running' || echo 'Stopped'"
echo ""

if [[ "$DISABLE_DASHBOARD" == "true" ]]; then
    echo "Stop the server"
    echo "    kill \$(cat $PID_FILE) && rm -f $PID_FILE"
else
    echo "Stop the server"
    echo "  Use the Serena web dashboard shutdown button."
    echo ""
    echo "  Do NOT use 'kill -9' — it kills the process without a graceful shutdown"
    echo "  and may leave stale lock files or corrupt state."
    echo ""
    echo "  After the server shuts down cleanly, remove the PID file with:"
    echo "    rm -f $PID_FILE"
fi

debug_log "Server output is logged to $LOG_FILE"

echo ""
