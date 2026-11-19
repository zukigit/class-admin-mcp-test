#!/usr/bin/env bash
# Install a saved agent folder into a ZeroClaw data dir:
#
#   <kit>/scripts/install-agent.sh --data <zeroclaw-data> agent1
#   <kit>/scripts/install-agent.sh --data <zeroclaw-data> path/to/agent1
#   <kit>/scripts/install-agent.sh --data <zeroclaw-data> agent1 --force
#
# <kit> is this kit folder, wherever it lives. A bare name resolves to
# <kit>/agents/<name>; anything that is an existing folder is used as-is.
#
# --data (-d) is required: the ZeroClaw home dir that is mounted into the
# container (the one that holds, or will hold, .zeroclaw/config.toml).
#
# The agent is named after its folder. This
#   - adds its settings from agent.toml to <data>/.zeroclaw/config.toml
#     (creating that file from templates/config.base.toml if it is missing)
#   - copies workspace/*.md to <data>/.zeroclaw/agents/<name>/workspace/
#
# --force replaces the agent's config sections and overwrites its workspace
# files (including MEMORY.md the agent may have edited); its memory database
# and sessions are kept.
#
# Afterwards restart ZeroClaw, e.g. docker compose restart zeroclaw
set -euo pipefail

KIT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

usage() {
  echo "usage: $0 --data <zeroclaw-data-dir> <agent-name|agent-folder> [--force]" >&2
  exit 1
}

SRC=""; DATA_DIR=""; FORCE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -d|--data) [[ $# -ge 2 ]] || usage; DATA_DIR="$2"; shift 2 ;;
    --data=*)  DATA_DIR="${1#--data=}"; shift ;;
    --force)   FORCE=1; shift ;;
    -h|--help) usage ;;
    -*)        echo "unknown option: $1" >&2; usage ;;
    *)         [[ -z "$SRC" ]] || usage; SRC="$1"; shift ;;
  esac
done
[[ -n "$SRC" && -n "$DATA_DIR" ]] || usage
mkdir -p "$DATA_DIR"
DATA_DIR="$(cd "$DATA_DIR" && pwd)"

[[ -d "$SRC" ]] || SRC="$KIT/agents/$SRC"
if [[ ! -f "$SRC/agent.toml" ]]; then
  echo "not an agent folder (no agent.toml): $SRC" >&2
  exit 1
fi
SRC="$(cd "$SRC" && pwd)"
NAME="$(basename "$SRC")"
if [[ ! "$NAME" =~ ^[a-z0-9_-]+$ ]]; then
  echo "folder name must be lowercase letters, digits, _ or -: $NAME" >&2
  exit 1
fi

ZC_DIR="$DATA_DIR/.zeroclaw"
CONFIG="$ZC_DIR/config.toml"
WORKSPACE="$ZC_DIR/agents/$NAME/workspace"

mkdir -p "$ZC_DIR"
if [[ ! -f "$CONFIG" ]]; then
  cp "$KIT/templates/config.base.toml" "$CONFIG"
  echo "created $CONFIG from templates/config.base.toml"
fi

if grep -qE "^\[agents\.$NAME(\.|\])" "$CONFIG"; then
  if [[ $FORCE -eq 0 ]]; then
    echo "agent '$NAME' is already installed; use --force to reinstall" >&2
    exit 1
  fi
  # Drop every [agents.<name>] / [agents.<name>.*] table.
  cp "$CONFIG" "$CONFIG.bak"
  awk -v n="$NAME" '
    /^\[/ { skip = ($0 ~ "^\\[agents\\." n "(\\.|\\])") }
    !skip
  ' "$CONFIG.bak" > "$CONFIG"
  echo "removed old '$NAME' settings (backup: $CONFIG.bak)"
fi

# Rename [agent...] tables to [agents.<name>...], drop comments, append.
{
  echo
  echo "# --- agent: $NAME ---"
  sed -E -e '/^[[:space:]]*#/d' -e "s/^\[agent(\.|\])/[agents.$NAME\1/" "$SRC/agent.toml"
} >> "$CONFIG"
echo "added agent '$NAME' to $CONFIG"

PROVIDER="$(sed -nE 's/^model_provider *= *"([^"]+)".*/\1/p' "$SRC/agent.toml" | head -1)"
if [[ -n "$PROVIDER" ]] && ! grep -qF "[providers.models.$PROVIDER]" "$CONFIG"; then
  echo "warning: provider '$PROVIDER' is not defined in $CONFIG" >&2
fi

mkdir -p "$WORKSPACE"
if [[ -d "$SRC/workspace" ]]; then
  for f in "$SRC"/workspace/*; do
    dest="$WORKSPACE/$(basename "$f")"
    if [[ -e "$dest" && $FORCE -eq 0 ]]; then
      echo "  kept existing $(basename "$f")"
    else
      cp -r "$f" "$dest"
    fi
  done
fi
echo "workspace: $WORKSPACE"
echo "next: docker compose restart zeroclaw"
