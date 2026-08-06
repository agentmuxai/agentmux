# SPEC — `muxspect dock`: diagnose and clear stuck Activity Dock entries

**Date:** 2026-08-06
**Type:** Extension to an existing tool (`muxspect`) — adds its first mutating command
**Trigger:** Live debugging session, tonight — a Bash tool call was rejected by
the outer CLI harness before it ever executed; its `ToolNode` stayed stuck at
`status: "running"` in the Agent pane's Activity Dock, and `muxspect describe`
could not see or explain it. Notable: `SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md`'s
own stated trigger (§ header) was **the same bug** — "the user asked to locate
'2 stuck sleep dock items.'" This is a recurring, still-unsolved pain point,
not a one-off.
**Status:** Proposed
**Related:** `docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md`
(§5.3, §7 — the two prior scope exclusions this spec proposes narrowly
reversing), `docs/specs/SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md`
(§3, §7 — independently diagnosed the identical bug, recommends a
complementary first-party in-renderer fix), `docs/reports/REPORT_MUXSPECT_SPAWN_REFUSAL_DIAGNOSIS_EXTENSION_2026_08_03.md`
(closest prior art — `last_error`, same "diagnose a wedged pane" shape),
Discussion #2375 (process/turn-liveness tracking thread — lists this exact bug
as "⏳ Diagnosed, not fixed"; per that thread's own instruction, this spec is
posted there rather than forked into a new discussion)

---

## 0. What this is and isn't

