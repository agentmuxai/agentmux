# SPEC: Align pane scrollback with actual model context, and make cross-instance opens honest

**Date:** 2026-08-05
**Status:** active — Part A (session-outcome transcript event) shipped in PR #2426, and extended 2026-08-27 to cover the no-`--resume`-attempted case (§2.1's superseded callout). Parts B (rehydrate-before-resume) and C (session_id backfill) still not started — `rehydrate_claude_session` exists only in `scripts/import-agents.sh`, not in `agentmux-srv`. Verified against `main` 2026-08-27. See also SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md, which builds on Part A.
**Severity:** Medium — no data loss, but a real correctness gap: the pane can
imply the agent remembers a conversation it does not, with no visible signal
that anything went wrong.
**Trigger:** Follow-up to `audits/AUDIT_AGENT_PANE_HISTORY_2026_08_05.md` (an
agent-pane-history audit written the same day), which found that "what the pane
displays" and "what the model actually remembers" are two independently-loaded
stores that can silently disagree.

---

## 0. Ask

> "we want them to align as close as possible, is that possible? We also want
> agents that open in a new version or new instance of agentmux retain whatever
> is there, or at least loads empty if the memory is empty ... that is just in
> the agent pane though, the actual data needs to be saved and organized"

Three asks, in order of what this doc actually solves:

1. **Align pane scrollback with model context "as close as possible."** Not
   fully possible — the provider CLI's own context window/compaction is opaque
   to AgentMux (see the audit, F2). What *is* possible, and what Part A of this
   doc implements: never let the two *disagree silently*. Every time a
   `--resume` attempt's outcome becomes known, record it as an explicit,
   persisted transcript event, so the scrollback the human reads always
   honestly reflects whether the model actually continued or started over.
2. **Cross-instance/cross-version opens retain history if it exists, or load
   genuinely empty if it doesn't** — never a stale/misleading in-between. This
   is Part B (§6.2): rehydrate the provider's isolated session file from the
   already-existing global transcript zone *before* attempting `--resume`, so
   resume either genuinely works or is honestly skipped.
3. **The actual data needs to be saved and organized.** Already substantially
   true (see §1) — the global per-agent transcript zone
   (`agent:<definitionId>:current`) built in
   `docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md`
   steps 1–4 is exactly this. What's missing is (a) the resume-outcome record
   from Part A, and (b) the backfill/rehydrate step that doc's own §9 deferred
   (Part B, C below).

---

## 1. What already exists (confirmed by reading the current code, not assumed)

- **Global per-agent transcript store.** `agent:<definitionId>:current` is a
  `FileStore` zone in a global (not per-channel) store, mirrored from every
  channel's local blockfile on every append
  (`agentmux-srv/src/backend/blockcontroller/shell.rs`, `resolve_global_output_zone`).
  This is the "saved and organized" piece the ask calls for — it already exists
  and is already the read-fallback for cross-channel opens
  (`agentmux-srv/src/server/app_api/blockfile.rs`).
- **The pane loads the tail, not everything, on mount** — 200 lines, paged
  further back on scroll (`frontend/app/view/agent/hooks/useHistoryPagination.ts`).
  This is a UI/virtualization bound, unrelated to model context, and out of
  scope here — it's already correct for its purpose (instant open on very long
  sessions).
- **Model context is resumed via `--resume <session_id>`**, a value AgentMux
  persists as opaque block meta (`agent:sessionid`) and hands to the provider
  CLI on every spawn/respawn (`agentmux-srv/src/server/app_api/agent_io.rs`).
  AgentMux does not read or reason about the CLI's actual transcript content.
