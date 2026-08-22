# SPEC: `muxspect find` — cross-instance block/agent lookup

**Date:** 2026-08-22
**Status:** Implemented
**Author:** Korp
**Repos touched:** `agentmux` (`agentmux-srv/src/server/muxspect_handlers.rs`,
`agentmux-srv/src/server/mod.rs`, `agentmux-srv/src/backend/shellintegration/muxspect.mjs`)
**Related:** Ext 4 of `docs/reports/REPORT_MUXSPECT_MUXLOG_CROSS_CHANNEL_INSPECTION_2026_08_22.md`;
builds on the cross-channel fan-out mechanism `SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md`
(Phase A) already shipped for `muxspect conversations`.

## 1. Problem

`muxspect` (Phase 1, `SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md`)
only ever queries the instance the caller is already inside — it cannot
answer "does a *different* running instance have a controller or subagent
dispatch matching this block id / agent name." A live debugging session
needed exactly this question answered and had no tool for it: confirming
which of several running channels actually owned a given block id took
manually reading `$AGENTMUX_*` env vars, walking `~/.agentmux/channels/*/versions/*/data`
by hand, and cross-referencing `Get-Process` output — the report's whole
motivating example.

## 2. What ships

`GET /api/v1/muxspect/find?block_id=X` or `?agent=X`, and the matching CLI
command `muxspect find <block_id_or_agent>`.

1. **Host tier (no network):** checks `state.process_broker.list()` — the
   same source `handle_muxspect_list` already uses — for a matching
   `block_id`, and (for an `agent` query) cross-references
   `state.reactive_handler.list_agents()` the same way
   `handle_muxspect_conversations`'s host tier already does.
2. **Cross-channel tier:** reads the shared reactive registry
   (`crate::registry::resolve_shared_reactive_dir` +
   `backend::reactive::registry::list_all_shared` — the exact mechanism
   `handle_muxspect_conversations`'s cross-channel tier already uses, see
   that handler's own doc comment for the security rationale). Each
   `AgentEntry` already carries `block_id`/`agent_id`/`channel`/`local_url`,
   so **matching costs zero network calls** — only a channel whose entry
   actually matches the query gets a forwarded `describe` call (same
   single-hop / `CROSS_CHANNEL_PREVIEW_TIMEOUT_MS` timeout discipline as the
   conversations handler) to fill in process/controller detail.

The CLI dispatches by shape: a UUID-looking query is sent as `block_id`,
anything else as `agent` — no separate flags needed for the common case of
"I don't remember which one this is, I just have the string."

Always 200; an empty `results` array is a legitimate "not found on this
host, in any known channel" answer, same posture as every other handler in
this file (not an error). More than one result is possible and, if it
happens, is a real anomaly worth surfacing rather than silently collapsing
to the first match.

## 3. Non-goals / limitations

- **LAN/WAN tiers are not reached.** Same Phase A boundary
  `muxspect conversations` already has — no remote-read protocol exists for
  those tiers yet.
- **Not tested against a live rebuilt instance in this change.** Verifying
  this specific endpoint end-to-end (the way Ext 1 and Ext 3 were verified
  against this session's actual running instance) requires a full
  srv rebuild + relaunch, since the currently-running binary predates this
  route. Verified instead via `cargo check`, the CLI's `parseArgs` unit
  tests, and direct mirroring of `handle_muxspect_conversations`'s
  already-proven fan-out pattern — flagged here rather than silently
  presented as more thoroughly checked than it is.
- Does not add a dedicated Rust-side unit/integration test for the handler
  itself (no existing precedent for testing the async, `AppState`-composing
  handlers in this file in isolation — the file's existing `#[cfg(test)]`
  module only covers pure helpers like `classify_last_error_source`/
  `last_error_frame`). Relies on code-review + the mirrored pattern's
  existing track record instead.
- **Cross-channel `block_id` search only covers agent-registered blocks**
  (reagent P2 on PR #2745). It reads the shared `AgentEntry` registry, not
  a remote channel's full controller inventory — a plain shell/terminal
  block, or any other non-agent controller, in a *different* channel is
  not findable by `block_id` on the cross-channel tier (the host tier has
  no such limit — it searches this instance's full `ProcessBroker::list()`,
  any controller type). Reaching a non-agent controller cross-channel would
  need a remote `/api/v1/muxspect/list`-style forward-and-filter call, a
  bigger change than a registry lookup; left for a follow-up if it turns
  out to matter in practice.