This is **not** the fix for the bug. The bug — an optimistically-created
`ToolNode` that never receives its terminating event when an outer-harness
guardrail rejects a tool call pre-execution — has an already-recommended,
better, first-party fix on record (§1.2). This spec is the **diagnostic and
manual-remediation tooling** on top: give `muxspect` a way to (a) see that a
pane has a stuck entry and why, and (b) clear it immediately, live, without a
full pane reload — valuable on its own even after the root-cause fix ships,
because an active debugging session (like tonight's) shouldn't have to wait
for an auto-expiry timer, and an agent (not a human driving a browser) needs
a scriptable way to both notice and act on this.

## 1. Root cause (for context — not what this spec fixes)

### 1.1 The state machine gap

Traced end-to-end in `frontend/app/view/agent/`:

1. A `ToolNode` is created **optimistically** at `status: "running"` the
   instant a `tool_call` stream event is parsed — `stream-parser.ts`'s
   `toolCallToNode()` (line ~461, unconditional `status: "running"` at
   line ~519) — before any confirmation the tool actually executed.
2. The only programmatic path back out of `"running"` for that specific
   node is a matching `tool_result` content block carrying the same
   `tool_use_id` (`providers/claude-translator.ts:334-350`).
3. An outer-harness rejection *before* execution (no matching validator
   exists anywhere in `agentmux-srv`/`agentmux-bashwrap` — this is
   external, upstream-CLI behavior, not an AgentMux bug in the rejection
   itself) surfaces as a **top-level, turn-terminating `"result"`/`is_error`
   frame instead of a scoped `tool_result` for that id** — the same frame
   shape `muxspect`'s own `last_error` field reads. It produces a separate
   `agent_error` bubble and ends the turn; it never touches the still-open
   `ToolNode`.
4. `session_end` → `finalizeTurn()` (`hooks/useTurnLifecycle.ts:73`)
   dispatches `TurnEnd` to the **pane-state** reducer (clears "Working…")
   but never touches the **document** reducer that owns `nodes[]`, and
   never calls `scrubOrphanedInProgress`.
5. `scrubOrphanedInProgress` (`frontend/app/store/agent-document/reducer.ts:53-148`)
   only runs on three real triggers, all session/reload/reconnect
   boundaries — `SessionEnd` (pane unmount/reconnect/close), `HistoryLoaded`,
   `HistoryRestored` — never on an ordinary turn boundary. (A fourth,
   standalone `ScrubOrphanedInProgress` command exists in the reducer
   specifically for scrubbing without crossing a session boundary, but has
   zero live dispatch call sites anywhere in the app today — exercised
   only by its own test.)

Net: the node survives, visibly, until the pane's subscription actually
tears down (close/reload/reconnect) — not before.

### 1.2 The already-recommended fix (out of scope here, cited for reconciliation)

`SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md:51,53,108,117`
diagnosed this same bug independently and recommends: give the dock (or
`scrubOrphanedInProgress`) a **generation/max-age fallback** so an orphaned
`"running"` entry can self-expire on its own, without waiting for a session
boundary — modeled on the generation-tagged state machine pattern already
shipped for an analogous backend bug (`persistent_resume.rs`, PR #2371/#2373).

That fix, when it ships, closes the bug **automatically for every user**,
which this spec's tooling does not — a CLI command only helps if someone
notices and runs it. **This spec does not duplicate or compete with that
work.** It's the complementary manual/scriptable layer: useful before that
fix ships, and still useful after (an auto-expiry timer is necessarily
minutes-scale to avoid false positives on legitimately long-running tools;
"clear it right now, I can see it's dead" is a different, faster need an
active debugging session — human or agent — still wants).

## 2. Why `muxspect` currently can't help, and why that's being narrowly revisited

`SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md` made two deliberate
scope calls this spec proposes narrowly reversing:

- **§5.3**: full Activity Dock introspection is out of scope for v1 because
  the dock is pure in-renderer SolidJS state, invisible to any backend-only
  tool without either mirroring `ToolNode` state server-side or using CEF's
  own remote-debugging (Chrome DevTools) protocol. The recommendation was:
  use CEF devtools for "inspect this renderer's live JS state" generally,
  don't build a custom mirror in `muxspect`.
- **§7**: "any mutating action (kill/restart — already exists elsewhere,
  this tool doesn't duplicate it)" is explicitly out of scope, any phase —
  the design intent was "point at existing mutating RPCs, don't invent new
  ones inside `muxspect`."

Both calls were correct **for the general case** they were written for
(full dock introspection; kill/restart duplication). Neither directly
covers what's being proposed here:

- This isn't full dock introspection — it's one narrow, specific summary
  (per-node id/tool-name/status/age) sufial for the one diagnostic question
  "is anything stuck," not a general renderer-state mirror.
- There's no existing mutating RPC for "clear one dock entry" to point at
  instead — `agent.stop` kills the controller (too broad, ends the whole
  turn/session), and `tooldecision`/`agentanswer` mutate a *different*
  status transition (`pending_approval`/`awaiting_answer`), not a stuck
  `"running"` node. This would be a genuinely new capability, not a
  duplicate.
- CEF devtools remains correct advice for genuinely general renderer
  debugging, but it isn't scriptable/agent-usable the way a CLI command is
  — and the concrete, recurring failure mode this spec targets is an
  *agent* (not a human with a browser open) needing to notice and fix its
  own pane's stuck state as part of a normal session, exactly as happened
  tonight.

## 3. Design

### 3.1 Read side: a lightweight dock snapshot, pushed not polled

Rather than adding a request/response path that asks one specific live
renderer connection a question (a new, riskier "srv reaches into one
particular open pane" mechanism), the renderer **pushes** a small
dock-summary delta to `agentmux-srv` whenever a `ToolNode`'s status
changes, as a new fire-and-forget RPC command over the frontend's existing
single persistent WS connection (`frontend/app/store/ws.ts`) — the same
connection every other `RpcApi.*Command` already uses, called the same
"dispatch, don't await the response" way `RpcApi.SetMetaCommand` already
is at `frontend/app/view/armory/armory-view.tsx:46-49`. (There is no
separate telemetry-specific channel to reuse — verified; this is a new RPC
verb on the one channel that already exists.) Payload per node: `block_id`,
`node_id`, `tool_name`, `status`, `timestamp` (mapped from `ToolNode.timestamp`
— there is no separate `created_at`/`updated_at` pair on the node; the push
call itself stamps `observed_at` server-side on receipt). No transcript
content, no arguments/output — just enough to answer "what's stuck and how
long."

`agentmux-srv` keeps the **latest snapshot per block in memory only**
(never persisted — this mirrors `ProcessBroker`'s own cached-not-durable
status shape, and matches the dock's own nature: if no renderer is
currently attached to a block, there's nothing live to report, and that's
a correct, distinct answer, not missing data).

New `muxspect` command:

```bash
node ~/.agentmux/shell/muxspect.mjs dock <block_id>
```

Output: each tracked node's id, tool name, status, and age; nodes past the
existing `TOOL_PROMOTION_MS` (30s) threshold with `status: "running"` and
no matching entry in `ProcessBroker`'s own tracked-process list for that
block are flagged distinctly (e.g. `STUCK?`) — this is the actual
root-cause-assist part: correlating "dock says running" against "srv sees
no backing process for this block at all" is the same signal a human would
manually cross-reference today, automated.

### 3.2 Write side: clear one node, live

```bash
node ~/.agentmux/shell/muxspect.mjs dock clear <block_id> <node_id>
```

`agentmux-srv` validates the block/node exist in its cached snapshot, then
publishes a new WPS event (`EVENT_DOCK_CLEAR`, `"dock:clear"`) scoped to
`scopes: vec![format!("block:{block_id}")]` — the same real, currently-live
server-side scope-routing mechanism `EVENT_SHELL_NODE_CREATE` already uses
(`agentmux-srv/src/server/mod.rs:698-710`, subscribed via
`waveEventSubscribe({ scope: \`block:${blockId}\` })` in
`frontend/app/view/agent/hooks/useShellNodeStream.ts:95-118`). This is
genuine server-side per-connection routing (`wps.rs`'s `Broker::subscribe`/
publish path) — a renderer not currently displaying that block never
receives the message at all; no client-side block-id self-filter is
needed. The one filter the frontend still does itself is "is `node_id`
still present in *my* document" (a no-op if the node already resolved by
the time the event arrives). The receiving renderer dispatches a new
targeted reducer command — `ForceCancelToolNode { nodeId, at }` — setting
`status: "canceled"` and (since `ToolNode` has no dedicated
note/reason field) reusing `summary` for the distinguishing marker
(`"⏹ Canceled — cleared via muxspect"`, matching how the existing orphan
scrub already piggybacks status text onto `summary`)
so it's visually/auditably different from a normal cancellation. Every
other renderer (wrong block, or node already resolved) no-ops, exactly like
existing broadcast events already behave.

If no renderer is currently attached to that block, the command reports
"no live renderer — nothing to clear" rather than silently succeeding.
This is not a gap: with no renderer attached, the dock state doesn't
visibly exist to anyone right now either, and `HistoryRestored`'s existing
`scrubOrphanedInProgress` already correctly cleans it up the next time the
pane *is* reopened (§1.1) — the manual-clear command's whole reason to
exist is the case where someone is looking at it right now, which requires
a live renderer by definition.

### 3.3 Why scope-routed WPS, not a targeted single-connection push

`agentmux-srv`'s WPS broker (`wps.rs`) already does real server-side,
per-connection scope routing keyed on strings like `block:<id>` — a
publish only reaches WS connections actually subscribed to that scope
(`Broker::subscribe`'s `scope_subs` lookup on the publish path), not every
connection with client-side filtering after the fact. Reusing it for
`dock:clear` avoids inventing a new "address one specific connection out
of possibly several open windows for the same instance" mechanism from
scratch, and correctly handles the same block being open in two windows
(both get scoped, both react) without extra logic. Both directions —
§3.1's frontend→srv RPC push and §3.2's srv→frontend WPS event — reuse
existing channel machinery; only the new RPC verb and the new
`EVENT_DOCK_CLEAR` scope-routed event are new.

## 4. Non-goals

- **Not** a fix for the underlying bug (§1.2 covers that, elsewhere,
  already recommended).
- **Not** full Activity Dock introspection — no transcript content,
  arguments, or output ever leaves the renderer; only the four-field
  per-node summary in §3.1.
- **Not** a general "mutate any renderer state from the CLI" capability —
  one command, one narrow effect (flip a specific stuck node to
  `canceled`), nothing else.
- **Not** a replacement for `agent.stop`/kill-restart — this never touches
  the controller, process tree, or turn/session state; it only clears a
  UI-visible status entry that (per §1.1) no longer corresponds to
  anything real.
- **Does not** change `muxspect`'s Phase 2 cross-instance-discovery
  roadmap (`SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md` §4/§5.2) —
  orthogonal, unaffected.

## 5. Implementation phases

### Phase 1 — Read: dock snapshot push + `muxspect dock <block_id>` (1 PR)

- Frontend: on every `ToolNode` status transition, push the §3.1 delta to
  `agentmux-srv` over the existing telemetry WS channel.
- `agentmux-srv`: in-memory latest-snapshot-per-block cache; new
  `muxspect`-facing RPC to read it.
- `muxspect.mjs`: `dock <block_id>` subcommand; cross-reference against
  `ProcessBroker`'s existing tracked-process list to compute the `STUCK?`
  flag.
- Test: synthetic snapshot with one `"running"` node past the promotion
  threshold and zero backing processes → flagged; one within-threshold
  `"running"` node → not flagged.

### Phase 2 — Write: `muxspect dock clear` (1 PR, depends on Phase 1's snapshot cache existing for validation)

- New mutating RPC, srv-side validation against the Phase 1 cache.
- Broadcast event + frontend `ForceCancelToolNode` reducer command
  (agent-document reducer, alongside the existing `ScrubOrphanedInProgress`
  command it's structurally closest to).
- "No live renderer attached" response path (§3.2).
- Test: two-renderer scenario (block A open, block B open) confirms only
  block A's matching node clears; confirm the cleared node's `status`
  and distinguishing note round-trip correctly through a reload.

### Phase 3 — Cross-link and reconcile (docs only, no code)

- Post this spec to Discussion #2375 per its own stated instruction.
- Cross-reference `SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md`'s
  checklist item (line 117) noting this tooling shipped as the manual
  complement, not a substitute, and that item remains open until the
  auto-expiry fix itself lands.

## 6. Open questions

1. **Should the Phase 1 snapshot push be throttled/debounced?** A pane with
   many rapid tool calls (a fast agent loop) could push frequently.
   **Recommend:** push only on status *transitions* (not per-render), which
   is already infrequent relative to render churn — revisit only if this
   proves measurably noisy in practice.
2. **Should `dock clear` require confirmation or be gated to a specific
   permission tier**, given it's `muxspect`'s first mutating command ever
   and changes visible UI state a user might be mid-look-at?
   **Recommend:** no special gate beyond normal `muxspect` auth (the same
   `AGENTMUX_AUTH_KEY` every other command already requires) — the blast
   radius is one dock entry's display status, not process/data state, and
   over-gating a narrow, reversible, visible-only action adds friction
   without a matched real risk.
3. **Does the Phase 1 in-memory cache need any size/TTL bound** for a
   long-running instance with many blocks created and destroyed over
   time? **Recommend:** cache is keyed by `block_id` and naturally bounded
   by how many blocks currently exist; evict on block deletion (an event
   `agentmux-srv` already observes) rather than a time-based sweep.
