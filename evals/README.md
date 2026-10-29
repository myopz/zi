# Evals (smoke)

Lightweight checks that the workspace builds a runnable `zi` binary and documents stay aligned with session v3.

Project docs: [docs/README.md](../docs/README.md).

## smoke.sh

From the repo root:

```bash
./evals/smoke.sh
```

What it does:

1. `cargo run -p zi-coding-agent -- --help` — CLI parses and prints usage (no API key required).
2. If `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` is set, runs a minimal print prompt:
   `cargo run -p zi-coding-agent -- -p 'Reply with exactly: ok'` with `--no-session` to avoid writing under `~/.zi/agent`.

Exit code is non-zero if `--help` fails. The optional live LLM step is best-effort: failures print a warning but do not fail the script (network/provider errors).

## Fixtures

| File | Purpose |
|------|---------|
| `fixtures/session_v3_sample.jsonl` | Minimal valid v3 file (header + user message + model_change) for manual `zi-session` roundtrip notes |

Example (requires `cargo test` — not run by smoke):

```bash
# Parse-only sanity: load lines in a small Rust test or REPL using zi_session::load_entries_from_path
```

## Future

Full pi-style eval suites (golden transcripts, provider mocks) will hook here and in CI once catalog generation and WASM extensions stabilize. See `xtask/README.md` for planned catalog/check tasks.
