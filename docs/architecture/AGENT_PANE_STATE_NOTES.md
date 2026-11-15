# Agent pane state: notes on fields, commands and events

**Status:** living: one entry per member, added or edited in the change that adds or changes the member.

The full reasoning behind members of [`frontend/app/store/agent-pane-state/types.ts`](../../frontend/app/store/agent-pane-state/types.ts), moved here from the doc comments in that file: why a field is cleared where it is, what a command must not touch, which race a timeout covers. In the code each of these members keeps a short summary with the rule a reader needs to change it safely; look the member up here by name for the rest. The state machine itself is in [`docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md`](../specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md).

## Types

Type docs.

### `AttachedTaskState`

Live "at least one agent-declared long-running task is attached" state, independent of
`turnPhase`: a sibling axis, not a `TurnPhase` variant, like `CompactionState` a
concurrent-but-orthogonal concern that must outlive the turn lifecycle
(docs/specs/SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md §2). Sourced from the dock's aggregated
`PinnedActivity` list (shell/subagent/tool adapters), not raw OS process counts (§4).

`since` is the ms the current unbroken "≥1 running" episode began; a second task starting doesn't
reset it.

### `ResumeRetryState`

Live "controller is recovering from a stale `--resume` session id" state, or `null`: set when the
backend's stale-resume retry begins (`mps.EVENT_AGENT_RESUME_RETRY`, `{status:"retrying"}`),
cleared once the outcome (Fresh or Resumed) is known (`{status:"resolved"}`). Like
`AttachedTaskState` it needs no staleness gating: both ends travel over the same reliable MPS
channel, unlike `CompactionState`'s completion signal
(docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md §6.2).

### `PaneFailure`

A classified backend failure (the `agentfailure` wave event's payload, `AgentFailure` in
`frontend/types/srv-types.d.ts`) surfaced for this pane: the single source of truth for "is there
an active failure" (docs/specs/SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md). It does
NOT carry the auto-retry countdown/budget or the expanded/retrying view flags: pure presentation
timing, hook-local in `useAgentFailure.ts`.

### `PaneFailure.turnAttempted`

Whether a turn was ever attempted before this failure: `true` for every backend-classified
failure; `false` only for the synthetic pre-launch auth failure the pane raises when the
mount-time launch bails with `auth_failed`, so there is nothing to re-run after a login.
Load-bearing: it selects `relogin`'s `retryAfterLogin`, and `true` for a never-started agent
makes a successful login silently re-send its last OLD message as a new turn. Defaults to
`true` when absent, so a failure persisted before this field keeps the old behaviour
(docs/specs/PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md §4).

### `modelEndedTurn`

The model has ended its turn (`stop_reason: end_turn`) but the turn
is still open. Absent/false means the model is still working:
generating, or waiting on a tool result. Set by `ModelEndedTurn`,
cleared by `ModelMessageStarted`; a new turn starts without it.
The busy predicate's background-work carve-out applies only when
this is true (working-indicator.ts `turnHeldOnlyByBackgroundWork`).

### `InitPhase`