- **A dedicated state machine already tracks resume outcome internally.**
  `agentmux-srv/src/backend/blockcontroller/persistent_resume.rs` — built after
  issue #2368 and a live recurrence (agent "Marks", 2026-07-30) — is a pure
  `(state, event) -> (state, effects)` machine that already knows, precisely and
  per-generation, whether a `--resume` attempt (a) succeeded, (b) was confirmed
  unreachable and is retrying fresh, or (c) never resolved. **This state is
  computed today and then only logged** (`tracing::info!`) — never persisted to
  the transcript, never shown to the user. Part A of this doc is almost entirely
  "surface state that's already being computed."
- **There is exact precedent for a synthetic, non-CLI-native transcript event
  rendered as a distinct divider node**: `compact_boundary` /
  `ContextCompactedNode`
  (`docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`,
  `frontend/app/view/agent/compact-boundary.ts`). Both the live path
  (`useAgentStream.ts`) and the history-replay path (`parseHistoryLines.ts`)
  intercept a raw `{"type":"system","subtype":"compact_boundary",...}` frame
  directly (bypassing the generic provider `Translator`, which has no shape for
  it) and construct a node with a **content-derived stable id** so the same
  event seen live and via a later history page merges into one node instead of
  duplicating. Part A reuses this exact shape and this exact dual-interception
  pattern for a new frame type.

---

## 2. Part A — explicit "session outcome" event (implemented in this PR)

### 2.1 Backend: emit the event where the outcome is already decided

`persistent_resume.rs`'s `update()` function has exactly two places where a
resume attempt's fate becomes unambiguously known — both already produce
`ResumeEffect`s today, just not this one:

1. **`SessionCaptured` resolving a tracked generation** (the `AwaitingOutcome`
   and `ConfirmedRetry` arms, both guarded by `sid != attempted_sid ||
   is_confirmed_success`): `sid == attempted_sid && is_confirmed_success` means
   the CLI genuinely continued the requested session — **Resumed**. `sid !=
   attempted_sid` means the CLI itself silently rolled to a different session —
   **Fresh**, even though no retry was ever fired.
2. **`ConfirmedRetry` + `ProcessExited` (not `stop_requested`)** — the
   `FireRetry` arm. `ResumeUnreachable` already confirmed the attempted session
   is dead; the caller is about to relaunch with `session_id` cleared
   (`retry_after_resume_failure` sets `config.session_id = String::new()`).
   Unambiguously **Fresh**.

Added, purely additively (no existing arm's condition or return value changes,
only appends to the effects `Vec` already being built):

```rust
#[derive(Debug, Clone, PartialEq)]
pub(super) enum SessionOutcome {
    /// The CLI continued the exact session `--resume` was given.
    Resumed,
    /// The CLI could not continue that session — a new one was (or is about
    /// to be) started. The model has none of the prior turns.
    Fresh,
}
```

```rust
/// A resume attempt's outcome became definitively known. Surfaced as a
/// persisted transcript event (§2.1 of
/// SPEC_AGENT_PANE_HISTORY_ALIGNMENT_2026_08_05.md), not just a trace log
/// line, so the pane's scrollback can never silently disagree with what the
/// model actually has in context.
EmitSessionOutcome { outcome: SessionOutcome, attempted_sid: String, actual_sid: Option<String> },
```

`persistent.rs`'s three existing `ResumeEffect` match sites (the stdout
`SessionCaptured` handler, the process-exit handler, and the stop-path handler)
each get one new arm, mirroring `flush_error_line_now`'s existing
`handle_append_block_file` call — appends to both the per-channel blockfile and
the global mirror zone, so the event survives cross-channel/cross-instance
exactly like every other transcript line already does:

```rust
fn emit_session_outcome_line(&self, outcome: SessionOutcome, attempted_sid: String, actual_sid: Option<String>) {
    let Some(ref broker) = self.broker else { return };
    let line = serde_json::json!({
        "type": "system",
        "subtype": "agentmux_session_outcome",
        "outcome": match outcome { SessionOutcome::Resumed => "resumed", SessionOutcome::Fresh => "fresh" },
        "attempted_sid": attempted_sid,
        "actual_sid": actual_sid,
        "timestamp": chrono_like_rfc3339_now(),
    }).to_string() + "\n";
    let global_output_zone = super::shell::resolve_global_output_zone(&self.wstore, &self.block_id);
    super::shell::handle_append_block_file(
        broker, &self.block_id, PERSISTENT_OUTPUT_SUBJECT, line.as_bytes(),
        self.filestore.as_ref(), global_output_zone.as_deref(),
    );
}
```

