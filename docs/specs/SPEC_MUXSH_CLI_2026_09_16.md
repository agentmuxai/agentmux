# SPEC: `muxsh` — a terminal-side pane-opener for the Agent App API

**Author:** Vmer
**Date:** 2026-09-16
**Status:** active — PRs touching this work, newest first: #3255
**Related:** `docs/reports/REPORT_WSH_STYLE_CLI_FOR_AGENT_APP_API_2026_09_16.md` (research + naming decision this spec implements), `docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md` (the `muxopen` precedent this follows exactly), `agentmux-docs/.../internals/agent-app-api.md` (`OpenEditor` / `pane.open`), `docs/specs/archive/SPEC_RETIRE_WSH_2026_04_12.md` (why AgentMux doesn't import Wave Terminal's `wsh` wholesale)

---

## 1. What this ships

A new shell command, **`muxsh`**, following the exact architecture of the existing `muxlog` / `muxspect` / `muxopen` family: a thin Node core deployed to `~/.agentmux/shell/muxsh.mjs`, delegated to by a shell function in each of the four supported shells (bash/zsh/fish/pwsh), authenticated the same way all three siblings already are — `$AGENTMUX_LOCAL_URL` + `$AGENTMUX_AUTH_KEY` inherited from the pane environment, no new IPC, no new auth scheme.

