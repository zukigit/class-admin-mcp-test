#!/usr/bin/env bash
# Create a new profile file from the template:
#
#   <kit>/scripts/new-profile.sh risk strict      -> <kit>/risk_profiles/strict.toml
#   <kit>/scripts/new-profile.sh runtime agentic  -> <kit>/runtime_profiles/agentic.toml
#
# Edit it, point an agent at it (risk_profile = "strict" in agent.toml), then
# install with install-agent.sh (installs missing profiles automatically)
# or install-profile.sh.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

[[ $# -eq 2 ]] || zc_die "usage: $0 <risk|runtime> <name>"
zc_kind "$1"
NAME="$2"
zc_check_name "$NAME"

DEST="$KIT/$KIND_TABLE/$NAME.toml"
[[ ! -e "$DEST" ]] || zc_die "already exists: $DEST"

mkdir -p "$KIT/$KIND_TABLE"
cp "$KIT/templates/$KIND_HEADER.toml" "$DEST"
echo "created $DEST"
