# ANALYSIS: Agent App API — open file / folder in editor pane

**Date:** 2026-05-30
**Status:** Analysis (not a spec — exploring design space)
**Owner:** Frontend + sidecar (RPC) + agent-pane shell integration
**Immediate use case:** an agent running inside an AgentMux pane wants to say *"open `/path/to/foo.ts`"* and have AgentMux open it in an editor pane, **reusing an existing editor pane if one is open**.

---

## 1. Why analysis, not spec

The 30-character ask ("open a file in the editor pane, reuse if one exists") sits on top of three layered design problems that each have multiple valid answers. A spec written before these are debated would either pick wrong or over-specify. The decisions:

1. **Pane-reuse scope.** "If a pane exists" — *where*? Same tab, any tab, focused window?
2. **Transport — how does the agent actually reach the app.** RPC over the existing sidecar socket? A terminal command (`amux open foo.ts`)? An MCP server? A clipboard / OSC-sequence convention?
3. **Path translation.** An agent running in a container says `/workspace/foo.ts`. The host editor pane can't open that. Who translates, and how?

Plus a broader framing question: this is the *first* shipped agent → app callback. The shape we pick becomes the template for the next ten (`open in browser`, `show diff`, `attach screenshot`, `jump to PR`, `start new tab with…`, …). Worth getting the spine right before any code lands.

---

## 2. Current state (what we already have)

| Capability | Status | Reference |
|---|---|---|
| `pane.open` RPC — high-level intent: open a pane with `view`, `file`, `cwd`, `title`, `tab_id`, `split_direction`, `focus` | ✅ implemented | `COMMAND_PANE_OPEN` const at `agentmux-srv/src/backend/rpc_types.rs:407`, `CommandPaneOpenData` struct at `:809`, handler in `agentmux-srv/src/server/app_api.rs` |
| Editor pane backed by multi-tab state (`tabs[]`, `activeTabId`, `recentlyClosed[]`) | ✅ shipped | `frontend/app/store/editor-pane-state-store.ts:64-69` |
| `createblock` low-level RPC (returns `ORef`) | ✅ shipped | `frontend/app/store/rpc-api.ts:105` |
| `open_agent` callback (CEF → frontend via `CustomEvent`) — proves the agent→app pattern works | ✅ shipped | `agentmux-cef/src/commands/palette.rs:60` |
| `WshRpcEngine` handler registration framework on the sidecar | ✅ shipped | `agentmux-srv/src/backend/rpc/engine.rs:189` |
| Terminal CLI for agent → app calls (anything an agent could invoke from `bash` to reach the app) | ❌ **never existed** | The `wsh*` files in `frontend/app/store/` are a **frontend** RPC client layer renamed `rpc-*` per `docs/specs/SPEC_RENAME_WSH_TO_RPC_2026_04_17.md`; no terminal-side binary has ever shipped. |
| MCP server inside AgentMux | ❌ none |   |
| Path translation (container → host, WSL → host, SSH → host) | ❌ none |   |

**Net read of the audit:** the *app* end of the agent→app loop is mostly built. The *agent* end is empty — there is no current way for a process running in an AgentMux pane to make an RPC call. That gap is the bottleneck, not the API itself.

What `pane.open` does today that's relevant:
- `view: "editor"` + `file: "/abs/path"` creates a **new** editor pane on that file.
- `tab_id` optional — defaults to the active tab.
- `split_direction` + `split_reference_block_id` — can place the new pane relative to an existing block.

What `pane.open` does **not** do today:
- Detect an existing editor pane in the target tab and reuse it.
- Open a folder (only file).
- Translate paths across connections.

---

## 3. What "agent app API" should be (broader framing)

Before we lock in transport for one verb, decide the API contract category. Three options form the broader framing — every future agent→app callback (open URL, show diff, start agent, …) will land in one of them.

### 3.1 Option A — RPC over the existing sidecar socket

The agent calls a method on the same `WshRpcEngine` the host already uses. Agents in **local** terminal panes inherit the connection; agents in **container/SSH/WSL** sessions need a tunnel.

- **Pro:** zero new server surface; type-safe RPC types already exist; auth handled by socket identity.
- **Con:** the agent process has to *find* the socket. For local agents that's a known env var; for container agents the socket lives on the host. Tunnel via stdin/stdout-multiplexed control channel, or have the sidecar listen on a tcp port reachable via host.docker.internal.
- **Maps to:** v1 of every agent→app callback we'll ever want.

