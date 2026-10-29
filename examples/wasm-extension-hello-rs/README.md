# wasm-extension-hello-rs

用 **Rust** 实现的最小 zi WASM ABI v1 guest（行为对齐 [`../wasm-extension-hello/`](../wasm-extension-hello/) 的 WAT 版）。

端到端步骤（开发 → 构建 → 安装 → 在 zi 里用）见文档：[`docs/EXTENSIONS.md`](../../docs/EXTENSIONS.md) 中的 **「完整示例：Rust 开发 WASM 扩展」**。

## 行为

- `zi_extension_init`：log、注册 `/hello-rs`、订阅 `agent_end`
- `agent_end`：`ui_notify` 一条短消息
- `/hello-rs`：返回 `hello from /hello-rs`
- `contributes.prompts`：附带 `prompts/hello-rs.md`

## 快速构建并安装

```bash
# 一次：安装 wasm 目标
rustup target add wasm32-unknown-unknown

cd examples/wasm-extension-hello-rs
cargo build --release
# → target/wasm32-unknown-unknown/release/wasm_extension_hello_rs.wasm
#   （crate 名里的连字符会变成下划线）

DEST="$HOME/.zi/agent/extensions/hello-rs"
mkdir -p "$DEST/prompts"
cp extension.json "$DEST/"
cp -R prompts/. "$DEST/prompts/"
cp target/wasm32-unknown-unknown/release/wasm_extension_hello_rs.wasm \
  "$DEST/extension.wasm"
```

重启 `zi`，然后 `/extensions`、`/hello-rs`。换 `.wasm` 后须再重启（`/reload` 不热替换模块字节）。

本目录是**独立 crate**，不是 workspace member；不要把它加进根 `Cargo.toml` 的 `members`。