Init lifecycle, the "history still loading" gap (#728 gap 1). The pane starts in `InitPending`;
the history load resolves to `InitReady` or `InitFailed` (with a diagnostic reason). One-way:
once terminal, `InitStart` is a no-op, and a re-mount makes a new pane slot back at
`InitPending`. A terminal variant carries the ms it was entered in its payload, so a transition
is one atomic write.

### `DisconnectReason`

Why the stream dropped while a turn was in flight. `stream-unsubscribed`: a clean
`StreamUnsubscribe` (unmount, PTY exit) tore down the local end. `transport-error`: reserved for
an in-flight transport failure; no caller wires it yet, but it is part of the API contract so the
dispatcher can adopt it without type churn.

### `CompactionState`

Live "compaction in progress" state: set when the `PreCompact` hook's `compaction_started` MPS
event lands (Tier 1, docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md), cleared
when the matching `compact_boundary` frame (a real `CompactionBoundary` command) arrives.
`startedAt` drives the live elapsed-time counter (Tier 2) near the pane status chip.

### `TurnPhase.Disconnected`

Stream dropped while a turn was in flight. `lastKind`: the working-kind we lost (the UI
says "was streaming" vs "was submitting"). `lastConnectedAt`: ms the disconnect command
was processed (`command.at` of `StreamUnsubscribe`), for a "disconnected 5s ago" label.
`reason`: see {@link DisconnectReason}.

## State fields

Fields of `AgentPaneState`.

### `turnTokens`

The current turn's live usage, from the stream, in the stream's own order: set by `TokensIn`,
cleared by that turn's `TurnEnd`, or by `StreamSessionStarted` when the process died before a
`result`. Nothing else clears it on a normal turn end: `ReconcileTurnActive`, the liveness
recovery and `TurnStart` leave it alone, because srv sends the turn-ended push before it
forwards the `result` line, so the turn's `session_end` (and sometimes earlier calls) still
follow, and a queued message or the next turn can start in that gap. Hard ends (disconnect,
failure, timeouts, reset) clear it.

### `turnLedger`

The turn as the user sees it, over however many CLI passes: srv's
`agentturn` event, verbatim (turn-ledger.ts). Null until srv has
published one for this block. The working row runs its clock and counter
from it; `turnPhase` and `turnTokens` stay per pass.

### `pendingCompactTurn`

True for the duration of a turn started specifically to send a manual "/compact" (the
composer's "Compact now" button, or typed): set on `TurnStart` when `command.content ===
"/compact"`, consumed and reset by `CompactionBoundary`. A manual
`CompactionBoundary.trigger` alone is NOT a safe signal that THIS pane's working turn is the
synthetic one: compaction_started/compact_boundary travel over transports with no ordering
guarantee against the primary NDJSON stream (the reducer's tests model a manual boundary
landing during an unrelated streaming turn), so the turn ends only when we know this pane's
own turn is the one the manual compact opened (#2659).

### `detailsOpen`

Whether the composer details panel (the Shell drawer holding the interactive terminal) is
open. Default `false`; a per-session ephemeral preference, reset on pane unmount (a new pane
gets `initialState()`) and not persisted to the backend.

It holds only the shell: the session banners moved to `AgentSessionNotices`,
Archive/Export/Restore to the `AgentSessionStats` popover and the runtime dropdowns to the
strip (SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §3). It no longer auto-collapses on
`TurnStart`, because the live shell (AgentShellSubblock) must survive sending more messages
(SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md §5.4).

### `context`

The context meter's reading: tokens in context, the model's window, and
where each came from — see store/agent-pane-state/context-reading.ts.
Set by `TokensIn` (live) and `ReconcileContextFromHistory` (the
restored transcript, only while `contextSeedable`). Unlike `turnTokens`
it is NOT cleared at TurnEnd, so the meter shows the last known fill
between turns. Cleared on TurnReset (session wipe), ContextInvalidated
(the conversation was replaced) and CompactionBoundary (it no longer
measures anything until the next call). Displays read it through
`plausibleReading`, never directly.

### `registryAttachedTaskSince`

Earliest `started_at_ms` among this block's currently-`Running` `db_background_tasks` rows
(docs/specs/SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md Phases A/B), or `null`.
A separate floor signal, not a duplicate of `attachedTask`: the durable registry can know a
task the live transcript doesn't (e.g. one that survived a session restart under a fresh
controller generation).

Owned EXCLUSIVELY by `RegistryAttachedTaskObserved`/`RegistryAttachedTaskCleared` (dispatched
only from `useBackgroundTaskRegistry.ts`), so the transcript-derived `attachedTask` effect
(`agent-view.tsx`) can never stomp on it; that effect READS this field and combines it with
its own value (registry ∪ transcript, earliest start wins) (#2685).

### `lastCompactionBoundaryAt`

Ms of the most recent REAL `CompactionBoundary` event (backend-sourced, exact, unlike the
≥50%-drop heuristic). The `TokensIn` heuristic checks it before firing its own synthetic
`context-compacted` event, so a session that just got a real boundary doesn't get a duplicate
for the next turn's token drop. Providers without a structured signal (codex/gemini/copilot)
never set it, so the heuristic is their only detection
(docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.3).

### `pendingCompactionPing`

A `compaction_started` ping the reducer rejected because `turnPhase` wasn't (yet) working,
but which a LATER authoritative signal could still promote for THIS turn:
`ReconcileTurnActive(active: true)` or `StreamFlushObserved`'s promotion arm, both of which
then apply it into `compacting`. Cleared on that promotion; when an authoritative signal
proves the turn is NOT running (`ReconcileTurnActive(active: false)`, even as a same-ref
no-op for `turnPhase`); by a fresh unrelated `TurnStart`; or by any other terminal transition
that clears `compacting`
(docs/specs/SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02.md).

### `turnCarry`

Each pass's own figures (its `TurnEnd` stats), summed over the turn srv
reported it in. For providers whose passes srv doesn't count (ACP, App
Server, one-shot subprocess: no `result` usage reaches the ledger), the
Worked line reports the turn from this.

## Commands

Commands of the reducer.

### `ReconcileTurnActive`

Reconciliation from `BlockControllerRuntimeStatus.turn_active` (backend-verified, from the
health monitor on the NDJSON stream, `crates/srv/src/backend/blockcontroller/health.rs`),
fetched via `GetControllerStatus` at mount and dispatched on every live `controllerstatus`
MPS event (useControllerStatusEvents). Bidirectional:
- `active: true` promotes the mount-default `Idle` to `Streaming` when the backend reports a
  turn in flight. It corrects only an `Idle` phase: a phase a real stream or user event
  already produced (TurnStart, StreamFlushObserved) wins (#2005 Finding 1).
- `active: false` demotes a stuck `Streaming` to `Idle`. The backend flips `turn_active`
  false only on the CLI's `result` event, so this is authoritative. It covers a missed
  `session_end` (pane backgrounded or unmounted across the transition, a dropped MPS event):
  the stuck-"Working" / stuck-"Queued" class. Only `Streaming` is demoted: `Submitting` is
  covered by `SUBMIT_TIMEOUT` (and is where the send race lives: a local `TurnStart` racing a
  backend that hasn't marked the turn active), `Interrupting` by `INTERRUPT_TIMEOUT`, and
  `Done` / `Idle` / `Disconnected` are already correct.

See docs/retro/retro-agent2-stuck-queued-message-2026-07-16.md.

### `ReconcileContextFromHistory`

Mount-time reconciliation: history replay found the main agent's last API call from before
this pane went live (`HistoryParser.lastContext`: that call's whole prompt and model, never a
`result`'s turn total). Seeds `context` so the meter shows a real number at once rather than
staying blank until the first live `TokensIn`. Seeds only while `contextSeedable` (no live
reading, invalidation, compaction or reset yet): a historical snapshot must never clobber
real-time data. An implausible seed is refused (`context-reading-rejected`). `tokens` is
absent when the restored window holds no main-agent call. `reportedWindows` (the history's
own `result.modelUsage` windows) merge UNDER any reported live. See
docs/plans/PLAN_PANE_REOPEN_SESSION_RESUME_AND_STATS_BAR_2026_07_10.md and
docs/reports/REPORT_AGENT_PANE_CONTEXT_METER_2026_10_05.md.

### `TurnStart`

User pressed send: the turn becomes active, and the previous turn's stale `sessionStats` are
cleared.

`content` is the literal text just submitted, threaded into
`TurnPhase.Submitting.pendingContent` (optional so the ~90 reducer/hook test call sites that
build this action without it still compile; the reducer defaults to `""`).
`useAgentActivitySummary.ts` uses it to derive the session-title Haiku call without a
FileStore read (docs/specs/SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md).

### `TurnStart.hidden`

True for a hidden memory-reinjection turn (`memory-reinjection-controller.ts`), never
set by an ordinary send. Propagated onto `TurnPhase.Submitting.hidden` so every ambient
side-effect hook keyed off `turnPhase` (`useAgentActivitySummary.ts`,
`useNextPromptSuggestion.ts`, any future one) can skip its own LLM call for the turn
with no separate wiring. Needed because `useAgentActivitySummary.ts` forwarded
`pendingContent` to an ambient summarization call whose result became the human-visible
pane title (#3502). Defense in depth: `content` for a hidden turn is also never the
real message (see memory-reinjection-controller.ts), so a consumer that forgets to
check `hidden` still sees only a safe placeholder.

### `TurnStartFailed`

Revert an OPTIMISTIC `TurnStart` when the turn never began: the send's own RPC failed
synchronously (no controller registered, the identity spawn gate blocked it, a network
rejection), or the input was a slash/bang command handled locally. Phase only: unlike
`TurnReset` it must NOT touch `sessionStats` / `sessionTotals` / `context`, which accumulate
over the pane's lifetime, so a transient send failure doesn't wipe the history of prior
completed turns (#2318). See `deliverToBackend` in `useAgentCommands.ts`.

### `InterruptTimeoutElapsed`

Interrupt timeout watchdog fired (caller-scheduled setTimeout, see the
`schedule-interrupt-timeout` event). Bounded `Interrupting → Done` force transition: if the
agent never acks the SIGINT (backend died, stream dropped) the pane would stay in
Interrupting and the working animation never settle. A no-op once the phase has left
Interrupting (agent acked first, user resumed). Spec §8.

### `SubmitTimeoutElapsed`

Submit timeout watchdog fired (caller-scheduled setTimeout, see the `schedule-submit-timeout`
event). Bounded `Submitting → Done.errored` force transition (#728 gap 2): if the backend
never acks a TurnStart with a chunk, token or tool event within `SUBMIT_TIMEOUT_MS`, the pane
would stay in Submitting with no `isWorking → false` signal. A no-op once the phase has left
Submitting (promoted to Streaming, user stopped, stream dropped). Spec §8.

### `PendingMessageFlushStarted`

flushHeldMessages (useAgentCommands.ts) is about to deliver this held message: dispatched
right before `deliverToBackend`, the moment sending actually begins, not merely when the turn
it was queued behind has ended (far apart when a controller refresh is deferred; see the
`PendingMessageFlushStarted` reducer arm). Marks the entry `flushing` so PendingMessagesPanel
stops claiming "sends at the agent's next step"
(docs/specs/SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md Phase 1).

### `FailureObserved`

Backend classified a failure for this pane (the `agentfailure` wave event). Force-ends a
working turn unconditionally (Submitting / Streaming / Interrupting → Done.errored), closing
the "stuck Waiting after a rate-limit interruption" bug: an authoritative failure no longer
depends on the CLI process exiting, which persistent-mode agents never do between turns
(SPEC_AGENT_PANE_UNIFIED_FAILURE_REDUCER_2026_07_06.md).

### `CompactionStarted`

The `PreCompact` hook fired: compaction has begun (the `compaction_started` MPS event, from
`agentmux-bashwrap precompact` via `useCompactionStream.ts`). Sets `compacting` for the live
"Compacting…" chip and elapsed counter (Tier 1/2). Also bumps `lastEventMs` like
`ProviderWaiting`, so the stuck-stream watchdog doesn't misfire during a long compaction with
no other stream activity.

### `CompactionBoundary`

The real `compact_boundary` frame arrived: compaction finished (the backend's
`AgentEvent::CompactionBoundary`, translated 1:1 from Claude Code's stream-json `system`
frame, so exact, not inferred). Clears `compacting`, records `lastCompactionBoundaryAt` (the
dedup guard for the `TokensIn` heuristic), and reconciles `context` to `postTokens` so the
context bar shows the real post-compaction size without waiting for the next `TokensIn`.

### `CompactionBoundary.frameTimestamp`

The `compact_boundary` frame's own `timestamp`, raw (not parsed or reformatted); absent
or `null` when the frame has none (the CLI's stdout frame has none) or the caller has
no raw frame (a test). A time value only: the boundary's completion time for the
stale-boundary checks and the node's time, falling back to `at` / receipt time. Never
an id: see `boundaryUuid`.

### `ContextInvalidated`

The conversation the reading measured is gone — a `fresh` session
replaced it, or the transcript was archived or restored. Clears
`context` (and ends `contextSeedable`), so the next call's reading isn't
read as a compaction by the `TokensIn` heuristic and a late history
seed can't bring the old conversation back.

### `ContextModelSwitched`

The pane's model setting changed (`/model`, the model menu). The
conversation, and so the reading's tokens, carry over; the window is
the new model's, known only when its first reply reports it, so the
reading's window becomes unknown until then. `model` is the setting's
value (often a family alias, e.g. `sonnet`, which doesn't say its
window).

### `AttachedTaskObserved`

Fires on the 0→1 transition of "≥1 PinnedActivity is `running` for
this block." No-op (idempotent) if an episode is already in
progress — must NOT reset `since` when a second task starts running
alongside an already-running one.

### `ResumeRetryStarted`

`mps.EVENT_AGENT_RESUME_RETRY`'s `{status:"retrying"}` landed: a stale `--resume` was
detected and a retry is about to fire. Idempotent, like `AttachedTaskObserved`: a cascaded
second retry must not push `startedAt` later.

## Events

Events the reducer emits.

### `stream-disconnected`

The stream tore down while a turn was in flight (Submitting / Streaming / Interrupting).
Emitted alongside the generic `stream-unsubscribed`, only on the working-set → Disconnected
transition (not on idle unsubscribes), so the view can drive a "Disconnected — reconnecting…"
banner without re-deriving from `turnPhase.kind`.

### `stream-stuck.recoverThresholdMs`

The bar that would force this turn back to Idle: always larger than `thresholdMs`, and
larger still under a rate-limit backoff. Reported because a diagnostic showing only the
warning bar reads as "past the threshold and still stuck", which is how a real 5h stall
was misread.

### `working-recovered`

The watchdog force-recovered a hung `Streaming` turn to `Idle` after `idleSinceMs` ≥
`LIVENESS_RECOVERY_MS` with no tool active and no rate-limit wait: its terminal
`session_end` was never observed (the persistent-mode stuck-"Working" class). For
telemetry; the phase change is the real effect
(SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md).

### `turn-active-reconciled`

`ReconcileTurnActive` promoted the phase to `Streaming` because the backend reported a turn
already in flight, from the mount-default `Idle` or from a settled `Done.completed` episode
(the focus-triggered reconcile's missed-live-turn-start case). For diagnostics: distinguishes
this from a user-initiated `turn-started`.

### `turn-inactive-reconciled`

`ReconcileTurnActive` demoted a stuck `Streaming` phase to `Idle` because the backend's
authoritative `turn_active` reported the turn over (the `result` event fired) while the
frontend never saw `session_end`: the stuck-"Working" / stuck-"Queued" class. Completes
#2005's symmetry, which only promoted Idle→Streaming. For telemetry; the phase change is the
real effect (docs/retro/retro-agent2-stuck-queued-message-2026-07-16.md).

### `context-compacted`

`source: "real"`: an authoritative `CompactionBoundary` event (`trigger` / `durationMs`
present). `source: "heuristic"`: inferred from a ≥50% drop in `TokensIn`, the cross-provider
fallback (`trigger` / `durationMs` absent). See
`docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` §4.3.

### `schedule-interrupt-timeout`

Reducer signal: a turn has just entered Interrupting. The dispatch layer starts a setTimeout
for `INTERRUPT_TIMEOUT_MS` and fires `InterruptTimeoutElapsed` on expiry, gated by a shared
cancel flag (like an `Arc<AtomicBool>`) so a later `RequestStop` doesn't double-fire.
`deadlineMs` is `at + INTERRUPT_TIMEOUT_MS`, so the caller can compute the remaining slice
deterministically. Spec §8.

### `schedule-submit-timeout`

Reducer signal: a turn has just entered Submitting. The dispatch layer starts a setTimeout
for `SUBMIT_TIMEOUT_MS` and fires `SubmitTimeoutElapsed` on expiry, gated by a shared cancel
flag so a promotion to Streaming (`StreamFlushObserved`, or `bumpEvent` from tool / token
activity) cancels the timer first. `deadlineMs` is `at + SUBMIT_TIMEOUT_MS`. Spec §8, #728
gap 2.

### `turn-ended`

A turn finished and the phase became `Done`. Carries `outcome` directly (the reducer
already computes it) so consumers (notification sounds, telemetry, future UI) needn't
snapshot the slot to read `state.turnPhase.outcome`
(SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §3.2).

### `failure-observed`

A failure was observed and recorded on `state.failure`. `turnWasEnded`
is true iff the pane's turn was actually working (Submitting /
Streaming / Interrupting) at the time — surfaced for diagnostics
(e.g. distinguishing a genuine mid-turn failure from a stray late
event arriving after the pane was already idle).

### `waiting-for-input`

Emitted when the pane enters the waiting-for-input state: turn
completed with a question, composer empty. The sound service
starts the looping ambient tone on this event.
Spec: SPEC_AGENT_WAITING_AMBIENT_SOUND_2026_06_19.md §6.3.

## Timeouts and thresholds

Constants.

### `LIVENESS_RECOVERY_MS`

Liveness-recovery threshold: a `Streaming` turn with no observable activity for this many ms, no
tool running and not rate-limited, is treated as hung: its terminal `session_end` was never
observed and nothing else will clear it. A persistent-mode agent's process never exits between
turns, so `ControllerStatus: done` never fires and the process-exit grace timer can't recover the
phase; the watchdog itself moves out of "Working" (→ `Idle`). 3 min is well past
`STUCK_THRESHOLD_MS` (45s), so a merely quiet reasoning run is never mistaken for a hang
(SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md).

### `INTERRUPT_TIMEOUT_MS`

Bounded `Interrupting → Done.{interrupted}` window. After `RequestStop` fires SIGINT, if neither
`TurnEnd` nor `StreamUnsubscribe` lands within this many ms, the reducer force-transitions to
Done.interrupted on `InterruptTimeoutElapsed`. 5 s is generous for a healthy agent's sub-second
SIGINT ack but tight enough that a dead agent doesn't pin the working animation forever.
SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §8 suggests 3_000; 5_000 leaves headroom for slow
Windows PTY teardowns and remote agents on flaky networks. Tune if telemetry shows the watchdog
firing on live agents.

### `SUBMIT_TIMEOUT_MS`

Bounded `Submitting → Done.{errored}` window. After `TurnStart` puts a turn into Submitting, if
no stream activity (flush, or bumpEvent from tool / token) lands within this many ms, the reducer
force-transitions to Done.errored on `SubmitTimeoutElapsed`. 30 s is the generous outer bound for
a backend ack (network handshake, agent spawn, the first model token on a fresh session); longer
pins the working animation past the user's patience, and the streaming watchdog covers the longer
"agent is thinking" idle case once in Streaming.

Spec §8 / #728 gap 2. The dispatcher arms it from the `schedule-submit-timeout` event emitted
when `TurnStart` promotes the phase into Submitting; the same cancel-flag pattern as the
interrupt timeout gates a stale fire against a promotion to Streaming.

### `COMPACTION_HEURISTIC_SUPPRESS_MS`

Suppression window for the `TokensIn` ≥50%-drop compaction heuristic after a REAL
`CompactionBoundary`. Claude Code's `compact_boundary` frame precedes the next turn's
`message_start` in the same stream, so `context` is normally already reconciled to `postTokens`
before the heuristic's next check. The window is a backstop against an ordering surprise or a
`postTokens` that doesn't closely match the next `TokensIn.input`. 2 minutes covers one turn's
round trip without suppressing a LATER, genuinely new compaction
(`docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md` §4.3).
