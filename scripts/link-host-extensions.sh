#!/usr/bin/env bash
# Wire build-time host pack source for Cargo path deps.
#
# Defaults:
#   ZI_HOST_EXTENSIONS_PATH  →  ~/.zi/host-extensions
#   ZI_SOURCE_PATH           →  this zi repo (parent of scripts/)
#
# Cargo cannot expand env in `path = ...`, so:
#   1) <zi>/host-extensions  →  $ZI_HOST_EXTENSIONS_PATH
#   2) rewrite that crate's Cargo.toml with absolute paths to zi-ai / zi-session
#      (a symlink back into zi would make Cargo see duplicate workspace members)

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${ZI_HOST_EXTENSIONS_PATH:-${HOME}/.zi/host-extensions}"
ZI_SRC="${ZI_SOURCE_PATH:-${ROOT}}"
LINK="${ROOT}/host-extensions"
TEMPLATE="${TARGET}/Cargo.toml.in"
GENERATED="${TARGET}/Cargo.toml"

if [[ ! -d "${TARGET}" ]]; then
  echo "error: host extensions crate not found at:" >&2
  echo "  ${TARGET}" >&2
  echo "Clone or move zi-host-extensions there, or set ZI_HOST_EXTENSIONS_PATH." >&2
  exit 1
fi

if [[ ! -f "${ZI_SRC}/crates/zi-ai/Cargo.toml" || ! -f "${ZI_SRC}/crates/zi-session/Cargo.toml" ]]; then
  echo "error: zi source not found at:" >&2
  echo "  ${ZI_SRC}" >&2
  echo "Set ZI_SOURCE_PATH to the zi repo root." >&2
  exit 1
fi

if [[ ! -f "${TEMPLATE}" ]]; then
  echo "error: missing template ${TEMPLATE}" >&2
  exit 1
fi

# Remove stale back-link from earlier approach (if present).
rm -f "${TARGET}/zi-src"

ZI_AI="${ZI_SRC}/crates/zi-ai"
ZI_SESSION="${ZI_SRC}/crates/zi-session"
sed \
  -e "s|@ZI_AI@|${ZI_AI}|g" \
  -e "s|@ZI_SESSION@|${ZI_SESSION}|g" \
  "${TEMPLATE}" > "${GENERATED}"

ln -sfn "${TARGET}" "${LINK}"
echo "linked ${LINK} -> ${TARGET}"
echo "wrote ${GENERATED} (zi-ai=${ZI_AI})"