### 3.2 Option B — Terminal CLI (`amux <verb> ...`)

A small Rust binary (call it `amux` — `wsh` was retired but the shape was right) lives on `$PATH` inside every pane and forwards verbs to the sidecar.

```
$ amux open ./src/foo.ts
$ amux open ./src/                     # folder
$ amux open --new-pane ./src/foo.ts    # opt out of reuse
$ amux browse https://docs.example
$ amux say "agent finished phase 2"
```

- **Pro:** familiar UNIX-y surface (matches `code`, `xdg-open`, `open`); discoverable by `--help`; works from shell scripts as well as from agents.
- **Con:** a new binary to bundle, install into containers (via shell-integration injection?), keep version-compatible with the sidecar protocol. And it widens the install surface (one more thing to package, sign, ship).
- **Maps to:** every agent→app callback, with a low-friction surface that agents (and shell scripts, and curious users) can use.

### 3.3 Option C — MCP server inside AgentMux

Expose a Model Context Protocol server from the sidecar; agents that speak MCP (Claude Code, Cursor, Cline, …) discover and call it as a tool.

- **Pro:** the same agents already speak MCP; no new transport convention; tool-call protocol handles auth + path serialization. Tooling story for AI-native agents is unified.
- **Con:** doesn't help non-MCP agents (`bash` scripts, raw `claude` sessions without MCP wiring, future agent runtimes). And running an MCP server *inside* an MCP-host app is conceptually weird — it's a tool-call directed back at the same app that initiated the conversation.
- **Maps to:** AI agent → app callbacks specifically.

### 3.4 Recommendation: B (with A under the hood)

- **B (`amux` CLI)** is the *user-visible* surface — what an agent invokes, what a shell script invokes, what an internal smoke test invokes. The verb set is the API.
- **A (RPC)** is the *transport* the CLI uses. The CLI is a thin wrapper over the existing WshRpcEngine; one `amux open` call is one `pane.open` RPC.
- **C (MCP)** can be added later as a thin façade over the same verb set if/when we want AI agents to discover the surface as MCP tools. The verb set must already be solid for this to make sense.

Why not just A directly: agents (especially shell-based ones) need a syscall-shaped surface. A "drop down to RPC" requirement is the bar that kills agent uptake of agent→app callbacks across the board. The CLI lets a `bash` agent `amux open report.csv` and move on.

Why not C alone: leaves out everyone who isn't an MCP-speaking AI client.

Why not call it `wsh`: that name belonged to a *frontend* RPC client layer (a legacy fork-naming holdover from WaveTerm, renamed `rpc-*` per `SPEC_RENAME_WSH_TO_RPC_2026_04_17.md`). It was never a terminal-side CLI — and reusing the name would just relitigate the rename for no benefit. `amux` starts clean and matches the product name.

---

## 4. The immediate use case: `amux open` semantics

Concrete spec sketch for the *first* verb. Other verbs will follow this shape.

### 4.1 Surface

```
amux open <path>                       # file or folder
amux open --new-pane <path>            # force a new pane
amux open --tab <name|id> <path>       # open in a specific tab
amux open --split right <path>         # force a new pane, split right
amux open --read-only <path>           # open in editor pane in read-only mode
amux open --line 42 <path>             # open at line 42 (file only)
```

### 4.2 Default (no flags) decision tree

```
1. Is `<path>` a file?
   yes → 1a. Is there an editor pane in the current tab?
            yes → open as a new tab inside that pane (reuse) ✔
            no  → create new editor pane in the current tab
   no (folder) → 2. Is there an editor pane in the current tab?
                    yes → focus it, set its workspace root to the folder
                    no  → create new editor pane with the folder as the root
```

The current-tab default is the safer choice — see §5 for why "any tab" is a trap.

### 4.3 Pane-reuse: where does "exists" look?

Four candidate scopes, in order of widening:

| Scope | "Exists" means | Pro | Con |
|---|---|---|---|
| **A. Current tab only** *(recommended)* | An editor pane in the tab the agent is running in | Predictable; matches VS Code's "current editor group"; agent is implicitly scoped to its own tab | A user with editor panes in another tab will be surprised when a fresh one is created in *their* tab |
| **B. Focused window, any tab** | Any editor pane in the focused window | Spans across the user's working set | Pulls focus to a different tab silently — disorienting |
| **C. Any tab anywhere** | Any editor pane in any tab in any window | "Truly reuse if one exists" | Drag-stealing across windows is hostile; user loses control of which window they're in |
| **D. Caller-tab + caller-window-fallback** | Caller's tab first, then any in the caller's window | Hybrid | More rules to explain; same "fresh pane sometimes" surprise as A in cross-window cases |

**Pick A** because:
- It's analogous to VS Code's `code` CLI default (open in current editor group).
- It maps cleanly onto the existing `tab_id` plumbing in `pane.open` — caller's tab is the default already.
- Future users can override with `--tab <name>` (B/C explicit, not by default).

### 4.4 Folder semantics

Folders need a workspace root, not a `file:` meta. Two possible shapes:

- **Shape 1**: extend the editor view-model to track a `workspaceRoot` meta key separate from `file`. Folder drop sets the root + opens the file tree; file drop opens the file as a tab. **Recommended** — preserves the multi-tab model.
- **Shape 2**: synthesise a `pane.open` call per child file. Doesn't compose: folders with 200 files would explode the tab list.

Shape 1 needs a new meta key (`editor:workspaceRoot`) and view-model wiring. The editor pane's left-rail file-tree-on-root already exists per the audit; this is wiring not invention.

### 4.5 What `pane.open` needs to grow

Two new fields on `CommandPaneOpenData`, both optional:

- `reuse_strategy: "current-tab" | "none"` (default `"current-tab"` when called from `amux`; `"none"` for back-compat with existing callers — they currently always create new).
- `editor_workspace_root: string?` for folder semantics.

The handler-side logic is small: enumerate blocks in the resolved tab, pick the first one with `view == "editor"`, splice an `EditorTab` into its `editor:tabs` state.

---

## 5. Path translation — the load-bearing unsolved problem

If the agent is in a Docker container and runs `amux open report.csv`, the absolute path `/workspace/report.csv` is the *container's* view. The editor pane on the host has no concept of `/workspace/` and will fail to read the file.

This is **not** a problem the existing `pane.open` knows about. Every connection has its own filesystem namespace. The `cmd:cwd` meta is a string from the *agent's* world, but the editor pane needs a path from the *host's* world (or, alternatively, the editor pane needs to know how to read files through a connection).

Three approaches:

### 5.1 Translate at the boundary (push-to-host)

The `amux` CLI inside the container tar-streams the file's bytes to the sidecar; sidecar materialises into a host-side scratch directory; opens that copy.

