#!/usr/bin/env bash
# Install a saved profile into a ZeroClaw data dir:
#
#   <kit>/scripts/install-profile.sh --data <zeroclaw-data> risk default
#   <kit>/scripts/install-profile.sh --data <zeroclaw-data> runtime path/to/fast.toml
#   <kit>/scripts/install-profile.sh --data <zeroclaw-data> risk strict --force
#
# A bare name resolves to <kit>/risk_profiles/<name>.toml or
# <kit>/runtime_profiles/<name>.toml. The profile is named after its file.
# --force replaces a profile that is already in the config (a .bak is kept).
#
# Afterwards restart ZeroClaw, e.g. docker compose restart zeroclaw
set -euo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

usage() { zc_die "usage: $0 --data <zeroclaw-data-dir> <risk|runtime> <name|file> [--force]"; }

ARGS=(); DATA_DIR=""; FORCE=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    -d|--data) [[ $# -ge 2 ]] || usage; DATA_DIR="$2"; shift 2 ;;
    --data=*)  DATA_DIR="${1#--data=}"; shift ;;
    --force)   FORCE=1; shift ;;
    -h|--help) usage ;;
    -*)        echo "unknown option: $1" >&2; usage ;;
    *)         ARGS+=("$1"); shift ;;
  esac
done
[[ ${#ARGS[@]} -eq 2 && -n "$DATA_DIR" ]] || usage

zc_ensure_config "$DATA_DIR"
zc_install_profile "$CONFIG" "${ARGS[0]}" "${ARGS[1]}" "$FORCE"
echo "next: docker compose restart zeroclaw"
