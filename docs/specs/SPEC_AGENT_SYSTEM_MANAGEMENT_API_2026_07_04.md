# Agent API: System Management Surface (reload, process/render diagnostics, saga health)

**Date:** 2026-07-04
**Status:** Draft
**Author:** AgentA
**Related:**
- `docs/specs/SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md` — the naming/layout/introspection surface this extends; same conventions apply (self-context defaulting, capability tiers, MCP→REST→gateway layering).
- Incident that triggered this: 2026-07-04 session — `sysinfo`/`swarm` panes were pruned from a live layout tree (cause still unconfirmed) leaving a stale-rendered empty cell, and the operator had **no remote way to reload the window** or **query renderer/process counts** to diagnose it. Both had to be improvised: renderer counts via an ad-hoc `Get-CimInstance Win32_Process` PowerShell shellout, no path at all for a remote reload.
- Also motivated by verifying PR #1957 (`fix(browser-pane): app-owned wrapper HWND fixes Windows renderer leak on pane close`) — its own test plan flagged renderer-process-count verification as the one item it couldn't independently confirm; a first-class RPC for this would have made that trivial instead of ad-hoc.

---

## 1. Premise

Today an agent that needs to *observe or recover the app it's running in* — reload a stuck window, check whether a renderer actually tore down, see saga backlog — has no first-class way to do it. It must fall back to shelling out to OS tools (`tasklist`, `Get-CimInstance`) against a process it doesn't otherwise have a handle on, or ask the human to click through a menu. This spec adds a **`system` domain**: read-only diagnostics (process/render/saga health) plus a small number of safe self-recovery verbs (reload), following the exact layering and tiering convention `SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md` established for naming/layout.

---

## 2. Does RPC auto-bind to MCP? No — verified.

There is **no automatic binding**. Three independent layers exist, and a new ability requires hand-written code in each:

```
MCP tool   (agentmux-mcp/src/main.rs — a Rust const JSON schema + a call_tool() match arm)
   └── REST verb   (agentmux-srv/src/server/mod.rs route + handler — typed request/response)
          └── dispatch_service / host command  (the actual reducer/service call, or an agentmux-cef IPC command)
```

Concretely, adding one new MCP-visible ability today means touching:
1. A `const FOO_TOOL: &str = r#"{ "name": "Foo", ... }"#;` JSON schema literal in `agentmux-mcp/src/main.rs`.
2. Adding it to the `tools/list` array (currently enumerated by hand, one `Value` binding per tool — see `main.rs:476-515`).
3. A `"Foo" => { ... }` arm in `call_tool()` that does an HTTP call.
4. A REST route + handler in `agentmux-srv/src/server/mod.rs` (if none of the existing 47+ `dispatch_service` methods already covers it).
5. Possibly a new `dispatch_service` match arm, or (for anything host/window/process-level) a new `agentmux-cef` command — **nothing in that layer is agent-reachable at all today** (see §4).