- **Pro:** editor pane stays simple; same code as the file-drop spec (#1201).
- **Con:** the user is editing a *copy*; saves don't write back into the container. The single most painful UX of every "remote files" tool that does this.

### 5.2 Translate at the boundary (mount-aware path map)

Sidecar maintains a per-connection path map (`{container_path: host_path}`) derived from the container's bind mounts. `amux open` looks up the translation; the editor pane opens the *host* path; saves go to the host file which is the bind mount.

- **Pro:** zero-copy; saves work.
- **Con:** requires the connection to be set up with bind-mount visibility. Doesn't work for containers running on a remote Docker host.

### 5.3 Make the editor pane connection-aware

The editor pane gains a `connection:` meta. It reads/writes files via a sidecar-mediated transport (sftp, `docker cp`, `wsl <cmd>`) keyed on that connection — the same backends carved out for the file-drop Phase 2.

- **Pro:** Strictly more general. Works for containers, SSH, WSL, future runtimes. Aligns with the file-drop Phase 2 transport work — same backends serve both.
- **Con:** much larger surface (editor save flows have to learn the transport; latency tradeoffs for remote autosave; conflict resolution if the file changes under us).

### 5.4 Recommendation

Ship Phase 1 with **5.1 (push-to-host)** for non-local connections, **5.2 (path map)** for the easy case (local container with known bind mount = the workspace dir). 5.3 is the right long-term answer but is its own multi-week spec — call it out, don't block on it.

**Critically**: for the very first `amux open` PR, scope to **local connections only.** Container/SSH/WSL surfaces a "this connection doesn't support `amux open` yet" toast, the same way file-drop Phase 1 leaves remote transport for Phase 2. This keeps the first slice shippable in a small PR and proves the verb shape works before we eat the path-translation complexity.

---

## 6. Transport: how does `amux` reach the sidecar?

The CLI needs to find and call the sidecar's WshRpcEngine. Today the sidecar listens on a unix socket / named pipe; the path is in an env var that the host sets when it spawns the agent.

For **local terminal panes**: the env var passes naturally; `amux` reads it and dials. Simple.

For **container / SSH / WSL panes**: the env var inside the agent's environment points to a path that doesn't exist on the *agent's* host. Options:

- **Stdin/stdout multiplexing**: `amux` writes a framed RPC request to `stdout` with a magic prefix the terminal pane intercepts (OSC-style). The pane forwards to sidecar; reads back the response. **Compatible with shells; works through any pty.**
- **TCP via `host.docker.internal`**: sidecar listens on a tcp port; container reaches it via the docker host name. Simple but: requires open port, needs auth (currently socket-identity-based, would need a token), doesn't work for SSH/WSL the same way.
- **Reverse-mount the socket**: bind-mount the sidecar's socket into the container at a known path. Works if AgentMux owns container creation; doesn't if the user attaches to a pre-existing container.

For Phase 1 (local only), the simple env-var dial is enough. For Phase 2 the **OSC-multiplexed stdio channel** is the most universal — works for any pty-backed connection, no port management, no socket bind-mounting.

---

## 7. Path forward (concrete recommendations)

Ordered phases — each shippable independently.

### Phase 1 (small, ~2-3 days)
- Extend `pane.open` with `reuse_strategy: "current-tab" | "none"` (default `"none"` for back-compat) and the editor-pane reuse logic.
- Extend `pane.open` with `editor_workspace_root` for folders, wire to editor view-model.
- Create the `amux` CLI: single verb `open`, flags `--new-pane`, `--tab`, `--split`, `--read-only`, `--line`. Reads sidecar socket from env var; calls `pane.open`.
- **Local connections only.** Container/SSH/WSL drops a "not supported on this connection" toast.
- Spec it; PR it.

### Phase 2 (~1 week)
- OSC-multiplexed stdio channel so `amux` works inside containers.
- Path translation (5.1 + 5.2) for the open-from-container case.
- Second verb: `amux browse <url>` (open in browser pane).

### Phase 3 (deferred until we see usage)
- Editor pane connection-awareness (5.3) — full remote-file save support.
- MCP façade so AI agents discover the verb set as MCP tools (3.3).
- More verbs as demand surfaces (`amux say`, `amux notify`, `amux diff`, etc.).

### Phase 4 (much later, only if scope grows)
- Permission model: today every agent that can call `amux` can call every verb. If we ever support untrusted agents, gate verbs by capability.

---

## 8. Open questions

- ~~**Is `amux` the right binary name?**~~ **Resolved 2026-05-30:** `amux`. Short enough to type, distinct enough from `tmux` once parsed, namespaced under the product name.
- **Should `pane.open` reuse default to `"current-tab"` for ALL callers, not just `amux`?** Probably yes after a short deprecation window — current "always create new" is a foot-gun.
- **What's the right behavior for `amux open foo.ts` when the editor pane is already showing `bar.ts` with unsaved changes?** Phase 1: open the new file as a *second tab*; don't disturb the dirty buffer. (Editor multi-tab supports this already.)
- **Should the agent's identity (which agent / which pane) be on the RPC?** For Phase 1, no — implicit from caller socket. Eventually yes, for audit and for verb-level capability gates.
- **Does `amux open` block until the file is open, or fire-and-forget?** Fire-and-forget with an optional `--wait` flag. Agents shouldn't sit waiting on UI.
- **What's the relationship to the file-drop work (#1201)?** Same transport backends (Phase 2 of both), same de-collision rules. Phase 1 file-drop already ships the host-local copy path that `amux open` Phase 2 will reuse. The two specs should cross-reference.

---

## 9. What we are NOT analysing here

- Pane *closing* / focusing other than as a side-effect of `pane.open`. Comes later.
- The full verb surface beyond `open`. Listed in §7 only as plumbing for the design decisions.
- Permission / capability model. §7 Phase 4.
- An MCP server. §3.3 / §7 Phase 3.
