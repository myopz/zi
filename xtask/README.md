# xtask

Workspace maintenance binaries (invoked as `cargo run -p xtask -- …`).

## Commands

| Command | Status |
|---------|--------|
| `check` | Stub — prints readiness notes; does not invoke `cargo` |
| `gen-catalog` | Planned — regenerate `zi-ai` model catalog from pi-style definitions (parity with pi `packages/ai/catalog/`) |

## Planned catalog generation

pi keeps generated model metadata in TypeScript; zi will add a generator here that:

1. Reads catalog source definitions (TOML/JSON or synced TS export).
2. Emits Rust into `crates/zi-ai/src/` (or a generated module included at build time).
3. Is run in CI via `cargo run -p xtask -- gen-catalog --check`.

Until then, `zi-ai` uses a hand-maintained static catalog subset.

## Usage

```bash
cargo run -p xtask -- check
```
