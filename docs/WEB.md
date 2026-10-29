# Web UI（`zi --mode web`）

多用户本地 Web 宿主：OpenClaw 风格 **token 登录**、SQLite 权威存储、进程内按 `(userId, sessionId)` 隔离 agent runtime。前端在仓库 [`zi-web/`](../zi-web/)；本页描述 **服务端** 行为与 API 合同。

## 快速开始

```bash
# 可选：预先创建用户（明文 token 只打印一次；库内存 sha256）
cargo run -p zi-coding-agent -- web seed-user alice
cargo run -p zi-coding-agent -- web list-users

# 启动宿主（默认 127.0.0.1:3080；空库会自动 seed alice/bob/admin 并打印 token）
cargo run -p zi-coding-agent -- --mode web --no-open

# 可选：构建前端静态资源后由同一进程托管
pnpm --dir zi-web build
cargo run -p zi-coding-agent -- --mode web
```

开发时也可让 Vite 代理 `/api`、`/ws` 到 `127.0.0.1:3080`，见 `zi-web` 配置。

## 数据与路径

| 项 | 路径 / 说明 |
|----|-------------|
| SQLite | `$ZI_AGENT_DIR/web/zi-web.sqlite`（默认 `~/.zi/agent/web/zi-web.sqlite`） |
| 静态资源 | `zi-web/dist`（或 `ZI_WEB_DIST`）；缺失时仅 API/WS |
| 与 TUI NDJSON | **首版不强求双向同步**；Web 权威读写走 SQLite |

表：`users`、`user_tokens`（hash）、`workspaces`、`sessions`、`messages`；`schema_version = 1`。

## 鉴权

1. `POST /api/auth/login` `{ "token": "…" }` → `{ accessToken, user: { id, username } }`
2. 之后 REST / WS：`Authorization: Bearer <accessToken>`
3. **首版 accessToken = login token**（共享 secret）；库表预留后续可吊销 session
4. `POST /api/auth/logout` 服务端 no-op；客户端清 `sessionStorage`
5. 租户边界：`user_id`；只能看见自己的 workspace / session

管理命令：

```bash
zi web seed-user <username> [--token <raw>]
zi web list-users
```

## CLI 标志（web 模式）

| 标志 | 默认 | 含义 |
|------|------|------|
| `--mode web` | — | 启动 axum 宿主 |
| `--host` | `127.0.0.1` | 绑定地址（非 loopback 会告警） |
| `--port` | `3080` | 端口 |
| `--no-open` | off | 不自动打开浏览器 |
| `--model` | — | 新建 session 时的默认模型提示 |

## API 合同

公开：

- `GET /api/health` → `{ ok: true }`
- `POST /api/auth/login` `{ token }` → `{ accessToken, user }`

需 Bearer（JSON 字段均为 camelCase）：

- `POST /api/auth/logout`
- `GET /api/me` → `{ id, username }`
- `GET /api/defaults` → `{ defaultWorkspaceCwd }`（`$ZI_AGENT_DIR/web/workspaces/<userId>`，在 `~/.zi` 下，**不是** git 仓库）
- `GET|POST /api/workspaces` → workspace 含 `userId`；创建时 `cwd` 支持 `~/…`，空字符串则用默认 sandbox；目录不存在会自动创建
- `POST /api/workspaces/ensure-default` → 保证名为 `Default` 的工作区 cwd 为 `$ZI_AGENT_DIR/web/workspaces/<userId>`（把误指 git 仓库的旧 Default 迁过去）
- `PATCH /api/workspaces/:id` `{ name }` → 重命名 workspace
- `DELETE /api/workspaces/:id` → 删除 workspace（级联 sessions/messages）
- `GET /api/sessions?workspaceId=` → session 列表（可按 workspace 过滤）
- `POST /api/sessions` `{ workspaceId, title?, model? }`
- `GET /api/sessions/:id` → `{ session, messages, revision }`；`messages` 为 AgentMessage JSON 数组
- `PATCH /api/sessions/:id` `{ title }` → 重命名 session
- `DELETE /api/sessions/:id`
- `GET /api/models` → `{ models: [{ id, name, provider, api }], current? }`
- `POST /api/model` `{ model, sessionId? }` → `{ model }`
- `GET /api/status?sessionId=` → `{ sessionId?, busy, model?, revision? }`

WebSocket `GET /ws`：连接时 Bearer，或首帧 `{ "type": "auth", "token": "…" }`。

上行：

- `{ "type": "prompt", "sessionId", "message", "id"? }`
- `{ "type": "abort", "sessionId", "id"? }`

下行：`AgentEvent` JSON、`{ type: "response", command, success, error? }`、`{ type: "session_meta", sessionId, revision }`。

Agent 一轮结束后把终态消息写入 `messages` 并 bump `sessions.revision`。

## 实现落点

- `crates/zi-coding-agent/src/modes/web.rs`
- `crates/zi-coding-agent/src/web/`：`db`、`auth`、`routes`、`ws`、`state`、`seed`
