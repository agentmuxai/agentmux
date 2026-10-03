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
| 17 | Raw objc FFI boilerplate (`sel_registerName`, `objc_msgSend` and per-signature transmutes) is redeclared about 20 times. | `app/mod.rs`, `macos_compat.rs`, `platform_macos.rs`, `ui_tasks/window.rs`, `ui_tasks/drag.rs`, … | M: one `cfg(macos)` objc module with typed helpers |
| 18 | The "a pane is mid-close" gate (and in two places the quit-state gate) is pasted into every top-level window creator. | `commands/window/creation.rs:285,472`, `commands/drag.rs:413`, `window_pool.rs:451`, `pane_pool.rs:315`, … | S: `AppState::check_top_level_creation_allowed(caller)` |
| 19 | The host re-implements srv's `resolve_settings_dir()` (isolated channel, env overrides, config-home walk). | `cef/src/app/window_settings.rs:147`, `srv/backend/config_watcher_fs.rs:45` | M: move into `agentmux_common` |
| 20 | `set_window_opacity` handles the same two events four times; the macOS and Linux blocks are identical. | `commands/window/transparency.rs:239,255,284` | S |

## 4. Suggested order

1. §1 first: both are small, and each removes a real behavioural difference between two code paths. Decide the intended precedence (#1) and whether the string form of `cmd:env` is valid (#2) before writing the shared helper.
2. Then the S items in §2, one PR each or grouped by file: they shrink the controllers that the accuracy audit found most often out of date.
3. #17 and #19 are worth a short design note first (module boundary, crate placement).
