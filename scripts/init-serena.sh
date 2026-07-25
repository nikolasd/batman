#!/usr/bin/env bash
#
# init-serena.sh — One-time project setup for Serena MCP (macOS/Linux).
#
# Handles the one-time steps required before using Serena MCP in a project.
# These steps need to be performed once per repo clone, not every session.
#
# To start the Serena MCP server for a coding session, use start-serena.sh instead.
#
# Prerequisite: serena must be installed as a uv tool. One-time install:
#
#     uv tool install serena-agent
#
# (upgrade later with: uv tool upgrade serena-agent)
#
# Multiple flags may be combined. Operations run in order: create → index → install-hooks.
# If any operation fails the script exits immediately without running subsequent operations.
#
# Usage:
#   ./init-serena.sh --create                                              # Create project (default languages)
#   ./init-serena.sh --create --language python --language typescript      # Create with custom languages
#   ./init-serena.sh --index                                               # Index project
#   ./init-serena.sh --install-hooks                                       # Install lifecycle hooks (requires jq)
#   ./init-serena.sh --create --index --install-hooks                      # Full first-time setup
#   ./init-serena.sh --debug                                               # Show diagnostic messages

set -euo pipefail

# ── Defaults ──────────────────────────────────────────────────────────────────
CREATE=false
INDEX=false
INSTALL_HOOKS=false
DEBUG=false
YES=false

WORKING_DIR="$(pwd)"

# Default languages — overridden if --language flags are supplied
LANGUAGES=("python" "typescript" "csharp" "powershell" "markdown")
USER_LANGUAGES=()

# ── Parse arguments ───────────────────────────────────────────────────────────
if [[ $# -eq 0 ]]; then
    echo "Usage: $0 [--create] [--index] [--install-hooks] [--language LANG]... [--debug]"
    echo "Run '$0 --help' for full usage."
    exit 0
fi

while [[ $# -gt 0 ]]; do
    case "$1" in
        --create)
            CREATE=true
            shift
            ;;
        --index)
            INDEX=true
            shift
            ;;
        --install-hooks)
            INSTALL_HOOKS=true
            shift
            ;;
        --language)
            USER_LANGUAGES+=("$2")
            shift 2
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
            echo "Usage: $0 [--create] [--index] [--install-hooks] [--language LANG]... [--yes] [--debug]"
            echo ""
            echo "One-time project setup for Serena MCP."
            echo "Run start-serena.sh to start the server each session."
            echo ""
            echo "Options:"
            echo "  --create         Create a new Serena project in the current directory"
            echo "  --index          Index the Serena project"
            echo "  --install-hooks  Install Serena lifecycle hooks into .claude/settings.json"
            echo "                   Idempotent — skips hooks already present. Requires jq."
            echo "  --language LANG  Language to include in project (repeatable; used with --create)"
            echo "                   Defaults: python typescript csharp powershell markdown"
            echo "  --yes, -y        Skip confirmation prompt and proceed automatically"
            echo "  --debug          Show diagnostic messages (constructed commands, etc.)"
            echo "  --help, -h       Show this help message"
            echo ""
            echo "Examples:"
            echo "  $0 --create                                         # Create project"
            echo "  $0 --create --language python --language typescript # Custom languages"
            echo "  $0 --index                                          # Index project"
            echo "  $0 --install-hooks                                  # Install hooks"
            echo "  $0 --create --index --install-hooks                 # Full first-time setup"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            echo "Run '$0 --help' for usage." >&2
            exit 1
            ;;
    esac
done

# Apply user-supplied language overrides (if any)
if [[ ${#USER_LANGUAGES[@]} -gt 0 ]]; then
    LANGUAGES=("${USER_LANGUAGES[@]}")
fi

# ── Prerequisite: serena must be in PATH ──────────────────────────────────────
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

[[ "$DEBUG" == "true" ]] && debug_log "Debug mode enabled."

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
            ""|y|Y) return ;;
            n|N) echo "Aborted."; exit 0 ;;
            *) echo "Please enter Y or N." ;;
        esac
    done
}

# ── Validate prerequisites for selected operations ────────────────────────────
SETTINGS_PATH="$WORKING_DIR/.claude/settings.json"