This confirms the premise in the question: **RPC commands are not auto-exposed to MCP.** The `/agentmux/service` gateway (`dispatch_service`) is reachable over plain HTTP by anything with the auth key (including an ad-hoc script, as used during tonight's investigation), but it is *not* the same thing as an MCP tool — no MCP tool exists unless someone wrote the three layers above by hand. This is a deliberate, documented choice (§5 of the first-class-surface spec): the raw gateway is "internal/advanced," curated verbs are "the supported, documented surface" — but it means coverage is only as complete as someone has gotten around to wrapping.

---

## 3. What exists today (inventory, verified against `main` @ `ca7ff4d3`)

### 3.1 MCP tools (28, `agentmux-mcp/src/main.rs`)

| Category | Tools |
|---|---|
| Shell/process (agent's own subprocess) | `Shell`, `ShellStop`, `ShellInput`, `ShellStatus` |
| Panes/editor | `OpenEditor` |
| Messaging | `SendMessage`, `DiscoverAgents` |
| Self-context / naming (§4 of the first-class-surface spec) | `WhoAmI`, `SetName` (window/tab/pane/workspace) |
| Layout / navigation | `Layout` (query=layout\|windows\|workspaces\|tabs), `SetActiveTab`, `NewTab`, `FocusWindow` |
| Scheduling | `Loop`, `LoopStop`, `LoopList`, `CronCreate`, `CronDelete`, `CronList`, `CronPause`, `CronResume` |
| Memory (brain) | `MemoryList`, `MemoryRead`, `MemoryWrite` |
| Presets | `PresetList`, `PresetGet` |
| Identity | `IdentityAccounts`, `IdentityValidate` |

Notably **absent**: anything about the app's own runtime health — no process info, no renderer/window diagnostics, no reload/restart, no saga/queue health. There is no `system` category at all.

### 3.2 REST verbs (`agentmux-srv/src/server/mod.rs`, curated `/api/v1/*` surface)

`shell/{create,stop,input,status}`, `pane/open`, `voice/transcribe`, `self`, `window/name`, `tab/name`, `pane/title`, `workspace/name`, `layout`, `windows`, `workspaces`, `tabs`, `tab/activate`, `tab/new`, `window/focus`, `agent/memory/{list,read,write}`, `agent/preset/{list,get}`, `agent/identity/{accounts,validate}`.

### 3.3 The raw gateway (`/agentmux/service`, `dispatch_service`) — bigger than documented, still growing

The 2026-06-17 spec counted **47 methods**; re-verified today the four service modules alone contain (arm count from the reducer's `match` blocks):

| Service | Methods |
|---|---|
| `object` | `GetObject`, `GetObjects`, `UpdateTabName`, `CreateBlock`, `DeleteBlock`, `UpdateObjectMeta`, `UpdateObject` |
| `client` | `GetClientData`, `GetTab`, `FocusWindow`, `AgreeTos`, `GetAllConnStatus`, `TelemetryUpdate` |
| `window` | `GetWindow`, `CreateWindow`, `CloseWindow`, `SwitchWorkspace`, `SetWindowPosAndSize` |
| `workspace` | `CreateWorkspace`, `GetWorkspace`, `DeleteWorkspace`, `ListWorkspaces`, `CreateTab`, `SetActiveTab`, `CloseTab`, `UpdateWorkspace`, `UpdateTabIds`, `MoveBlockToTab`, `PromoteBlockToTab`, `ReorderTab`, `MoveTabToWorkspace`, `RestoreTornOffTab`, `TearOffBlock`, `RedockFloatingPane`, `TearOffTab` |
| `misc` (userinput/block/subagent/history/agent) | `SendUserInputResponse`, `GetControllerStatus`, `SendCommand`, `SaveTerminalState`, `ListActive`, `GetHistory`, + more |

None of `window`/`workspace`'s more powerful verbs (`CloseWindow`, `DeleteWorkspace`, `SetWindowPosAndSize`) are MCP-wrapped — consistent with the Tier-1 gating the first-class-surface spec calls for, but today they're simply **unwrapped**, not gated; nothing stops a raw `/agentmux/service` POST from calling them (verified during tonight's session: `DeleteBlock` was called directly over HTTP with only the shared auth key, no capability check).

### 3.4 Diagnostics that exist but are **not** reachable by an agent at all

- **`GET /agentmux/diag/sagas`** — durable saga log + in-flight count (`mod.rs`, "operator visibility into the durable saga log"). No MCP tool, no `/api/v1` alias. HTTP-reachable with the auth key, undocumented for agent use.
- **`agentmux-launcher --diag {sagas,wrr,srv}`** — CLI-only, pipe-IPC, not reachable over HTTP/MCP at all (would require shelling out to the launcher binary with the right working directory).
- **GPU status indicator** (PR #1337, `frontend/app/statusbar/GpuStatus.tsx`) — frontend-only, reads host state directly; no backend RPC surfaces it.
- **`sysinfo` pane/widget** — same: a frontend view, not backed by an agent-queryable RPC.
- **Renderer/subprocess process info** — CEF's own `--type=renderer|gpu-process|utility` subprocess tagging is read once at each child's own startup (`agentmux-cef/src/lib.rs:304-307`) purely to decide "am I a subprocess," and is **never aggregated or exposed** anywhere — not to the frontend, not over RPC. Tonight's renderer-count check had to shell out externally (`Get-CimInstance Win32_Process -Filter "Name='agentmux-0.50.0.exe'"` + regex the command line for `--type=`) because nothing in-process tracks this.
- **Window reload** — grepped the entire repo (`agentmux-cef`, `frontend`, `docs`): there is no "reload the window/view" command anywhere — no RPC, no documented keybinding, no host IPC command. `browser_api/types.rs` has a `reload` **for browser-pane content** (i.e. reload the web page inside a `browser` view), which is a completely different thing from reloading the app's own chrome/frontend. Toggling DevTools (`dev:devtools`, native-menu-only, `agentmux-cef/src/macos_menu.rs`) is the closest adjacent capability and is also not RPC-reachable.

---

## 4. Proposed `system` domain

Following §4.1's layering and §5's tiering from the first-class-surface spec.

### 4.1 Tier 0 (default, ungated — read-only or self-scoped-and-reversible)

| MCP tool | REST | Backing |
|---|---|---|
| `SystemProcessInfo()` | `GET /api/v1/system/processes` | **New.** Host-side: enumerate own child processes by CEF `--type=` (browser/renderer/gpu-process/utility), return counts + PIDs. Removes the need for the external `tasklist`/WMI shellout used tonight; directly answers the exact check `SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03.md` §4 item 4 calls for ("check via `tasklist`... before/after close"). |
| `SystemSagaHealth()` | alias existing `GET /agentmux/diag/sagas` under `/api/v1/system/sagas` | Already implemented; just needs the REST alias + MCP wrap. |
| `SystemReloadView(block_id?)` | `POST /api/v1/system/reload-view` | **New.** The literal capability missing tonight: reload just the calling agent's own window/frontend (or an explicit `block_id`'s owning window, defaulting to self via `WhoAmI`-style resolution). Implemented as a host IPC command that calls the CEF browser's own `Reload()`/`ReloadIgnoreCache()` on the frontend's root browser — not the same code path as a browser-*pane*'s content reload (`browser_api::reload`), which only affects an embedded web page, not the app chrome. Self-scoped and reversible (worst case: momentary blank window while it repaints) — Tier 0. |
| `SystemGpuStatus()` | `GET /api/v1/system/gpu` | **New**, thin wrap of whatever `GpuStatus.tsx` already reads host-side — surfaces enabled/disabled + driver info to agents, not just the statusbar. |

### 4.2 Tier 1 (gated — destructive or global; default off, per existing convention)

| MCP tool | REST | Backing |
|---|---|---|
| `SystemRestartInstance()` | `POST /api/v1/system/restart` | **New.** Full app restart (not just a view reload) — closer to what would have been needed if a hard-refresh hadn't sufficed tonight. Genuinely destructive to in-flight state; gate behind the same capability flag §5 of the first-class-surface spec proposes for `CloseWindow`/`DeleteWorkspace`. |
| `SystemToggleDevTools()` | `POST /api/v1/system/devtools` | Wraps the existing native-menu-only `dev:devtools` command. Not destructive, but exposes live JS console access to whatever's rendered — treat as Tier 1 out of caution (it's a debugging escape hatch, not a normal agent verb) unless there's a concrete agent use case for it. |

### 4.3 Deliberately excluded / needs its own investigation first

- **Root-causing *why* a pane gets pruned from a layout tree** (tonight's actual missing-panes incident) is a correctness bug hunt, not an API gap — out of scope here. This spec only closes the "I had no way to check/recover" gap around it.
- **Cross-instance system management** (reload/restart *another* AgentMux instance) — stays out per the existing non-goal ("an agent only acts on its own instance").
- **Killing arbitrary child processes by PID** — never; would conflict with the I1–I6 isolation invariants (`CLAUDE.md`) and the "no `taskkill //im`, no killing processes you didn't spawn" rule. `SystemRestartInstance` must go through the launcher's own lifecycle (Job Object), never a raw `TerminateProcess`.

---

## 5. Why this matters beyond tonight

The renderer-leak fix (PR #1957) shipped with one item explicitly flagged as **not independently re-confirmed**: renderer-process-count returning to baseline after repeated pane closes, because "the test instance was terminated externally before a clean measurement could be taken." `SystemProcessInfo()` turns that from a manual `Get-CimInstance` shellout (what this session had to improvise, against its own live host window, which is why the earlier verification attempt tonight was paused and redirected to an isolated instance instead) into a one-line, always-available check — for this fix's own future regressions and for any future CEF-process-lifecycle work.

---

## 6. Implementation plan

**Phase 1 — the two verbs tonight actually needed**
1. `SystemProcessInfo` (read-only, no host changes needed beyond enumerating own children — Windows via `Win32_Process`/`EnumProcesses` filtered to child PIDs already tracked by the Job Object; macOS/Linux via `/proc` or `sysctl` equivalents, scoped to own process tree only).
2. `SystemReloadView` (host IPC command + REST + MCP wrap).

**Phase 2 — diagnostics parity**
3. `SystemSagaHealth` (thin alias of existing endpoint).
4. `SystemGpuStatus` (thin wrap of existing host-side GPU state).

**Phase 3 — gated recovery**
5. Capability-tier plumbing (shared with the still-open Tier-1 gating from the first-class-surface spec — do this once, for both specs' Tier-1 verbs together, not twice).
6. `SystemRestartInstance`, `SystemToggleDevTools` behind that gate.

Each phase ships independently; Phase 1 alone would have fully covered tonight's incident (diagnose via process info, recover via reload, without falling back to raw HTTP + PowerShell shellouts against a live user session).

---

## 7. Open questions

1. **Process enumeration scope on Windows** — Job Object already tracks every child PID (I2/I3 invariants); should `SystemProcessInfo` read from the Job Object's own accounting (`QueryInformationJobObject`) instead of a fresh `Win32_Process` scan? Would be more consistent with "bounded blast radius" but needs checking whether renderer/gpu/utility subprocess PIDs are actually assigned into the same Job Object as the host.
2. **`SystemReloadView` scope when torn off** — same open question §8.2 of the first-class-surface spec raises for `SetWindowName`: does self-context resolve the right *window* (not just block) when the calling agent's pane was torn off into a floating window?
3. **Does reload actually fix a stale empty layout cell**, or would that require `SystemRestartInstance`? Untested as of this writing — the incident that motivated this spec was redirected to a hard-refresh-by-hand request rather than confirmed via either new verb (neither exists yet).
4. **Capability storage** — same unresolved question as the first-class-surface spec §8.1; should be answered once, shared across both specs' Tier-1 verbs.
