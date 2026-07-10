# Plan: fix session resume on pane reopen + stale/wrong stats bar

**Date:** 2026-07-10
**Author:** Agent1
**Status:** Proposed, not started
**Triggered by:** user report — closed AgentMux, reopened, opened an existing
Agent1 pane, saw its historical conversation rendered, but the actual spawned
CLI process was a fresh session with no memory of it, and the token/context
stats bar above the composer showed wrong numbers.

## Relationship to prior work

This is not a new bug class. It's the unresolved half of
`docs/retro/retro-cross-channel-conversation-continuity-regression-2026-06-16.md`
("Mechanism 2 — continuity"), whose action items 2–4 and 6 were never
implemented, plus a second, previously undocumented symptom (the stats bar)
that shares the same root shape: **reopened-pane UI state is not reconciled
against backend/session truth at mount** — the same pattern
`docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md` names for
`TurnPhase`.

## Problem statement

1. **Session continuity:** on reopen, whether the next CLI spawn resumes the
   original provider session depends entirely on `agent:sessionid` block meta
   being resolvable at spawn time. If the reopen doesn't land on the exact
   block that meta lives on (cross-channel open, picker reattach, or any path
   the retro didn't cover), the lookup misses, a **new** session is spawned,
   and that new session id then gets persisted — silently orphaning the
   original conversation going forward.
2. **Stats bar:** `contextTokensAtom`/`contextWindowAtom` are populated only
   by a *live* `session_end` event from the currently-running process
   (`useAgentStream.ts:415-469`, `claude-translator.ts:87-104`). Historical
   transcript replay does parse old `session_end` frames
   (`parseHistoryLines.ts:57-73`) but discards the stats payload. So on
   reopen the bar is blank/stale until the new (usually much smaller) session
   completes its first turn — at which point it shows numbers for the wrong
   session, not the resumed conversation.

Both stem from the same gap: nothing reconciles what the pane displays with
what the backend/session actually is at mount time.

## Goals

- A reopened pane resumes the original provider session whenever one exists,
  regardless of which channel/block the reopen lands on.
- The stats bar reflects the true current session's usage immediately on
  reopen — either the resumed session's last known stats, or an explicit
  "unknown until first turn" state — never a stale number from a different
  session presented as current.
- Existing agents whose session got silently orphaned by this bug can be
  recovered (pointed back at their real last session) where safe.

## Non-goals

- Rewriting `TurnPhase` reconciliation (Finding 1 of the 07-07 report) — real,
  related, but a separate architecture decision with its own open questions.
  Do not conflate this plan with that one.
- Fixing subagent completion detection or ambient-call concurrency (Findings
  2–3 of the same report) — unrelated subsystems.

## Root cause references

- Resume flag / controller type: `agentmux-srv/src/backend/providers.rs:115,152-153`
- Local session id storage: `agentmux-srv/src/backend/blockcontroller/core.rs:21` (`META_SESSION_ID`), `:126-173` (`persist_session_id`)
- Lazy spawn (no resume attempt on mere reopen): `agentmux-srv/src/backend/blockcontroller/persistent.rs:1029-1040`
- Spawn-time resume hydration: `persistent.rs:566-598`
- Per-turn re-read of block meta: `agentmux-srv/src/server/agent_handlers/input.rs:256-269`, `agentmux-srv/src/server/app_api/agent_io.rs:189-201`
- Cross-channel path with no meta → fresh controller: `agent_open.rs:47-70`, `blockcontroller/mod.rs:347-463` (`resync_controller`)
- Registry-level `session_id` field declared but never written in production: `agentmux-srv/src/registry/schema.rs` (only in tests, per retro)
- Stats bar component: `frontend/app/view/agent/components/AgentComposerStrip.tsx`, wired at `frontend/app/view/agent/agent-view.tsx:1080-1096`
- Stats atoms, live-only population: `frontend/app/view/agent/state.ts:69,173`; `frontend/app/view/agent/hooks/useAgentStream.ts:415-469,841`; `frontend/app/view/agent/claude-translator.ts:87-104`
- Historical stats dropped during replay: `frontend/app/view/agent/parseHistoryLines.ts:57-73`

## Proposed changes

### Phase 1 — session continuity (closes retro action items 2, 3, 4, 6)

1. **Populate the global/registry `session_id` on every turn**, not just the
   block-local meta. Write the CLI-emitted authoritative session id into the
   registry record alongside the existing block-meta write in
   `persist_session_id` (`core.rs:126-173`).
2. **Read the registry session id into spawn config whenever block-local meta
   is absent**, so a cross-channel/cold reopen's *first* turn still
   `--resume`s the right session instead of starting fresh. Wire this into
   the same site(s) that currently read only block meta
   (`input.rs:256-269`, `agent_io.rs:189-201`).
3. **Add a resume guard**: before `resync_controller`/spawn is allowed to
   proceed with `session_id = None`, check whether the registry has a known
   last session for this agent. If yes, resume it; only fall back to a fresh
   session if genuinely none exists. This directly implements action item 4
   from the retro ("a reopen with an existing transcript must resume before
   starting a new session").
4. **Backfill migration** for already-affected agents: for each registry
   record with `session_id: null` but a discoverable provider transcript
   (via `history/claude_adapter.rs` discovery), set `session_id` to the
   newest **original** session — explicitly excluding any short/orphaning
   session created after this bug started, per retro guidance. Needs a
   per-agent judgment call where multiple candidate sessions exist; do not
   auto-run destructively — write a dry-run report first, apply only after
   review.
5. **e2e test**: run live in channel/build A, reopen in a fresh channel/build
   B, assert (a) the same conversation renders, and (b) the CLI is invoked
   with `--resume <original sid>` on the first post-reopen turn. This is the
   exact coverage gap the retro calls out as missing.

### Phase 2 — stats bar hydration

1. In `parseHistoryLines.ts`, stop discarding the `stats` payload from
   historical `session_end` frames — surface the **last** one found during
   replay alongside the parsed `DocumentNode`s.
2. At pane mount, once history replay completes, dispatch that last-known
   stats into `contextTokensAtom`/`contextWindowAtom` (and
   `sessionStatsAtom` if present) so the bar shows real numbers immediately,
   before any new turn runs.
3. If Phase 1's resume succeeds, the first new live `session_end` will
   naturally overwrite this with fresh numbers, continuing the same
   session's running totals — no separate reconciliation needed once resume
   works correctly.
4. If no historical stats can be found (never-run agent, corrupted
   transcript), leave the bar in its current blank/placeholder state rather
   than showing a misleading zero.

## Risks / edge cases

- Multiple provider sessions may exist for one agent (e.g. from repeated
  instances of this bug). Backfill must pick deterministically (newest by
  file mtime/session start time) and log what it skipped, not silently guess.
- Registry writes must not race with the existing block-meta write — same
  turn, two locations; keep them in the same transaction/call site to avoid
  a torn state where one updates and the other doesn't.
- A pane that was never live (agent created but never sent a message) has no
  session to resume — guard must fall through to "start fresh" cleanly, not
  error.
- Stats hydration must clearly be presented as "as of last session end,"
  since a resumed session's *next* turn may report a different (larger)
  cumulative total — avoid a UI flash/jump that looks like a bug in itself.

## Testing plan

- Unit: registry session id read/write round-trip; resume-guard decision
  table (meta present / registry present / neither).
- Backfill: dry-run against a snapshot of real affected registry records
  (Naki et al., per the retro's evidence section) — verify it selects the
  original long session, not the short orphaning one.
- e2e (per item 5 above): live-in-A → reopen-in-B → same content + real
  `--resume`.
- Manual: reproduce the user's exact repro (close app, reopen, open Agent1
  pane) and confirm both symptoms are gone.

## Rollout

- Phase 1 and Phase 2 are independent and can ship as separate PRs/changesets.
- Backfill migration should be reviewed manually before running against real
  user data — do not wire it to run automatically on startup without an
  explicit opt-in, given the "pending user OK" caveat already noted in the
  retro for recovering already-affected agents.
