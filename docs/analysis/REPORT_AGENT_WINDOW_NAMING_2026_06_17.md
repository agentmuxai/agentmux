# Can an agent set the window/taskbar name? (e.g. "Starter Workspace")

**Date:** 2026-06-17
**Question:** Does the agent-facing app API have a binding to set the OS window name as it appears in the taskbar, so an agent can launch `task dev` with a recognizable name for easy ID?
**Short answer:** **No binding exists for agents today.** The native taskbar-title plumbing is fully implemented, but the only way to drive it is a frontend-reactive meta key (`window:displayname`) set over the WebSocket RPC by the in-app UI. There is **no MCP tool, no HTTP `/api/v1` endpoint, and no launch-time flag/env var.** Adding one is small — see §5.

---

## 1. How the taskbar title actually works (verified)

The native window caption (what Windows shows in the taskbar) is **derived from the HTML `document.title`**, one-directionally, frontend → host:

1. **Frontend computes** `document.title` reactively in `installWindowTitleEffect()` (`frontend/app-init.ts` ~L675–750, ~L877).
   Format (`frontend/util/window-title.ts`):
   ```
   {Window Name} - {Tab Name} - AgentMux
   ```
   **Window Name** is a 3-tier fallback (`resolveWindowName`, window-title.ts:33-39):
   1. `WaveWindow.meta["window:displayname"]` (user-set, ≤64 chars)
   2. assigned `workspace.name`
   3. positional `"Window N"`

2. **CEF mirrors it to the OS.** When `document.title` changes, CEF fires `on_title_change`, which sets the native caption (`agentmux-cef/src/client/mod.rs:205-243`):
   - macOS/Linux: `window.set_title(...)` (CEF Views)
   - **Windows: `SetWindowTextW(hwnd, …)`** (L224-243) — this is the taskbar text. Guarded to skip when CEF passes `None` (reagent P1 #876).

So **the binding to the taskbar exists and works** — it's just keyed entirely off `document.title`, which is computed in the renderer.

## 2. Where "Starter workspace" comes from

It's the hardcoded **default workspace name** created on first DB init:
`agentmux-srv/src/backend/wcore/mod.rs:85` → `create_workspace(store, "Starter workspace")`.

Because Window Name falls back to the workspace name (tier 2), a window with no explicit `window:displayname` and the default workspace shows **"Starter workspace - <tab> - AgentMux"** in the taskbar. That's why you see it.

## 3. How the name is set today (and by whom)

`window:displayname` is written **only** from the frontend UI, via the generic object-meta RPC:

- `ObjectService.UpdateObjectMeta(makeORef("window", id), { "window:displayname": name })`
  - InstancePanel rename: double-click a window row or **F2** (`frontend/app/statusbar/InstancePanel.tsx:226`)
- `UpdateObjectMeta` is a **backend service call routed through the reducer** (`agentmux-srv/src/server/service.rs:311`), reached via `WOS.callBackendService("object", "UpdateObjectMeta", …)` — i.e. over the **WebSocket `/ws` RPC**, not a REST endpoint.

## 4. What the agent API exposes today — and the gap

Agents talk to their sidecar with `AGENTMUX_LOCAL_URL` + `X-AuthKey`. The surface:

| Channel | Available to agents | Window-naming capability |
|---|---|---|
| **MCP tools** (`agentmux-mcp`): Shell, ShellStop, OpenEditor, SendMessage, DiscoverAgents | yes | ❌ none touch window/title |
| **HTTP `/api/v1/*`** (shell/create, shell/stop, pane/open) and `/agentmux/*` (reactive, discovery) | yes | ❌ no `window`/`meta`/`rename`/`title` route exists (grep of all `.route(` = none) |
| **WebSocket `/ws`** RPC incl. `setmeta` / `UpdateObjectMeta` | technically auth-gated, but this is the **frontend's** channel; agents have no client for it | ⚠️ could set `window:displayname` *if* it had the window ID and a WS client |

**Two structural problems for the "name my `task dev`" use case even via `/ws`:**
1. **No agent client for the WS RPC** — agents use the MCP/HTTP surface, not the SolidJS `ObjectService`.
2. **`task dev` spawns a *separate* AgentMux instance** — its own sidecar, own `auth_key`, own window IDs. An agent running in instance A cannot reach instance B's sidecar to set B's window name. The name has to be applied **by the new instance, at its own startup.**

**Conclusion:** there is no quick path today. The natural channel for "launch `task dev` already named" is **launch-time configuration the new instance reads about itself**, not a cross-instance RPC.

## 5. Recommended implementation (smallest thing that fits the goal)

**Option A — launch-time env var `AGENTMUX_WINDOW_NAME` (recommended).**
Ride it on the command the agent already runs:
```bash
AGENTMUX_WINDOW_NAME="PR-1503 repro" task dev
```
On window init, if the env var is set, seed `window:displayname` on the window object before `installWindowTitleEffect` first runs. Two viable wiring points:
- **Backend startup** (`wcore`): when creating/attaching the window, if `AGENTMUX_WINDOW_NAME` is set, write it into the window meta. Pro: one place, persists; works for first-launch and is read by the same reactive effect.
- **Frontend init** (`app-init.ts`): read the value (exposed by the host) and call `UpdateObjectMeta` once at boot. Pro: reuses the exact path the UI uses.

Effort: ~½ day. No new API surface, no cross-instance concerns, composes with `task dev`. **Best fit for the stated goal.**

**Option B — MCP `SetWindowName` tool + HTTP `POST /api/v1/window/name`.**
Lets a *running* agent rename *its own* instance's window at runtime (needs the window ID; resolvable since the sidecar knows its windows). Pro: also useful outside dev. Con: does **not** by itself solve "launch `task dev` pre-named" because of the separate-instance problem in §4 — you'd still want Option A for that. Effort: ~1 day (new route + reducer call + MCP tool + tests).

**Option C — `task dev` convenience flag.**
e.g. `task dev -- --name "X"` that just sets `AGENTMUX_WINDOW_NAME` for the child. Thin sugar on Option A. Effort: ~1 hr once A exists.

**Suggested path:** ship **A** (covers the dev-window ID use case directly), optionally add **C** as sugar. Add **B** later only if agents need to rename running windows generally.

## 6. Key file references

| What | File:line |
|---|---|
| Title format + 3-tier resolution | `frontend/util/window-title.ts:11-55` |
| Reactive `document.title` effect | `frontend/app-init.ts` ~675-750, ~877 |
| Native caption set (Win32 `SetWindowTextW`) | `agentmux-cef/src/client/mod.rs:205-243` |
| Default "Starter workspace" name | `agentmux-srv/src/backend/wcore/mod.rs:85` |
| Rename UI (writes `window:displayname`) | `frontend/app/statusbar/InstancePanel.tsx:226` |
| `UpdateObjectMeta` reducer route (WS RPC) | `agentmux-srv/src/server/service.rs:311` |
| Window-meta host commands (no displayname setter) | `agentmux-cef/src/commands/window/meta.rs` |
| HTTP router (no window/title route) | `agentmux-srv/src/server/mod.rs` (`.route(` set) |
| MCP tools (no window tool) | `agentmux-mcp/src/main.rs` |
