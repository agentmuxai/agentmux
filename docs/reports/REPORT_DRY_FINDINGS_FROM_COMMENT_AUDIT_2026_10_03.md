# REPORT: DRY and modularization findings from the comment accuracy audit

**Date:** 2026-10-03
**Author:** agent3
**Status:** analysis. Nothing here is fixed; this is a work list.
**Related:** [`REPORT_COMMENT_COMPRESSION_WORTH_IT_2026_10_02`](REPORT_COMMENT_COMPRESSION_WORTH_IT_2026_10_02.md) §6 (the dead-reference triage that led to the audit).

## 0. Where these came from

The comment accuracy audit re-read 249 comment blocks in 164 files against the current code. While reading, the four auditors logged clear duplication and modularization cases they saw, with file:line evidence. They were told to log, not fix, and to log only clear cases. That makes this a by-catch list, not a systematic survey: the files it covers are the ones that had stale comments.

22 entries came in; two pairs were the same finding seen from different files (the objc FFI boilerplate, the window-creation gate), so 20 remain. I checked the two that look like latent bugs (§1) against the code myself. The rest are as the auditors recorded them; re-check each before acting on it.

Effort: **S** = one small PR, mechanical; **M** = a few files or a design choice.

## 1. Duplication that already disagrees (possible bugs)

| # | What | Where | Effort |
|---|---|---|---|
| 1 | **Tools PATH is built twice with different precedence.** The PTY shell path prepends the bundled tools dir and *appends* the user tools dir after the system PATH (documented as deliberate); the persistent agent-CLI path *prepends both*. A user-installed tool therefore shadows the system copy for the agent CLI but not for its shell. Verified. | `blockcontroller/shell/lifecycle.rs:~790`, `server/agent_handlers/input.rs:~690` | S: one `tool_store::tools_path(existing)` with the documented precedence |
| 2 | **`cmd:env` parsing is copied at about six sites, and the copies accept less than the helper.** `acp.rs`'s `meta_string_map` also accepts the JSON-string form of the map; the hand copies silently return an empty map for it. Verified that the helper exists and is tested for the string form. | `server/app_api/agent_io.rs:273`, `server/agent_handlers/input.rs:784`, `persistent/eager_resume.rs:97`, `ambient/cli.rs:55`, `app_server_controller.rs`, … | S: promote `meta_string_map` (or `cmd_env_of(meta)`) to `blockcontroller/mod.rs`, decide once whether the string form is valid |

## 2. Duplication in the agent controllers (srv)

| # | What | Where | Effort |
|---|---|---|---|
| 3 | The same 10-field `BlockControllerRuntimeStatus` literal is hand-built at about 11 sites; `persistent/status.rs` already says it is "worth factoring out". | `acp.rs:157,376,417,502…`, `persistent/status.rs:59` | S |
| 4 | `answer_question` / `deny_question` are near-copies (lookup, error text the frontend matches, stdin send, dead-air fallback). | `persistent/input.rs:551,670` | M |
| 5 | The NDJSON stdout reader (stats, session-id capture, blockfile append) is hand-rolled in both the subprocess and persistent controllers. | `subprocess/host_spawn.rs:314`, `persistent/spawn.rs:783` | M |
| 6 | "Fetch block, wrap in `MuxObjUpdate`, broadcast `waveobj:update`" is re-inlined at three sites although `activity_watcher::broadcast_block_update` exists for exactly this. | `core.rs:126`, `session_recovery.rs:83,131` | S |
| 7 | Two arms of `ResumeEvent::SessionCaptured` have identical bodies. | `persistent_resume.rs:339,387` | S |
| 8 | `agent.answer` / `agent.cancel` RPC handlers repeat the same arg check, controller lookup, downcast and error text. | `server/websocket.rs:1588,1629` | S |
| 9 | `AGENTMUX_AGENT_ID` then `WAVEMUX_AGENT_ID` lookup is implemented twice over two map types. | `persistent/mod.rs:252`, `shell/lifecycle.rs:281` | S |
| 10 | Block agent id from `agentId` with the legacy `agent:id` fallback is reimplemented at three sites. | `identity/resolver/inject.rs:480`, `agent_session/migrations/v1_blocks.rs:62`, `transcript_backfill.rs:219` | S |
| 11 | `subagent:spawned` / `subagent:completed` event payloads are built twice with identical shapes. | `subagent_watcher/jsonl.rs:328,425,486,543` | S |
| 12 | `generate_dispatch_name` repeats `generate_subagent_name`'s whole pipeline; its own doc says it "mirrors … exactly". | `ambient/tasks.rs:227,285` | S |
| 13 | Session-preview parsing (plus its `collapse_preview`) is copied in two places. | `agent_session/archive.rs:338`, `server/agent_handlers/mod.rs:94` | S |
| 14 | `memory_dir_for_registry_record` inlines `working_dir_from_record`. | `server/native_memory_handlers.rs:350`, `agent_registry_lookup.rs:100` | S |
| 15 | The HTTP memory-write provenance struct copies `NativeMemoryWriteProvenance`. | `server/http_app_api.rs:86`, `rpc_types/native_memory.rs:86` | S |
| 16 | "Open the shared definitions store and attach it" is repeated in two migrations and bootstrap. | `m0020…:63`, `m0021…:134`, `bootstrap/stores.rs:277` | S |

