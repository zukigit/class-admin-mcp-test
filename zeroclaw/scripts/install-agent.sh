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
#   - installs the risk_profile / runtime_profile it names from the kit,
#     if the config doesn't have them yet (existing profiles are left alone)
#   - copies workspace/*.md to <data>/.zeroclaw/agents/<name>/workspace/
#
# --force replaces the agent's config sections and overwrites its workspace
# files (including MEMORY.md the agent may have edited); its memory database
# and sessions are kept. Profiles are shared between agents, so --force does
# not touch them; use install-profile.sh --force for that.
#
# Afterwards restart ZeroClaw, e.g. docker compose restart zeroclaw
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

usage() { zc_die "usage: $0 --data <zeroclaw-data-dir> <agent-name|agent-folder> [--force]"; }

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

[[ -d "$SRC" ]] || SRC="$KIT/agents/$SRC"
[[ -f "$SRC/agent.toml" ]] || zc_die "not an agent folder (no agent.toml): $SRC"
SRC="$(cd "$SRC" && pwd)"
NAME="$(basename "$SRC")"
zc_check_name "$NAME"

zc_ensure_config "$DATA_DIR"
WORKSPACE="$(dirname "$CONFIG")/agents/$NAME/workspace"

if zc_has "$CONFIG" agents "$NAME"; then
  [[ $FORCE -eq 1 ]] || zc_die "agent '$NAME' is already installed; use --force to reinstall"
  zc_remove "$CONFIG" agents "$NAME"
fi
zc_append "$CONFIG" "$SRC/agent.toml" agent agents "$NAME"

# Profiles the agent refers to: install from the kit if missing.
for kind in risk runtime; do
  zc_kind "$kind"
  profile="$(zc_value "$SRC/agent.toml" "$KIND_HEADER")"
  [[ -n "$profile" ]] || continue
  zc_has "$CONFIG" "$KIND_TABLE" "$profile" && continue
  if [[ -f "$KIT/$KIND_TABLE/$profile.toml" ]]; then
    zc_install_profile "$CONFIG" "$kind" "$profile" 0
  else
    echo "warning: $KIND_TABLE.$profile is not in the config or the kit" >&2
  fi
done

PROVIDER="$(zc_value "$SRC/agent.toml" model_provider)"
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
