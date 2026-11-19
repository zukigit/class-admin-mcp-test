#!/usr/bin/env bash
# Create a new kit item from its template. The folder name is the item's name.
#
#   ./new.sh agent   agent1      -> agents/agent1/            (agent.toml + workspace/*.md)
#   ./new.sh risk    strict      -> risk_profiles/strict/     (risk_profile.toml)
#   ./new.sh runtime fast        -> runtime_profiles/fast/    (runtime_profile.toml)
#   ./new.sh provider opencode.big -> providers/opencode.big/ (provider.toml)
#
# Edit the new folder, then install it with ./install.sh --data <dir> <folder>.
# Works from any directory; paths are relative to this kit folder.
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib/zeroclaw.sh"

[[ $# -eq 2 ]] || zc_die "usage: $0 <agent|risk|runtime|provider> <name>"
zc_type "$1"
NAME="$2"
zc_check_name "$NAME"

DEST="$T_DIR/$NAME"
[[ ! -e "$DEST" ]] || zc_die "already exists: $DEST"

mkdir -p "$T_DIR"
cp -r "$KIT/templates/$T_HEADER" "$DEST"
# Fill in the name in the template's text files.
# {{NAME_ENV}}: name in env-var path form (opencode.big -> opencode__big).
# {{KEY_VAR}}:  .env variable for a provider's key (opencode.big -> OPENCODE_API_KEY).
PROVIDER_ID="${NAME%%.*}"
KEY_VAR="$(printf '%s' "$PROVIDER_ID" | tr 'a-z-' 'A-Z_')_API_KEY"
find "$DEST" -type f \( -name '*.md' -o -name '*.toml' \) \
  -exec sed -i -e "s|{{NAME}}|$NAME|g" -e "s|{{NAME_ENV}}|${NAME//./__}|g" \
               -e "s|{{KEY_VAR}}|$KEY_VAR|g" {} +

echo "created $DEST"
if [[ "$T_KIND" == provider ]]; then
  echo "api key: add to docker-compose.yml (zeroclaw environment):"
  echo "  - ZEROCLAW_providers__models__${NAME//./__}__api_key=\${$KEY_VAR:?set $KEY_VAR in .env}"
  echo "  and $KEY_VAR=<key> to .env"
fi
echo "next: edit it, then run"
echo "  $KIT/install.sh --data <zeroclaw-data-dir> $DEST"
