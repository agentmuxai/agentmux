# REPORT — What Wave Terminal has that AgentMux doesn't, and what is worth bringing over

**Date:** 2026-10-02
**Status:** analysis — a comparison and recommendations; the remote terminals item is planned in `docs/specs/SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md`.
**Author:** AgentX (narko)
**Trigger:** Owner request: "take a look at https://docs.waveterm.dev/ .. we want to get some of the more innovative features into agentmux, like secrets. can you get the best stuff that isnt currently inside agentmux."
**Method:** Read the Wave docs (home, secrets, durable sessions, connections, Wave AI, AI modes / BYOK, Claude Code integration, wsh reference, widgets, custom widgets, tabs, workspaces, keybindings, release notes v0.12.3 to v0.14.5). Checked each feature against AgentMux's code on `main` at `9d206f9d7` (srv, cef, mcp, frontend, specs). AgentMux descends from Wave, so several Wave-era leftovers exist in config types or services with nothing wired to them; those are called out as such, not as features.

## 1. Summary

| # | Wave feature | AgentMux today | Worth it? | Effort |
|---|---|---|---|---|
| 1 | **Secrets store** (named, OS keychain, UI + CLI, referenced by name from config) | Keychain backend exists, used only for accounts and tokens; no named secrets, no references | **Yes, first** | M |
| 2 | **Badges** (icon + colour on block and tab, priority rollup, auto-clear on focus, CLI, bell, Claude Code hooks) | Tab flash, taskbar attention, OS toasts; no badge model | **Yes** | M |
| 3 | **Process viewer** (CPU/mem per process) | Sysinfo graphs only; per-process data exists in the process tracker, unused in UI | **Yes** | M |
| 4 | **Quake-mode global hotkey** | `app:globalhotkey` field defined, never read | **Yes** (cheap) | S |
| 5 | **Drop a file on a terminal to paste its path**, image paste, OSC 52 clipboard | Drop copies the file into the cwd; text-only paste; no OSC 52 | **Yes** (cheap, frequent) | S |
| 6 | **Bring-your-own-key AI modes** (OpenAI-compatible, Ollama, LM Studio, Gemini, OpenRouter; key from the secret store) | Fixed provider list; per-account keys; base-URL env override | **Partly**: for ambient calls first | M |
| 7 | **Scripting CLI parity** (`wsh getmeta/setmeta`, `termscrollback`, `getvar/setvar`, `notify`, `blocks list`) | `muxsh` covers open/edit/view/web/run/agent; MCP for agents | **Yes**, the gaps | S–M |
| 8 | **Workspaces** with switcher, icon and colour | Backend services left from Wave, no UI | Maybe | M |
| 9 | **Vertical tab bar** | None | Maybe | M |
| 10 | Cursor style, focus-follows-cursor | Only cursor blink | Small win | S |
| 11 | **SSH/WSL connections** with remote helper install, per-connection settings, remote file browsing | Constants and config fields left from Wave; no connection manager | Big, strategic: decide first | L |
| 12 | **Durable SSH sessions** (remote job manager survives drops and restarts) | None | Only after 11 | L |

