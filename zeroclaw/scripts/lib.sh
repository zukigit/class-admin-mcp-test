# Shared helpers for the kit's scripts. Source it; don't run it.
#
# Kit items (agents, risk profiles, runtime profiles) are stored with a
# name-free table header, e.g. `[agent]`, `[risk_profile.delegation_policy]`.
# Installing renames them to the config's real tables, e.g.
# `[agents.<name>]`, `[risk_profiles.<name>.delegation_policy]`.

KIT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

zc_die() { echo "$*" >&2; exit 1; }

zc_check_name() {
  [[ "$1" =~ ^[a-z0-9_-]+$ ]] || zc_die "name must be lowercase letters, digits, _ or -: $1"
}

# zc_kind <risk|runtime> -> sets KIND_HEADER (risk_profile), KIND_TABLE (risk_profiles)
zc_kind() {
  case "$1" in
    risk|risk_profile|risk_profiles)          KIND_HEADER=risk_profile;    KIND_TABLE=risk_profiles ;;
    runtime|runtime_profile|runtime_profiles) KIND_HEADER=runtime_profile; KIND_TABLE=runtime_profiles ;;
    *) zc_die "unknown profile type '$1' (use risk or runtime)" ;;
  esac
}

# zc_ensure_config <data-dir> -> sets CONFIG, creates it from the base if missing
zc_ensure_config() {
  mkdir -p "$1/.zeroclaw"
  CONFIG="$1/.zeroclaw/config.toml"
  if [[ ! -f "$CONFIG" ]]; then
    cp "$KIT/templates/config.base.toml" "$CONFIG"
    echo "created $CONFIG from templates/config.base.toml"
  fi
}

# zc_has <config> <table> <name>   e.g. zc_has "$CONFIG" agents minmin
zc_has() { grep -qE "^\[$2\.$3(\.|\])" "$1"; }

# zc_remove <config> <table> <name>: drop [<table>.<name>] and its sub-tables
zc_remove() {
  cp "$1" "$1.bak"
  awk -v t="$2" -v n="$3" '
    /^\[/ { skip = ($0 ~ "^\\[" t "\\." n "(\\.|\\])") }
    !skip
  ' "$1.bak" > "$1"
  echo "removed old $2.$3 (backup: $1.bak)"
}

# zc_append <config> <file> <header> <table> <name>: rename tables, drop comments, append
zc_append() {
  {
    echo
    echo "# --- $4.$5 ---"
    sed -E -e '/^[[:space:]]*#/d' -e '/^[[:space:]]*$/d' \
           -e "s/^\[$3(\.|\])/[$4.$5\1/" "$2"
  } >> "$1"
  echo "added $4.$5 to $1"
}

# zc_value <file> <key>: first string value of `key = "..."`
zc_value() { sed -nE "s/^$2 *= *\"([^\"]+)\".*/\1/p" "$1" | head -1; }

# zc_install_profile <config> <risk|runtime> <name|file> <force 0|1>
zc_install_profile() {
  local config="$1" src="$3" force="$4" name
  zc_kind "$2"
  [[ -f "$src" ]] || src="$KIT/$KIND_TABLE/$src.toml"
  [[ -f "$src" ]] || zc_die "no such $KIND_HEADER: $src"
  name="$(basename "$src" .toml)"
  zc_check_name "$name"
  if zc_has "$config" "$KIND_TABLE" "$name"; then
    [[ "$force" -eq 1 ]] || zc_die "$KIND_TABLE.$name is already installed; use --force to replace it"
    zc_remove "$config" "$KIND_TABLE" "$name"
  fi
  zc_append "$config" "$src" "$KIND_HEADER" "$KIND_TABLE" "$name"
}