if [[ "$INSTALL_HOOKS" == "true" ]]; then
    if [[ ! -f "$SETTINGS_PATH" ]]; then
        echo "" >&2
        echo "Error: .claude/settings.json not found at $WORKING_DIR" >&2
        echo "       Run this script from the project root directory." >&2
        echo "" >&2
        exit 1
    fi

    if ! command -v jq &>/dev/null; then
        echo "" >&2
        echo "Error: jq is required to install hooks but was not found in PATH." >&2
        echo "       Install jq:" >&2
        echo "         macOS:  brew install jq" >&2
        echo "         Ubuntu: apt-get install jq" >&2
        echo "         Alpine: apk add jq" >&2
        echo "" >&2
        exit 1
    fi
fi

# ── Confirm all planned operations upfront ────────────────────────────────────
op_lines=""
if [[ "$CREATE" == "true" ]]; then
    lang_list="${LANGUAGES[*]}"
    op_lines+="  - Create project (languages: ${lang_list// /, })"$'\n'
fi
if [[ "$INDEX" == "true" ]]; then
    op_lines+="  - Index project"$'\n'
fi
if [[ "$INSTALL_HOOKS" == "true" ]]; then
    op_lines+="  - Install lifecycle hooks -> $SETTINGS_PATH"$'\n'
fi

confirm_proceed "Settings:
  Directory:  $WORKING_DIR

Operations:
${op_lines%$'\n'}"

# ── Create project ────────────────────────────────────────────────────────────
if [[ "$CREATE" == "true" ]]; then
    echo ""
    echo "Creating Serena project in $WORKING_DIR ..."
    lang_args=()
    for lang in "${LANGUAGES[@]}"; do
        lang_args+=("--language" "$lang")
    done
    debug_log "Executing: serena project create ${lang_args[*]}"
    serena project create "${lang_args[@]}"
fi

# ── Index project ─────────────────────────────────────────────────────────────
if [[ "$INDEX" == "true" ]]; then
    echo ""
    echo "Indexing Serena project in $WORKING_DIR ..."
    debug_log "Executing: serena project index"
    serena project index
fi

# ── Install hooks ─────────────────────────────────────────────────────────────
if [[ "$INSTALL_HOOKS" == "true" ]]; then
    # The four Serena lifecycle hooks (parallel arrays)
    HOOK_EVENTS=("PreToolUse" "PreToolUse" "SessionStart" "SessionEnd")
    HOOK_MATCHERS=("" "mcp__serena__*" "" "")
    HOOK_COMMANDS=(
        "serena-hooks remind --client=claude-code"
        "serena-hooks auto-approve --client=claude-code"
        "serena-hooks activate --client=claude-code"
        "serena-hooks cleanup --client=claude-code"
    )

    echo ""
    echo "Installing Serena hooks into .claude/settings.json ..."

    current_json=$(cat "$SETTINGS_PATH")
    added=0
    skipped=0

    for i in "${!HOOK_EVENTS[@]}"; do
        event="${HOOK_EVENTS[$i]}"
        matcher="${HOOK_MATCHERS[$i]}"
        command="${HOOK_COMMANDS[$i]}"

        # Check if this command is already registered under this event
        exists=$(printf '%s' "$current_json" | jq \
            --arg event "$event" \
            --arg cmd "$command" \
            '[(.hooks[$event] // [])[] | (.hooks // [])[] | .command] | any(. == $cmd)')

        if [[ "$exists" == "true" ]]; then
            echo "  [skip] $event ($matcher): $command"
            skipped=$((skipped + 1))
        else
            current_json=$(printf '%s' "$current_json" | jq \
                --arg event "$event" \
                --arg matcher "$matcher" \
                --arg command "$command" \
                '.hooks[$event] = ((.hooks[$event] // []) + [{"matcher": $matcher, "hooks": [{"type": "command", "command": $command}]}])')
            debug_log "Added hook: $event ($matcher): $command"
            echo "  [add]  $event ($matcher): $command"
            added=$((added + 1))
        fi
    done

    printf '%s\n' "$current_json" > "$SETTINGS_PATH"

    echo ""
    echo "Done: $added added, $skipped already present."
fi