`type: "system"` with a project-specific `subtype` mirrors `compact_boundary`
exactly — a frame shape the provider CLI itself would never emit (no collision
risk) that both consumers already know how to special-case.

**Deliberately not touched (in the original PR — superseded 2026-08-27, see
below):** the `SpawnedFresh` transition (spawn-time classification of "no
resume attempted this generation"). As documented in
`persistent.rs:1918-1932`, that classification also fires for a truly-first-ever
turn (no session existed to lose) and for internal respawns with no immediate
message — neither is a "the model lost its memory" event, and disambiguating
them correctly from inside `spawn_process` would touch code with a long history
of subtle races (issue #2368, PR #2373 rounds 4/5/7/9). Out of scope for this
PR; not needed for the two outcome points above, which already cover every case
where AgentMux *positively knows* what happened to a resume attempt.

> **Superseded (2026-08-27).** The "no session existed to lose" reasoning has
> one real exception, which
> `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`
> then observed live: a long-lived named agent opened in a **fresh channel**
> whose shared-registry pointer is empty gets no `--resume` at all, so the CLI
> never errors and no outcome is ever emitted — while the pane renders the
> entire prior conversation through `blockfile.rs`'s cross-channel read
> fallback from the global transcript zone. There *is* something to lose in
> that case, and it was the one case with no signal of any kind.
>
> `spawn_process` now emits `Fresh` (with an empty `attempted_sid` — there was
> no id to attempt) when all three hold: no `--resume` was attached, this is
> the controller's **first** generation, and a transcript already exists
> locally or in the agent's global zone. The generation gate is what keeps the
> races above out of scope — later generations that spawn without `--resume`
> either emit their own outcome (`retry_after_resume_failure`) or are
> respawns of an already-classified session. `persistent_resume::update`'s
> `SpawnedFresh` arm is still effect-free and its regression guards (§8) still
> hold; the decision lives in `persistent.rs`'s `fresh_start_needs_disclosure`
> because "does prior history exist" is a FileStore question the pure state
> machine can't answer.

### 2.2 Frontend: render it as a distinct divider, not silently

New file `frontend/app/view/agent/session-outcome.ts`, structurally identical to
`compact-boundary.ts`:

```ts
export interface SessionOutcomeData {
    outcome: "resumed" | "fresh";
    attemptedSid: string;
    actualSid: string | null;
    frameTimestamp: string | null;
}

export function parseSessionOutcomeFrame(rawEvent: unknown): SessionOutcomeData | null { ... }

export function sessionOutcomeNodeId(data: SessionOutcomeData): string {
    // content-derived, same rationale as contextCompactedNodeId: a live-seen
    // event and the same event re-seen via history-page overlap must land on
    // one id, not two.
    const suffix = data.frameTimestamp ?? `notime-${data.outcome}-${data.attemptedSid}`;
    return `session-outcome-${suffix}`;
}
```

`types.ts` gains:

```ts
export interface SessionOutcomeNode {
    type: "session_outcome";
    id: string;
    outcome: "resumed" | "fresh";
    attemptedSid: string;
    actualSid: string | null;
    timestamp: number;
}
```
— added to the `DocumentNode` union.

**Interception, not `dispatchPane`.** `compact_boundary` round-trips through
`model.dispatchPane({ type: "CompactionBoundary", ... })` because the pane-state
machine also drives the live token-meter reset on compaction. The session-outcome
event has no such side effect — it is purely a transcript marker — so both
`useAgentStream.ts` (live) and `parseHistoryLines.ts` (replay) construct and
push the `SessionOutcomeNode` directly (`queue.pushNewNode(...)` /
`nodes.push(...)` with the same `indexById`-based dedup `parseHistoryLines.ts`
already uses for `context_compacted`), skipping the pane-dispatch layer
entirely. This keeps the change confined to the same two files
`compact_boundary` already touches for parsing, plus the render/size/expansion
wiring below — no new pane-state action type.

Render wiring (mirrors `context_compacted` at each site):
- `DocumentRow.tsx` — a `<Show when={... === "session_outcome"}>` divider row:
  *"Session continued"* (green/neutral) for `resumed`, *"New session started —
  prior conversation is not available to this agent"* (amber) for `fresh`.
- `renderers.ts` — non-expandable, fixed height (mirrors `context_compacted: 48`
  in the three switch statements there).
- `expansion-source.ts` — same `case "context_compacted":` treatment (not
  expandable).

### 2.3 Why this is the right (and only newly-needed) alignment mechanism

This does not make the pane's scrollback and the model's context *identical* —
that's impossible without AgentMux reimplementing the provider's own context
management (audit F2, explicitly a non-goal). What it does is close the gap
that actually matters: **the human can no longer be shown continuous-looking
scrollback while the model silently has none of it.** Every resume boundary is
now a first-class, persisted, honestly-labeled event in the same stream as
everything else — exactly the "align as close as possible" the ask asked for,
scoped to what's actually knowable from inside AgentMux.

---

## 3. Non-goals (this PR)

- Reimplementing or introspecting the provider CLI's own context
  window/compaction/token accounting. Out of reach from this codebase (audit
  F2); `compact_boundary`/`ContextCompactedNode` already covers the one piece
  of that the CLI *does* tell us about.
- Changing the 200-line pane page size or the ring-buffer fallback cap — both
  already correct for their purpose (audit §3).
- The coordination/activity history log (`appendagenthistory` et al., audit
  §4) — a separate feature, not a conversation store.

---

## 4. Scope and blast radius

- **Backend:** `agentmux-srv/src/backend/blockcontroller/persistent_resume.rs`
  (new enum + effect variant, additive changes to 3 match arms, new unit
  tests), `agentmux-srv/src/backend/blockcontroller/persistent.rs` (new
  `emit_session_outcome_line` helper + 3 new match arms at existing
  `ResumeEffect` consumption sites).
- **Frontend:** new `frontend/app/view/agent/session-outcome.ts`;
  `types.ts`, `useAgentStream.ts`, `parseHistoryLines.ts`,
  `virtualization/DocumentRow.tsx`, `virtualization/renderers.ts`,
  `virtualization/expansion-source.ts` — each gets the same small, additive
  case `context_compacted` already has, not a new subsystem.
- Every change is additive: no existing `ResumeState`/`ResumeEffect` variant,
  match arm condition, or return value is altered. Existing behavior (what
  gets persisted, when a retry fires, when `PublishDone` happens) is byte-for-
  byte unchanged; this PR only adds a new effect alongside effects that were
  already being produced.
- **Only the persistent-controller (Claude, and any future simple-flag-resume
  provider) path is covered** — the interactive agent pane's primary path
  (`providers.rs`: "Claude runs on the persistent controller"). The headless
  one-shot drone `run_agent` path (`agents/runner.rs`) and the
  `SubprocessController`/container-exec path do not share
  `persistent_resume.rs`; extending this to them is a natural follow-up but is
  not needed for the interactive-pane case this doc (and the originating
  audit) is about.

---

## 5. Testing

- `persistent_resume.rs`: new unit tests alongside the existing ~30, following
  the same style (pure `update()` calls, no process/tokio needed) —
  `session_captured_with_different_sid_emits_fresh_outcome`,
  `session_captured_confirmed_success_emits_resumed_outcome`,
  `fire_retry_emits_fresh_outcome`, plus a check that
  `EmitSessionOutcome` never appears for the untouched `SpawnedFresh`/never-
  confirmed paths (regression guard for §2.1's "deliberately not touched").
- Frontend: `session-outcome.test.ts` (parse + id-stability, mirroring
  `compact-boundary` tests already in the repo for the same shape), plus
  extending `parseHistoryLines.test.ts` and `useAgentStream`'s existing test
  coverage with one fixture each (live + replay produce the same node id for
  the same frame).

---

## 6. Deferred — Parts B and C (cross-instance retention)

Not implemented in this PR; scoped here so the remaining ask ("agents that open
in a new version or instance retain whatever is there, or load empty if truly
empty") has a concrete next step rather than staying an open question.

### 6.1 Why this is still open

`docs/analysis/ANALYSIS_CROSS_CHANNEL_CONVERSATION_HISTORY_2026_06_14.md` §9
already shipped steps 1–4 (global transcript zone, hot-path mirror, read
fallback, snapshot overlay) — the pane-display half of "retain whatever is
there" is done. Its own §6 and §9 "step 5" flag what's still missing for the
*model-continuation* half:

- The 9 originally-migrated agents have `session_id = null` — `--resume` has no
  id to attempt for them at all.
- Even where a `session_id` is known, AgentMux runs the CLI with
  `CLAUDE_CONFIG_DIR` pointed at an **isolated** per-agent home
  (`~/.agentmux/shared/providers/claude`); `--resume` only searches inside
  that isolated home. A session captured (or migrated) from a *different*
  instance/version's isolated home — or from the global `~/.claude` — is
  invisible to it, so `--resume` fails even though the transcript genuinely
  exists in the global transcript zone (§1) or on disk elsewhere.

### 6.2 Part B — rehydrate-before-resume

Before `agent_io.rs`/`input.rs` attach `--resume <persisted_session_id>` to a
spawn, check whether that session's `.jsonl` already exists inside the current
instance's isolated `CLAUDE_CONFIG_DIR`. If not, look it up (by `session_id`,
recoverable from the global transcript zone's own record of which sid produced
it — the live mirror already captures this for new agents, per the analysis
doc §6) and copy it into the isolated home before spawning — reusing the
`rehydrate_claude_session` shape already prototyped for the import path
(`import-agents.sh:107-120`, cited in the analysis doc). If no transcript can
be found anywhere (genuinely nothing to rehydrate), spawn without `--resume` —
this is the "load empty if the memory is empty" half of the ask.

**Correction (2026-08-27):** this paragraph originally continued "...and with
Part A already landed, that fresh-start is now an honest, visibly-labeled event
instead of an unexplained blank pane." That was **not** true of Part A as
shipped — §2.1 explicitly exempted the no-`--resume`-attempted spawn, and
`persistent_resume.rs`'s own regression guard
(`spawned_fresh_and_never_confirmed_paths_never_emit_a_session_outcome`) pinned
the opposite. It became true on 2026-08-27 via §2.1's superseded callout, which
labels exactly this case. Part B still has to hold up its own end: the label is
honest, but it only *describes* a lost conversation — it doesn't recover one.

### 6.3 Part C — backfill the session_id-less migrated agents

The analysis doc's deferred step 5: recover `session_id` for the 9 (or by then,
however many) agents that migrated without one, from the transcript filename
(`ClaudeHistoryAdapter` already does this lookup, per that doc's §5 evidence
table), and run it once, the same shape as the existing `import-agents.sh`
one-shot.

### 6.4 Suggested order

Part B alone is the correct next PR — it's the piece that turns "history is
visible" (already shipped) into "history is *resumable*" for any agent that
already has a captured `session_id`, and directly serves the ask's "retain
whatever is there" for the common case (an agent moving between instances/
versions, not the smaller backfill-only migrated set). Part C is a narrower
one-shot cleanup for the already-identified 9 (or current count) legacy
records and can follow independently.