## 3. Duplication across crates (cef / common)

| # | What | Where | Effort |
|---|---|---|---|
| 17 | Raw objc FFI boilerplate (`sel_registerName`, `objc_msgSend` and per-signature transmutes) is redeclared about 20 times. | `app/mod.rs`, `macos_compat.rs`, `platform_macos.rs`, `ui_tasks/window.rs`, `ui_tasks/drag.rs`, … | M: one `cfg(macos)` objc module with typed helpers **Deferred:** macOS-only; `cargo check --target aarch64-apple-darwin` cannot run on the Windows dev host (a dependency build script needs a macOS C toolchain), so it needs a macOS machine to verify. |
| 18 | The "a pane is mid-close" gate (and in two places the quit-state gate) is pasted into every top-level window creator. | `commands/window/creation.rs:285,472`, `commands/drag.rs:413`, `window_pool.rs:451`, `pane_pool.rs:315`, … | S: `AppState::check_top_level_creation_allowed(caller)` |
| 19 | The host re-implements srv's `resolve_settings_dir()` (isolated channel, env overrides, config-home walk). | `cef/src/app/window_settings.rs:147`, `srv/backend/config_watcher_fs.rs:45` | M: move into `agentmux_common` **Not a duplicate (checked 2026-10-03):** srv resolves one dir from `AGENTMUX_CONFIG_HOME` with a root fallback; the host scans candidate files from `AGENTMUX_CONFIG_DIR`, which only the host process has. Unifying them would make isolated channels read the wrong file. |
| 20 | `set_window_opacity` handles the same two events four times; the macOS and Linux blocks are identical. | `commands/window/transparency.rs:239,255,284` | S |

## 4. Suggested order

