# 开发指南

## 工作区命令

```bash
# 首次 / 更换 host pack 路径后：把 ~/.zi/host-extensions（或 ZI_HOST_EXTENSIONS_PATH）链到仓库根
./scripts/link-host-extensions.sh

cargo build --workspace
cargo test -p zi-session -p zi-telemetry -p zi-agent   # 按需加包
cargo run -p zi-coding-agent -- --help
cargo run -p xtask -- check
./evals/smoke.sh
```

Host pack 源码默认在 `~/.zi/host-extensions`（仓库外）。覆盖位置：

```bash
export ZI_HOST_EXTENSIONS_PATH=/path/to/zi-host-extensions
# 可选：export ZI_SOURCE_PATH=/path/to/zi
./scripts/link-host-extensions.sh
```

从零写扩展（WASM 或 host pack）到在 zi 里验证：见 [EXTENSIONS.md](./EXTENSIONS.md)。Rust guest 主示例：`examples/wasm-extension-hello-rs/`（`rustup target add wasm32-unknown-unknown` 后在该目录 `cargo build --release`）。

Rust 版本：workspace `rust-version = "1.85"`。

## 推荐工作流

```mermaid
flowchart LR
  A[改代码] --> B[cargo build --workspace]
  B --> C[定向 cargo test -p …]
  C --> D[./evals/smoke.sh]
  D --> E[更新 docs/ 若合同变更]
```

- 改协议形状（`AgentEvent`、session、MCP）→ 同步 [COMPAT.md](./COMPAT.md)。
- 改 crate 边界或数据流 → 同步 [ARCHITECTURE.md](./ARCHITECTURE.md) / [CRATES.md](./CRATES.md)。
- 文档中的流程图优先用 **Mermaid**（见 [docs/README.md](./README.md)）。

## Cargo 镜像（crm）

国内拉取 crates.io 较慢时，可用 [crm](https://github.com/wtklbm/crm) 切换镜像（写入 `~/.cargo/config.toml`）：

```bash
cargo install crm --locked
crm use rsproxy-sparse          # 推荐
# crm use ustc-sparse
# crm use tuna-sparse
crm best sparse-download
crm current
```

若卡在 `Blocking waiting for file lock on package cache`：

```bash
pkill -9 -f 'cargo|rustc' || true
rm -f ~/.cargo/.package-cache ~/.cargo/.package-cache-mutate
```

## Feature flags（节选）

| Crate | Feature | 作用 |
|-------|---------|------|
| `zi-coding-agent` | `wasm-extensions`（**default**） | 执行外部 `~/.zi/agent/extensions/*.wasm`（wasmtime）；`--no-default-features` 可关掉 |
| `zi-mcp` | `oauth` | OAuth 相关 |
| `zi-durable` | `sqlite` | SQLite 存储 |
| `zi-codemode` | （见 crate） | QuickJS 相关可选依赖 |

## xtask

```bash
cargo run -p xtask -- check
```

| 命令 | 状态 |
|------|------|
| `check` | Stub：打印就绪说明 |
| `gen-catalog` | 规划中：从目录定义生成 `zi-ai` 模型数据 |

详见 [xtask/README.md](../xtask/README.md)。

## 二进制一览

| 命令 | 包 |
|------|-----|
| `zi` | `zi-coding-agent` |
| `zi-envd` | `zi-env` |

```bash
cargo run -p zi-coding-agent -- …
cargo run -p zi-env --bin zi-envd -- …
```
