#!/usr/bin/env bash
# Minimal smoke: CLI --help, optional one-shot -p when an API key is present.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "== zi smoke: --help =="
cargo run -p zi-coding-agent -- --help

if [[ -n "${ANTHROPIC_API_KEY:-}" || -n "${OPENAI_API_KEY:-}" ]]; then
  echo "== zi smoke: live -p (no session file) =="
  set +e
  cargo run -p zi-coding-agent -- --no-session -p 'Reply with exactly: ok'
  status=$?
  set -e
  if [[ "$status" -ne 0 ]]; then
    echo "warning: live prompt exited $status (provider/network); smoke still passed --help" >&2
  fi
else
  echo "== zi smoke: skipping live -p (set ANTHROPIC_API_KEY or OPENAI_API_KEY to enable) =="
fi

echo "== zi smoke: ok =="
