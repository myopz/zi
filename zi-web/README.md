# zi-web

Browser UI for zi coding-agent web mode: OpenClaw-style token login, three-column shell (default light / white background; Settings → Theme for light/dark/system), IndexedDB cache, and WebSocket AgentEvent streaming.

## Prerequisites

- Node 22+ and [pnpm](https://pnpm.io/)
- zi backend listening on `127.0.0.1:3080` (`zi --mode web`, once available)

## Setup

```bash
cd zi-web
pnpm install
```

## Develop (Vite + proxy)

Vite proxies `/api` and `/ws` to the Rust host:

```bash
# terminal 1 — backend (from repo root)
cargo run -p zi-coding-agent -- --mode web
# seed a user if the DB is empty, then copy the printed token

# terminal 2 — frontend
cd zi-web
pnpm dev
```

Open the printed Vite URL (usually `http://127.0.0.1:5173`).

Login:

1. Paste the seeded token, or
2. Open `http://127.0.0.1:5173/#token=<raw>` — the hash is stripped immediately and used once.

The access token is stored in `sessionStorage` only (this tab + origin). Logout clears sessionStorage and the per-user IndexedDB database (`zi-web:<userId>`).

## Build

```bash
pnpm build
pnpm preview   # optional static preview; API still needs the backend or a reverse proxy
```

If you open **`http://127.0.0.1:3080`** (`zi --mode web`), the UI is the **built** `zi-web/dist`, not Vite HMR. After frontend changes you must `pnpm build` again (and hard-refresh the browser). For day-to-day UI work prefer `pnpm dev` on the Vite port with the API proxy.

Production: serve `dist/` from the axum static handler (or any CDN in front of `/api` + `/ws`).

## Architecture (frontend)

| Area | Location |
|------|----------|
| Auth / `#token=` | `src/lib/auth.ts`, `src/pages/Login.tsx` |
| REST client | `src/lib/api.ts` |
| IndexedDB (Dexie) | `src/lib/idb/` |
| WS + AgentEvent reducer | `src/lib/ws.ts`, `src/lib/transcript/reducer.ts` |
| Shell / chat UI | `src/components/layout/`, `src/components/chat/` |

API contract (must match the web host):

- `POST /api/auth/login` `{ token }` → `{ accessToken, user: { id, username } }`
- Bearer on `GET /api/me`, workspaces, sessions, models, status
- WS `/ws`: first frame `{ type: "auth", token }`, then `{ type: "prompt"|"abort", sessionId, message? }`; server sends AgentEvent JSON, `{ type: "response", ... }`, `{ type: "session_meta", sessionId, revision }`