First cut, two subcommands, both thin wrappers over the already-existing, already-generic `POST /api/v1/pane/open` route (`CommandPaneOpenData` — confirmed it accepts `view: "editor"|"term"|"browser"|"sysinfo"|"help"|"media"`, not just the editor-only subset `OpenEditor`'s MCP tool exposes):

| Command | `view` | Required arg | Purpose |
|---|---|---|---|
| `muxsh open <file>` | `editor` | `file` | Open a file in an editor pane from the terminal — the most-demoed `wsh` gesture (`wsh view`/`wsh editor`) |
| `muxsh web <url>` | `browser` | `url` | Open a URL in a browser pane from the terminal (`wsh web`) |

This is deliberately narrow. Per the linked research report §4: don't import Wave Terminal's `wsh` wholesale (its SSH/WSL deployment, OSC hijacking, and bolt-on-AI subcommands don't map to AgentMux, exactly as the 2026-04-12 retirement spec found and nothing has changed there). Close the one specific, real gap instead — nothing on the command line can open a pane today; everything else goes through the GUI or an agent's MCP tool call.

## 2. Why through the Agent App API, and why this exact route

`agentmux-docs/.../agent-app-api.md` already draws the two-layer distinction this spec relies on: MCP tools are for agent/LLM callers going through `agentmux-mcp`'s stdio session; a terminal CLI is the *other* documented category — "scripts or tooling that run with explicit access to the auth key." `muxopen.mjs`'s own header comment states the precedent directly: *"authenticated exactly the way muxspect and agentmux-mcp already are... No new IPC, no new auth scheme."* `muxsh` reuses that same trust tier, not MCP, not a new WebSocket client.

`/api/v1/pane/open` is the right target specifically because it's already generic — `handle_pane_open` (`server/mod.rs`) is a thin wrapper over `app_api::open_pane`, the *same* logic the WebSocket `pane.open` RPC and the `OpenEditor` MCP tool both call, and its request shape (`CommandPaneOpenData`, `rpc_types/block.rs:446`) already accepts `view: "browser"` + `url` — the MCP/REST-table docs undersell this: the endpoint has always supported more than the curated `OpenEditor` tool exposes. No backend change is required to build `muxsh open`/`muxsh web` — this is a pure CLI addition against an existing route.

## 3. `muxsh open <file>` — request/response shape

```
POST /api/v1/pane/open
X-AuthKey: $AGENTMUX_AUTH_KEY
{
  "view": "editor",
  "file": "<absolute path>",
  "title": "<optional>",
  "tab_id": "<$AGENTMUX_TABID, if set>",
  "split_direction": "<right|left|down|up, only sent if split_reference_block_id is too>",
  "split_reference_block_id": "<$AGENTMUX_BLOCKID, only sent if non-empty>",
  "focus": true,
  "tree_expanded": true,
  "floating": false
}
→ { "block_id": "...", "tab_id": "...", "view": "editor", "created": true }
```

**`split_direction` alone does nothing.** The server's `resolve_placement`
(`server/app_api/pane.rs`) falls back to plain `insert` whenever
`split_reference_block_id` is empty, regardless of `split_direction` — this
was a real bug in the first cut of this spec/implementation (ReAgent
review, PR #3255), caught before merge. `muxsh` reads `$AGENTMUX_BLOCKID`
(and `$AGENTMUX_TABID`) — already injected into every terminal pane's
environment for exactly this purpose
(`blockcontroller/shell/lifecycle.rs`), the same source the `OpenEditor`
MCP tool already uses — and only sends `split_direction` paired with a real
`split_reference_block_id`. Without a known block id, the new pane is
inserted at the tab root instead, regardless of `--split`.

CLI surface (mirrors `muxopen`'s flag style):

```
muxsh open <file>                    open by absolute path, default split=right (needs $AGENTMUX_BLOCKID)
muxsh open <file> --title <t>        set the pane/tab title
muxsh open <file> --split <dir>      right (default) | left | down | up
muxsh open <file> --collapse-tree    open with tree_expanded=false
muxsh open <file> --floating         open as a floating window instead of a docked split
muxsh open <file> --no-focus         open without focusing the new pane
muxsh help                           this text
```

## 4. `muxsh web <url>` — request/response shape

```
POST /api/v1/pane/open
X-AuthKey: $AGENTMUX_AUTH_KEY
{ "view": "browser", "url": "<url>", "title": "<optional>", "focus": true, "floating": false }
→ { "block_id": "...", "tab_id": "...", "view": "browser", "created": true }
```

```
muxsh web <url>
muxsh web <url> --title <t>
muxsh web <url> --split <dir>        right (default) | left | down | up — same $AGENTMUX_BLOCKID requirement as `open`
muxsh web <url> --floating
muxsh web <url> --no-focus
```

Only `--collapse-tree` is editor-only (`tree_expanded` is documented in `rpc_types/block.rs` as "`editor` only" — nothing else in `CommandPaneOpenData` is) and is rejected with a clear error if passed to `muxsh web`, rather than silently ignored. `--split` is not editor-only — `split_direction`/`split_reference_block_id` are generic pane-placement fields — so `muxsh web` supports it too.

## 5. Non-goals (this cut)

- No `muxsh term`/`sysinfo`/`help`/`media` — the route supports them, but there's no demonstrated CLI-shaped demand yet (unlike `open`/`web`, which map directly to `wsh`'s two most-cited gestures). Adding one later is a small, mechanical extension of the same pattern — not a reason to over-build now.
- No unification of `muxlog`/`muxspect`/`muxopen` under `muxsh` as a dispatcher. `muxsh` ships as its own standalone tool, same tier as its three siblings, per the research report's open question §4 — the repo owner hasn't picked a direction on family consolidation, and this spec doesn't need to wait on that decision to close a real, narrow gap.
- No cross-instance/cross-channel targeting — same single-instance scope every sibling in this family has today (`muxopen`'s own doc: *"This opens the agent in the instance this pane belongs to; cross-instance opening is not implemented here"*).

## 6. Implementation plan

Exact mechanical mirror of `muxopen`'s existing deployment (`agentmux-srv/src/backend/shellintegration.rs`, `docs/reports/REPORT_AGENT_OPEN_API_GAP_2026_09_06.md`):

1. `agentmux-srv/src/backend/shellintegration/muxsh.mjs` — new core. Pure `parseArgs`/`renderResult` functions (unit-testable, no I/O — same contract `muxopen.mjs` and `muxspect.mjs` already follow), `main()` does the fetch + auth-env checks + error rendering.
2. `agentmux-srv/src/backend/shellintegration/muxsh.test.mjs` — vitest unit tests for `parseArgs`/`renderResult`, mirroring `muxopen.test.mjs`'s structure.
3. `agentmux-srv/src/backend/shellintegration.rs`:
   - `const MUXSH_JS: &str = include_str!("shellintegration/muxsh.mjs");`
   - Deploy to `shell_base.join("muxsh.mjs")`, same block shape as the `muxopen_path` deploy.
   - Add `MUXSH_JS.hash(&mut hasher)` to `version_marker()`.
   - **Drive-by fix, same function:** `version_marker()` currently hashes `BASH_SCRIPT`/`ZSH_SCRIPT`/`PWSH_SCRIPT`/`FISH_SCRIPT`/`MUXLOG_JS`/`MUXSPECT_JS` but **not** `MUXOPEN_JS` — an edit to `muxopen.mjs` alone doesn't change the deployment marker, so an already-running instance would never redeploy an updated `muxopen.mjs` (the exact failure mode the function's own comment warns about, just not fully applied). Adding `MUXOPEN_JS.hash(&mut hasher)` alongside the new `MUXSH_JS` line closes this while touching the same lines anyway.
   - Extend the deployment test (~line 300) to assert `muxsh.mjs` exists alongside its siblings.
4. Shell integration, one block added to each of the four scripts, copying the existing `muxopen` block's shape exactly (only the resolved-path syntax differs per shell — already proven per-shell in the `muxopen` blocks):
   - `bash.sh` / `zsh.sh` — `_AGENTMUX_MUXSH_JS=...`, `muxsh() { ... }`
   - `fish.fish` — `set -g _agentmux_muxsh_js ...`, `function muxsh ... end`
   - `pwsh.ps1` — `$global:AgentmuxMuxshJs = ...`, `function Global:muxsh { ... }`
5. `docs/MUXSH.md` — new reference doc, same shape as `docs/MUXLOG.md`/`docs/MUXSPECT.md`.
6. Changeset: `task changeset -- minor "feat(shell): add muxsh — open editor/browser panes from the terminal"` (new user-facing command surface, not a patch-level fix).

## 7. Verification

- `cargo check --workspace` clean.
- `npx vitest run muxsh.test.mjs` clean.
- Manual, against a running dev instance: `node ~/.agentmux/shell/muxsh.mjs open <file>` opens a real editor pane; `muxsh web <url>` opens a real browser pane; both idempotency-free (unlike `muxopen`, opening the same file twice is expected to open two panes — `pane.open` has no dedup concept the way `agent.open` does for a single agent identity, and that's correct: files aren't identity-scoped the way agents are).
- Confirm `muxsh.mjs` is present in `~/.agentmux/shell/` after a version bump/restart (the redeploy path this spec's drive-by fix also protects).

## 8. Open questions

Same as the research report's §6, restated for this specific PR: is the `open`/`web` scope right for a first cut, or is there a different specific gesture (e.g. `muxsh term`) that matters more? Family-consolidation (§5) remains explicitly out of scope here, not forgotten.
