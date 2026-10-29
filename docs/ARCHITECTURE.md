# 架构

zi 是按 pi monorepo 包边界组织的 Rust workspace。两条产品路径共享 telemetry 与 AI 类型：

1. **CLI 路径** — 本地 coding agent（二进制 `zi`）。
2. **Durable 路径** — 托管 harness（`zi-durable` + `zi-chord` + `zi-env`）。

## 双路径总览

```mermaid
flowchart TB
  subgraph shared [共享]
    TEL[zi-telemetry]
    AI[zi-ai]
  end

  subgraph cliPath [CLI]
    CA[zi-coding-agent / zi]
    AG[zi-agent]
    TUI[zi-tui]
    SES[zi-session]
    MCP[zi-mcp]
    CM[zi-codemode]
    CA --> AG
    CA --> TUI
    CA --> SES
    AG --> AI
    AG --> MCP
    CA --> CM
  end

  subgraph durablePath [Durable]
    DUR[zi-durable]
    CHD[zi-chord]
    ENV[zi-env / zi-envd]
    DUR --> CHD
    DUR --> ENV
    DUR --> CM
    CHD --> TEL
  end

  AG --> TEL
  AI --> TEL
```

## Crate 依赖（简化）

```mermaid
flowchart BT
  TEL[zi-telemetry]
  SES[zi-session]
  AI[zi-ai]
  AG[zi-agent]
  MCP[zi-mcp]
  TUI[zi-tui]
  CM[zi-codemode]
  CA[zi-coding-agent]
  CHD[zi-chord]
  DUR[zi-durable]
  ENV[zi-env]

  AI --> TEL
  AG --> AI
  AG --> TEL
  CA --> AG
  CA --> TUI
  CA --> SES
  CA --> MCP
  CA --> CM
  CA --> TEL
  CHD --> TEL
  DUR --> CHD
  DUR --> CM
  ENV --> DUR
```

| Crate | 角色 |
|-------|------|
| `zi-telemetry` | 追踪/指标上下文（CLI 默认 NOOP） |
| `zi-session` | Session v3 NDJSON 读写与迁移 |
| `zi-ai` | 模型、Provider HTTP/SSE、`AssistantMessageEvent` |
| `zi-agent` | Agent 循环、工具、`AgentEvent` 广播 |
| `zi-mcp` | MCP 客户端（stdio、streamable HTTP） |
| `zi-codemode` | QuickJS codemode DSL 宿主 |
| `zi-tui` | 终端 UI 与交互驱动 |
| `zi-coding-agent` | `zi` CLI：print / json / rpc / interactive |
| `zi-chord` | Chord v2 风格 host / store / contracts |
| `zi-durable` | Harness、compaction、memory/jsonl/sqlite |
| `zi-env` | `zi-envd` 与本地/远程执行客户端 |

更细的对照见 [CRATES.md](./CRATES.md)。

## 数据流：prompt → tools → events

```mermaid
flowchart TD
  U[User: -p / RPC / TUI] --> M[zi-coding-agent modes]
  M --> BR[build_runtime]
  BR --> A[Agent::prompt]
  A --> LOOP[run_agent_loop]

  LOOP --> SF[stream_fn]
  SF --> PROV[zi-ai provider SSE]
  PROV --> AME[AssistantMessageEvent]
  AME --> AE1[AgentEvent Message*]

  LOOP --> TC[tool calls]
  TC --> TOOLS[coding-agent tools]
  TC --> MCP[zi-mcp mounts]
  TOOLS --> AE2[AgentEvent Tool*]
  MCP --> AE2

  AE1 --> OUT[print / json / rpc / TUI]
  AE2 --> OUT
  OUT --> SESS[zi-session append]
  SESS --> DISK["~/.zi/agent/sessions/"]
```

### 事件分层

```mermaid
flowchart LR
  W[Wire: Provider SSE/HTTP] --> AME[AssistantMessageEvent]
  AME --> AE[AgentEvent]
  AE --> UI[CLI / TUI / JSONL]
  AE --> ND[Session v3 NDJSON]
```

1. **Wire** — 各厂商 chunk，在 `zi-ai` adapter 内归一化。
2. **AssistantMessageEvent** — 流式 partial assistant message。
3. **AgentEvent** — 回合生命周期、工具执行、终态消息；UI 与 `--mode json` 消费。
4. **Session entries** — 持久化的 `message` 等 v3 行。

## Durable 路径（摘要）

```mermaid
sequenceDiagram
  participant H as Harness
  participant S as Storage
  participant E as zi-envd
  participant C as Chord host

  H->>S: append / commit
  H->>E: exec / read / write frames
  E-->>H: RESULT / EVENT
  H->>C: get / call objects
  C-->>H: Value / Reference
```

`zi-durable` 提供可插拔存储；`zi-env` 通过帧协议做沙箱文件/执行；`zi-chord` 提供长生命周期协调原语（对齐 pi Chord v2 slice）。

## 相关文档

- [COMPAT.md](./COMPAT.md) — 跨语言合同
- [SESSION.md](./SESSION.md) — 会话格式
- [CLI.md](./CLI.md) — 用户入口行为