Already present and not repeated here: custom `widgets.json` (AgentMux has it, with submenus and modules), the sysinfo graph widget, tab background rendering (no UI to set it), and a Wave AI–style chat (AgentMux's agent panes go well beyond it). Pinned tabs were removed on purpose and are not recommended back.

**Recommended order:** 1 Secrets, 2 Badges, 4 + 5 (small, same PR window), 3 Process viewer, 7 CLI gaps, 6 BYOK for ambient calls. Then decide on 11 before anything in 12.

## 2. The features

### 2.1 Secrets store — first

**What Wave does.** Named secrets (`^[A-Za-z][A-Za-z0-9_]*$`) stored in the OS keychain: Keychain on macOS, Credential Manager on Windows, Secret Service on Linux (no compatible backend means no secrets). A Secrets page in settings; `wsh secret list|get|set|delete|ui`. Config refers to a secret **by name** and never holds the value: `ssh:passwordsecretname` for SSH passwords, `ai:apitokensecretname` for AI keys (provider presets default to names such as `OPENAI_KEY`). "Secrets are never stored unencrypted in logs or accessible files."

**What AgentMux has.** The hard part already exists. `crates/srv/src/identity/secret_store.rs` wraps the `keyring` crate across all three platforms, with a locked-down file fallback for headless machines, and already holds Armory account API keys, OAuth tokens, the Claude OAuth token, MuxBus credentials and browser-pane logins. `SecretRef` (`backend/storage/identities.rs`) can point at the keychain, an env var or a secrets manager, but only identity accounts use it. There are no user-named secrets, no reference syntax in config, and `muxsh` deliberately left `secret list` out.

**What to build.**
- **Named secrets** in the existing store under their own namespace, with Wave's naming rule.
- **An Armory "Secrets" tab:** list names (never values), add, replace, delete. Values are write-only in the UI, with an explicit reveal behind a confirmation.
- **References, not values, in config:** `cmd:env` values written as `secret:GITHUB_TOKEN`, resolved at spawn. That covers agent definitions' env, MCP server env, widget `cmd:env`, and terminal blocks. Resolved values go into the child's environment only, never into block meta, logs or the frontend.
- **`muxsh secret list|set|delete`** for people, and an optional `get` that refuses inside an agent's process unless the user enabled it.

**Why first, and the AgentMux-specific risk.** It removes plaintext tokens from env files and agent definitions, and it is mostly wiring around a store that already works. The risk Wave does not have: AgentMux runs agents that can call tools and read their own environment. Design rules:
- No MCP tool returns a secret's value. Agents get secrets only by injection into their own process, by name, chosen by the user in the agent's definition.
- A jekt asking for a secret is already forced to `TIER=sensitive` by the credential keyword rule. That stays.
- Redact known secret values from transcripts and logs. A spawned process could still print one; AgentMux can scrub what it stores and shows.

### 2.2 Badges — second

**What Wave does.** A badge is an icon (Font Awesome name) and a colour on a block, with a priority. The tab shows the highest-priority badge among its blocks. A badge clears when you focus the block: "purely a 'hey, look over here' signal". Set it with `wsh badge`; a terminal bell sets a bell badge by default. The Claude Code integration installs hooks in `~/.claude/settings.json` that set badges for "permission needed" (gold, 20), "question" (gold, 20) and "done" (green, 10).

**What AgentMux has.** Agent panes know these states natively and show them in the pane, and in the Swarm chip (Korp's question-chip work, PR #4242, adds a question state). Tabs flash with agent tool sounds; there is OS taskbar attention and toasts. What is missing is the general mechanism: a small, prioritised, auto-clearing marker that **any** pane can raise, rolled up to its tab.

**What to build.**
- Block meta `badge` (`icon`, `color`, `priority`, `source`) with tab rollup and clear-on-focus.
- Agent panes set it from states they already have: question or permission pending (high), turn finished while unfocused (low). No hooks needed for our own agent panes.
- A terminal bell sets a low-priority badge (xterm's `onBell`; AgentMux has no bell handler today).
- `muxsh badge set|clear` for scripts: `make test; muxsh badge set check --color green` tells you a long build is done in a background tab.
- For a plain terminal running the `claude` CLI outside an agent pane, ship the same three hooks Wave documents, pointing at `muxsh badge`.

### 2.3 Process viewer — third

**What Wave does.** A Process Viewer widget (v0.14.5): processes with CPU and memory, for local and remote machines, with its own keybindings.

**What AgentMux has.** The sysinfo graph widget. The backend already attributes commit memory per process (`backend/sysinfo.rs`), and the process tracker knows each agent pane's process tree, but nothing shows either.

**What to build.** A process pane grouped **by agent pane**: each agent, its CLI, and everything it spawned (dev servers, test runners, MCP servers), with CPU, memory and age, plus "kill tree" (the `agent.kill-tree` command exists). This is more useful to AgentMux than Wave's flat list, because "which agent left a dev server running" is a real question here. It also gives the Swarm a data source for runaway processes.

### 2.4 Quake-mode global hotkey

**What Wave does.** A system-wide hotkey that shows and hides the window (v0.14.5), configurable.

**What AgentMux has.** `app:globalhotkey` in `wconfig/types.rs`, read by nothing.

**What to build.** Register the hotkey in the CEF host (Win32 `RegisterHotKey`, the macOS and Linux equivalents) to toggle the last-focused window. Small, and the setting already exists.

### 2.5 Drag-and-drop paths, image paste, OSC 52

**What Wave does.** Dropping a file on a terminal inserts its path (v0.14.5); images paste into terminals (v0.12.3); OSC 52 lets a program in the terminal (tmux, vim, an SSH session) set the clipboard (v0.14.0).

**What AgentMux has.** Dropping files on a terminal **copies them into its working folder** (`term.tsx`, `drag/file-drop-actions.ts`). Paste is text-only. `termwrap.ts` registers OSC 0, 2, 7, 9283 and 16162, not 52.

**What to build.**
- A modifier on drop (Alt, or a drop-zone choice) that inserts the shell-quoted path(s) instead of copying. Agents in terminal panes take paths far more often than copies.
- An OSC 52 handler: write-only (set clipboard), never read, with a size cap.
- Image paste into a terminal pane: save to a temp file and paste its path. That is what CLI agents in a terminal can actually use.

### 2.6 Bring-your-own-key AI modes

**What Wave does.** `waveai.json` defines modes with a provider preset (`openai`, `openrouter`, `groq`, `google`, `azure`, `custom`; Ollama, LM Studio and vLLM through OpenAI-compatible endpoints), the key by secret name (`ai:apitokensecretname`), and capabilities (`tools`, `images`, `pdfs`).

**What AgentMux has.** A fixed provider list (`backend/providers.rs`), per-account keys in the keychain, and `base_url_env_var` so Claude can point at a proxy or OpenRouter.

**What to build, scoped.** The place this pays first is **ambient calls** (titles, summaries, ghost text, naming), which today spawn a CLI: let them use a user-defined OpenAI-compatible endpoint (a local Ollama model, say), with the key from 2.1. That cuts cost and latency for the calls that run most often. A general "custom provider" for agent panes is larger and can follow.

### 2.7 Scripting CLI gaps

**What Wave has, that `muxsh` lacks.** `getmeta` / `setmeta` (read and write a block's settings from inside it), `termscrollback` (dump a terminal's history to a file), `getvar` / `setvar` (per-block variables), `notify` (an OS notification from a script), `blocks list`, and `conn ensure`.

**What to build.** `muxsh meta get|set`, `muxsh scrollback`, `muxsh notify`, `muxsh badge` (2.2). Each is a thin wrapper over an existing RPC. `getvar`/`setvar` overlaps with block meta and is not needed separately.

### 2.8 Workspaces

**What Wave does.** Named, saved sets of tabs with an icon and colour, a switcher left of the tab bar, and per-workspace layout and history.

**What AgentMux has.** `CreateWorkspace`, `ListWorkspaces` and `SwitchWorkspace` services left from Wave; no UI calls them. AgentMux's windows and channels already partition work in their own way.

**Worth it?** Only if the owner wants "project" separation inside one window. The cost is the UI and the persistence rules (Wave's unsaved-workspace semantics are subtle). Not before 2.1 to 2.5.

### 2.9 Vertical tab bar, cursor style, focus-follows-cursor

Vertical tabs (Wave v0.14.4) are a real gain for people with many agents per window. Cursor style and focus-follows-cursor (v0.14.1) are small settings over xterm options. Low priority, low risk.

### 2.10 SSH/WSL connections and durable sessions — decide before building

**What Wave does.** A terminal block can belong to a connection (`user@host`, `wsl://<distro>`). Wave parses `~/.ssh/config` (not `Match`), supports `ProxyJump`, installs its helper (`wsh`) on the remote host for file browsing and widgets, and has per-connection settings (`term:theme`, `cmd:env`, init scripts, `ssh:passwordsecretname`). **Durable sessions** (v0.14.0) put a small job manager on the remote host, so shells survive network drops, sleep and app restarts, and reattach with buffered output; status shows on a shield icon. Keepalives and stalled-connection detection come with it.

**What AgentMux has.** Wave-era constants (`CONN_TYPE_SSH`, `CONN_TYPE_WSL`), unused `ssh:*` config fields, a `GetAllConnStatus` that returns empty ("connection manager not yet wired"), and a connection typeahead calling a WSL command with no handler. `SPEC_RETIRE_WSH_2026_04_12.md` records "AgentMux has no SSH/WSL pane story."

**Recommendation.** This is the largest item by far: a connection manager, a remote helper per platform, remote file operations and then durable sessions. AgentMux's current answer to "work on another machine" is to run AgentMux there and see it over LAN and WAN (the multi-host Swarm). Decide whether remote terminals belong in the product before spending on it. If they do, **WSL first** (no network, no remote install), then SSH without durability, then durable sessions. Either way, remove the dead Wave connection code so it stops implying a feature that isn't there.

## 3. Not recommended

- **Wave AI chat widget:** AgentMux's agent panes are already a far richer version of it.
- **Pinned tabs:** removed deliberately (`tabbar.tsx` migrates them away).
- **Tab backgrounds:** the renderer exists; a UI for it is cosmetic and not where the value is.
- **Telemetry:** out of scope.

## 4. Proposed delivery

| Step | Scope | Depends on |
|---|---|---|
| A | Secrets: named secrets, Armory tab, `secret:` references in env, `muxsh secret`, redaction | — |
| B | Badges: model, rollup, clear-on-focus, agent-pane states, bell, `muxsh badge` | — (coordinate with Korp's #4242 question chip) |
| C | Quake hotkey; drop-to-paste-path; OSC 52 (set only); image paste as a temp-file path | — |
| D | Process viewer grouped by agent pane, with kill tree | — |
| E | `muxsh meta`, `scrollback`, `notify` | B for `badge` |
| F | OpenAI-compatible endpoint for ambient calls, key via A | A |
| G | Decision: remote terminals or not; if yes, WSL first | owner |

Each step should start from its own spec, as the repo's other work does.

## 5. Sources

- Wave docs: [home](https://docs.waveterm.dev/), [secrets](https://docs.waveterm.dev/secrets), [durable sessions](https://docs.waveterm.dev/durable-sessions), [connections](https://docs.waveterm.dev/connections), [Wave AI](https://docs.waveterm.dev/waveai), [AI modes](https://docs.waveterm.dev/waveai-modes), [Claude Code](https://docs.waveterm.dev/claude-code), [wsh reference](https://docs.waveterm.dev/wsh-reference), [widgets](https://docs.waveterm.dev/widgets), [custom widgets](https://docs.waveterm.dev/customwidgets), [tabs](https://docs.waveterm.dev/tabs), [workspaces](https://docs.waveterm.dev/workspaces), [keybindings](https://docs.waveterm.dev/keybindings), [release notes](https://docs.waveterm.dev/releasenotes). Read 2026-10-02; some pages were summarised by the fetch tool, so exact key names are quoted only where the page stated them.
- AgentMux code at `9d206f9d7`: `crates/srv/src/identity/secret_store.rs`, `backend/storage/identities.rs`, `backend/shellintegration/muxsh.mjs`, `backend/wconfig/types.rs` (`app:globalhotkey`, `ssh:*`), `backend/shellexec.rs`, `server/service/client.rs`, `backend/providers.rs`, `backend/user_widgets.rs`, `backend/sysinfo.rs`, `frontend/app/view/term/termwrap.ts`, `frontend/app/view/term/term.tsx`, `frontend/app/tab/tabbar.tsx`, `docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md`.
