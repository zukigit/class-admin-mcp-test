#!/usr/bin/env bash
# Create a new agent folder from the template:
#
#   <kit>/scripts/new-agent.sh agent1      -> <kit>/agents/agent1/
#
# <kit> is this kit folder, wherever it lives.
#
# Edit agents/agent1/ (agent.toml, workspace/*.md), then install it with
#   <kit>/scripts/install-agent.sh --data <zeroclaw-data> agent1
set -euo pipefail

KIT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
NAME="${1:-}"

if [[ ! "$NAME" =~ ^[a-z0-9_-]+$ ]]; then
  echo "usage: $0 <agent-name>   (name: lowercase letters, digits, _ or -)" >&2
  exit 1
fi

DEST="$KIT/agents/$NAME"
if [[ -e "$DEST" ]]; then
  echo "already exists: $DEST" >&2
  exit 1
fi

mkdir -p "$DEST/workspace"
cp "$KIT/templates/agent/agent.toml" "$DEST/agent.toml"
for f in "$KIT"/templates/agent/workspace/*.md; do
  sed "s|{{AGENT_NAME}}|$NAME|g" "$f" > "$DEST/workspace/$(basename "$f")"
done

echo "created $DEST"
echo "next: edit $DEST/workspace/{IDENTITY,SOUL,USER}.md, then run"
echo "  $KIT/scripts/install-agent.sh --data <zeroclaw-data-dir> $NAME"
