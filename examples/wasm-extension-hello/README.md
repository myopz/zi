# wasm-extension-hello

Minimal WASM guest for zi ABI v1 (**WAT** source). Install **outside** the zi source tree (agent or trusted project extensions).

Rust 版同等行为：[`../wasm-extension-hello-rs/`](../wasm-extension-hello-rs/)。端到端（开发 / 构建 / 安装 / 使用）：[`docs/EXTENSIONS.md`](../../docs/EXTENSIONS.md)。

## What it does

- On init: logs, registers `/hello`, subscribes to `agent_end`
- On `agent_end`: `ui_notify` with a short message
- On `/hello`: returns `hello from /hello`
- Manifest `contributes.prompts` ships `prompts/hello-ext.md`

## Install (user agent — preferred)

Does not require the zi crates checkout after you have the example files once (or any ABI v1 `.wasm`):

```bash
DEST="$HOME/.zi/agent/extensions/hello"
mkdir -p "$DEST/prompts"
cp extension.json "$DEST/"
cp -R prompts/. "$DEST/prompts/"
# build wasm (requires WABT):
wat2wasm hello.wat -o "$DEST/extension.wasm"
```

Restart zi (WASM load is not hot-swapped), then `/extensions` and `/hello`.

## Install (trusted project)

```bash
DEST=".zi/extensions/hello"
mkdir -p "$DEST/prompts"
cp extension.json "$DEST/"
cp -R prompts/. "$DEST/prompts/"
wat2wasm hello.wat -o "$DEST/extension.wasm"
```

Then `/trust yes`, restart zi, `/extensions`, `/hello`.

Do **not** install under `crates/zi-*`; those trees may be encrypted or unavailable.

## Smoke test (from a zi checkout)

```bash
cargo test -p zi-coding-agent wasm_hello -- --nocapture
```

The test assembles `hello.wat` in-process via the `wat` dev-dependency (no `wat2wasm` required). Default features already enable WASM execution.