1. §1 first: both are small, and each removes a real behavioural difference between two code paths. Decide the intended precedence (#1) and whether the string form of `cmd:env` is valid (#2) before writing the shared helper.
2. Then the S items in §2, one PR each or grouped by file: they shrink the controllers that the accuracy audit found most often out of date.
3. #17 and #19 are worth a short design note first (module boundary, crate placement).

## 5. Added by the dead-symbol pass (2026-10-03)

A second accuracy pass checked 527 comments that named a function, type or field that exists nowhere in the code (184 were stale and are fixed in the same PR as this section). Its auditors logged 14 more cases the same way. #21 already disagrees in behaviour.

| # | What | Where | Effort |
|---|---|---|---|
| 21 | **Rust re-implements the frontend's per-tool header detail, and they have drifted** (WebFetch: Rust shows the full URL, the frontend host+path; the frontend also matches `read`/`read_file`). | `subagent_watcher/parse.rs:59` (`tool_input_detail`), `tool-meta/tool-descriptors.ts:243` (`toolDetail`) | M: send structured tool input to the Swarm feed and format it once, in the frontend |
| 22 | The "surface a final error line" sequence (classify, persist last failure, publish `EVENT_AGENT_FAILURE`) is repeated at five sites. | `persistent/resume_retry.rs:35`, `persistent/spawn.rs:1205,1299`, … | S: one `publish_failure(...)` free fn |
| 23 | The process waiter's wait arm and kill arm inline the same bounded await-then-abort of both reader tasks. | `persistent/spawn.rs:1497,1964` | S |
| 24 | `spawn.rs` (~2,200 lines) is essentially one function that inlines the stderr reader, the stdout reader and the process waiter. | `persistent/spawn.rs` | L: one file per task under `persistent/`, a small context struct |
| 25 | Two near-identical `emit_message_accepted` helpers. | `persistent/queue.rs:631`, `subprocess/mod.rs:309` | S |
| 26 | `random_token()` hand-rolls the CSPRNG fill `random_seed_bytes()` already provides (#4094 consolidated the other copies). | `storage/agent_tokens.rs:57`, `storage/agent_lan_keys.rs:31` | S |
| 27 | Two byte-identical `StubTracker` modules under opposite `cfg`s. | `process_tracker/mod.rs:206,234` | S |
| 28 | `carry_skills` / `carry_mcp_servers` are ~200-line near-copies in one migration. | `m0031_…:264,465` | M, only if the migration is touched again |
| 29 | "Create a block as a tab of this pane" (open, re-resolve the pane, clean up if gone, push onto the stack) exists three times. | `layout/lib/layoutStack.ts:76`, `agent/quick-fork.ts:205`, `agent/open-history-tab.ts:113` | M |
| 30 | The tab-bar tear-off re-implements the pool-first/cold-path fallback `openTearOffWindow` already provides. | `tab/tab-tearoff-rpc.ts:104`, `drag/tear-off-pool-helper.ts:137` | S |
| 31 | A pane's reactive stack id list is derived twice with the same logic. | `tab/pane-leaf-chrome.tsx:307`, `element/PaneChrome.tsx:101` | S |
| 32 | `tear_off_hook.rs` (~1,560 lines) holds both the Windows mouse hook and a ~670-line inline macOS module. | `commands/tear_off_hook.rs:894` | S: split into `tear_off_hook/{mod,windows,macos}.rs` |
| 33 | The main-frame URL expression is copied three times; the pane block id is resolved three times in one callback. | `browser_pane/callbacks.rs:476,525,606` | S |
| 34 | The auth reducer and controller carry a full two-phase save path (`authenticated`/`saving` kinds, `SaveBundleClicked`, cancel/dispose guards) for an `auth.savebundle` RPC that was never built. | `auth/auth-state.ts:82`, `auth/auth-flow-controller.ts:233` | M: build the RPC, or delete the dormant states (see §6) |

## 6. Code findings: dormant or unbuilt features the comments described as live

These surfaced as stale comments; the comments are now accurate, but each is a decision about code, not wording. I checked each against the code.

| What | Evidence | Decision needed |
|---|---|---|
| **Top-level window creation pipeline (H.6) is dormant.** The reducer arms emit `HostEvent::Effect { PostCreateWindow }`, but `HostCommand::EnqueueTopLevelWindow` and `TopLevelCallbackFired` are never dispatched outside the reducer and its tests, and `host_dispatch` only logs events. Nothing is lost today because nothing enqueues. | `reducer/top_level.rs:15,97,123`; `reducer/mod.rs:1241,1244`; no production dispatch site | Finish the migration (add the effect executor and switch the creators over) or delete the dormant arms |
| **`auth.savebundle` was never built**, but the frontend carries its two-phase flow (#34). | No non-comment reference outside the auth reducer and its tests | Build it or delete the scaffolding |
| **Process tracking is Windows-only.** The table in `process_tracker/mod.rs` listed Linux and macOS trackers as shipped; those platforms get the no-op `StubTracker`. | `process_tracker/mod.rs` | Accept, or schedule the Linux/macOS trackers |
| **The layout doctor skips unbalanced geometry passes.** `reportLayoutViolations` runs only when `updateTree` balances; resize, stack and magnify call `updateTree(false)`, so a corruption introduced there is not logged until the next balancing pass. | `frontend/layout/lib/layoutGeometry.ts:258-269` | Validate every pass (cost: one walk per resize frame), or accept |
| **Planned `Resync` recovery for the event buffer was never built.** `event-buffer.ts` documented a working force-push/`Resync` protocol. | No `Resync` command or force-push request exists | Accept (comment now says so), or build it |
