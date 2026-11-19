#!/usr/bin/env bash
# Create a new kit item from its template. The folder name is the item's name.
#
#   ./new.sh agent   agent1      -> agents/agent1/            (agent.toml + workspace/*.md)
#   ./new.sh risk    strict      -> risk_profiles/strict/     (risk_profile.toml)
#   ./new.sh runtime fast        -> runtime_profiles/fast/    (runtime_profile.toml)
#
# Edit the new folder, then install it with ./install.sh --data <dir> <folder>.
# Works from any directory; paths are relative to this kit folder.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib/zeroclaw.sh"

[[ $# -eq 2 ]] || zc_die "usage: $0 <agent|risk|runtime> <name>"
zc_type "$1"
NAME="$2"
zc_check_name "$NAME"

DEST="$T_DIR/$NAME"
[[ ! -e "$DEST" ]] || zc_die "already exists: $DEST"

mkdir -p "$T_DIR"
cp -r "$KIT/templates/$T_HEADER" "$DEST"
# Fill in the name in the template's text files.
find "$DEST" -type f -name '*.md' -exec sed -i "s|{{NAME}}|$NAME|g" {} +

echo "created $DEST"
echo "next: edit it, then run"
echo "  $KIT/install.sh --data <zeroclaw-data-dir> $DEST"
