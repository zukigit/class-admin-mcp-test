# Shared helpers for new.sh and install.sh. Source it; don't run it.
#
# Every kit item is a folder whose name is the item's name, holding one
# settings file with a name-free table header:
#
#   type     folder                    file                  header             config table
#   agent    agents/<name>/            agent.toml            [agent]            [agents.<name>]
#   risk     risk_profiles/<name>/     risk_profile.toml     [risk_profile]     [risk_profiles.<name>]
#   runtime  runtime_profiles/<name>/  runtime_profile.toml  [runtime_profile]  [runtime_profiles.<name>]
#
# Installing renames the headers (sub-tables too, e.g. [agent.memory] ->
# [agents.<name>.memory]) and appends them to ZeroClaw's config.toml.

KIT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ZC_TYPES=(agent risk runtime)

zc_die() { echo "$*" >&2; exit 1; }

zc_check_name() {
  [[ "$1" =~ ^[a-z0-9_-]+$ ]] || zc_die "name must be lowercase letters, digits, _ or -: $1"
}

# zc_type <agent|risk|runtime> -> T_HEADER, T_TABLE, T_FILE, T_DIR
zc_type() {
  case "$1" in
    agent|agents)                             T_HEADER=agent ;;
    risk|risk_profile|risk_profiles)          T_HEADER=risk_profile ;;
    runtime|runtime_profile|runtime_profiles) T_HEADER=runtime_profile ;;
    *) zc_die "unknown type '$1' (use: ${ZC_TYPES[*]})" ;;
  esac
  T_TABLE="${T_HEADER}s"
  T_FILE="$T_HEADER.toml"
  T_DIR="$KIT/$T_TABLE"
}

# zc_detect <folder> -> sets T_* from whichever settings file the folder has
zc_detect() {
  local t
  for t in "${ZC_TYPES[@]}"; do
    zc_type "$t"
    [[ -f "$1/$T_FILE" ]] && return 0
  done
  zc_die "not a kit folder (no agent.toml, risk_profile.toml or runtime_profile.toml): $1"
}

# zc_ensure_config <data-dir> -> sets CONFIG, creates it from the base if missing
zc_ensure_config() {
  mkdir -p "$1/.zeroclaw"
  CONFIG="$(cd "$1/.zeroclaw" && pwd)/config.toml"
  if [[ ! -f "$CONFIG" ]]; then
    cp "$KIT/templates/config.base.toml" "$CONFIG"
    echo "created $CONFIG from templates/config.base.toml"
  fi
}

# zc_has <table> <name>: is [<table>.<name>] in $CONFIG?
zc_has() { grep -qE "^\[$1\.$2(\.|\])" "$CONFIG"; }

# zc_remove <table> <name>: drop [<table>.<name>] and its sub-tables from $CONFIG
zc_remove() {
  cp "$CONFIG" "$CONFIG.bak"
  awk -v t="$1" -v n="$2" '
    /^\[/ { skip = ($0 ~ "^\\[" t "\\." n "(\\.|\\])") }
    !skip
  ' "$CONFIG.bak" > "$CONFIG"
  echo "removed old $1.$2 (backup: $CONFIG.bak)"
}

# zc_value <file> <key>: first string value of `key = "..."`
zc_value() { sed -nE "s/^$2 *= *\"([^\"]+)\".*/\1/p" "$1" | head -1; }

# zc_install <folder> <force 0|1>: install one kit folder into $CONFIG
zc_install() {
  local src force="$2" name
  src="$(cd "$1" && pwd)"
  name="$(basename "$src")"
  zc_check_name "$name"
  zc_detect "$src"
  local header="$T_HEADER" table="$T_TABLE" file="$T_FILE"

  if zc_has "$table" "$name"; then
    [[ "$force" -eq 1 ]] || zc_die "$table.$name is already installed; use --force to replace it"
    zc_remove "$table" "$name"
  fi
  {
    echo
    echo "# --- $table.$name ---"
    sed -E -e '/^[[:space:]]*#/d' -e '/^[[:space:]]*$/d' \
           -e "s/^\[$header(\.|\])/[$table.$name\1/" "$src/$file"
  } >> "$CONFIG"
  echo "added $table.$name"

  [[ "$header" == agent ]] && zc_install_agent_extras "$src" "$name" "$force"
  return 0
}

# Agent-only steps: missing profiles it uses, provider check, workspace files.
zc_install_agent_extras() {
  local src="$1" name="$2" force="$3" kind profile f dest
  for kind in risk runtime; do
    zc_type "$kind"
    profile="$(zc_value "$src/agent.toml" "$T_HEADER")"
    [[ -n "$profile" ]] || continue
    zc_has "$T_TABLE" "$profile" && continue
    if [[ -f "$T_DIR/$profile/$T_FILE" ]]; then
      zc_install "$T_DIR/$profile" 0
    else
      echo "warning: $T_TABLE.$profile is not in the config or the kit" >&2
    fi
  done

  local provider
  provider="$(zc_value "$src/agent.toml" model_provider)"
  if [[ -n "$provider" ]] && ! grep -qF "[providers.models.$provider]" "$CONFIG"; then
    echo "warning: provider '$provider' is not defined in $CONFIG" >&2
  fi

  local ws
  ws="$(dirname "$CONFIG")/agents/$name/workspace"
  mkdir -p "$ws"
  if [[ -d "$src/workspace" ]]; then
    for f in "$src"/workspace/*; do
      dest="$ws/$(basename "$f")"
      if [[ -e "$dest" && "$force" -eq 0 ]]; then
        echo "  kept existing workspace/$(basename "$f")"
      else
        cp -r "$f" "$dest"
      fi
    done
  fi
  echo "  workspace: $ws"
}
