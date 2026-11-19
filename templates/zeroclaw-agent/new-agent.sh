#!/usr/bin/env bash
# Create a new ZeroClaw agent from this template.
#
#   templates/zeroclaw-agent/new-agent.sh <agent-name> [data-dir]
#
# data-dir defaults to ./zeroclaw-data (the folder docker-compose mounts).
# Override the model provider with env vars:
#   PROVIDER=opencode PROVIDER_PROFILE=deepseek_flash MODEL=deepseek-v4.1-flash
#
# - No config.toml yet: writes a fresh one (shared settings + this agent).
# - config.toml exists: appends only this agent's sections.
# Either way it creates .zeroclaw/agents/<name>/workspace/ from the template.
# Restart the container afterwards: docker compose restart zeroclaw
set -euo pipefail

TEMPLATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

AGENT_NAME="${1:-}"
DATA_DIR="${2:-./zeroclaw-data}"
PROVIDER="${PROVIDER:-opencode}"
PROVIDER_PROFILE="${PROVIDER_PROFILE:-deepseek_flash}"
MODEL="${MODEL:-deepseek-v4.1-flash}"

if [[ -z "$AGENT_NAME" ]]; then
  echo "usage: $0 <agent-name> [data-dir]" >&2
  exit 1
fi
if [[ ! "$AGENT_NAME" =~ ^[a-z0-9_-]+$ ]]; then
  echo "agent name must match [a-z0-9_-]+" >&2
  exit 1
fi

ZC_DIR="$DATA_DIR/.zeroclaw"
CONFIG="$ZC_DIR/config.toml"
WORKSPACE="$ZC_DIR/agents/$AGENT_NAME/workspace"

render() {
  sed -e "s|{{AGENT_NAME}}|$AGENT_NAME|g" \
      -e "s|{{PROVIDER}}|$PROVIDER|g" \
      -e "s|{{PROVIDER_PROFILE}}|$PROVIDER_PROFILE|g" \
      -e "s|{{MODEL}}|$MODEL|g" "$1"
}

if [[ -e "$WORKSPACE" ]]; then
  echo "workspace already exists: $WORKSPACE" >&2
  exit 1
fi

mkdir -p "$ZC_DIR"
if [[ ! -f "$CONFIG" ]]; then
  { render "$TEMPLATE_DIR/config.base.toml"; render "$TEMPLATE_DIR/agent.toml"; } > "$CONFIG"
  echo "created $CONFIG"
elif grep -q "^\[agents\.$AGENT_NAME\]" "$CONFIG"; then
  echo "agent '$AGENT_NAME' already in $CONFIG, leaving config untouched"
else
  render "$TEMPLATE_DIR/agent.toml" >> "$CONFIG"
  echo "appended agent '$AGENT_NAME' to $CONFIG"
  echo "  note: it uses provider '$PROVIDER.$PROVIDER_PROFILE'; make sure that exists in the config"
fi

mkdir -p "$WORKSPACE"
for f in "$TEMPLATE_DIR"/workspace/*.md; do
  render "$f" > "$WORKSPACE/$(basename "$f")"
done
echo "created $WORKSPACE"
echo "next: edit IDENTITY.md / SOUL.md / USER.md, set the API key, restart zeroclaw"
