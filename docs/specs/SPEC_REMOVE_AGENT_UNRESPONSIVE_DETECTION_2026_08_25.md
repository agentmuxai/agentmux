# SPEC: Agent-pane status cleanup — remove "unresponsive" detection, consolidate Reconnecting/Compacting/Working

**Date:** 2026-08-25
**Status:** Both parts shipped and merged to main (2026-08-27).
- **Part 1 (removal):** PR #2825 (merged) — includes a fixup commit
  restoring container-exec failure classification, a real regression Codex
  caught in review (container-backed agents had silently lost their only
  route to in-band failure classification; `host_spawn.rs`'s own
  independent completion-time classifier was never mirrored in
  `container_spawn.rs`) — see that PR for detail.
- **Part 2 (status consolidation):** PR #2826 (merged) — implemented per
  §10's design recommendation. Review caught one real bug (reagent and
  Codex both independently found it): the working-row backdrop's
  `--loading` modifier didn't account for the two relocated sub-states,
  reintroducing the exact backdrop/row color mismatch
  `SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md` fixed for the plain
  loading case — fixed in a follow-up commit before merge.

**Outstanding (not blocking, tracked here for whoever picks these up):**
- Manual verification items in §6/§11's test plans (Swarm pane badge during
  a long tool call, `muxspect describe` lifecycle accuracy, live
  compaction/stale-resume-retry rendering) were not run against a live pane
  this session — code-verified only.
- Issue #2707 (open, pre-existing, unrelated) — `AgentWorkingRow`'s
  type-out reveal restarting on tool-call bursts — lives in the same
  component Part 2 touched but was not addressed here.
- `container_spawn.rs`'s exec-start-failure path (the `Err(e)` arm when the
  exec itself never starts) still has no route to a persisted
  `AgentFailure` — a pre-existing gap, out of scope for what Codex flagged
  on PR #2825, noted there as a follow-up candidate.
**Scope — Part 1:** `agentmux-srv/src/backend/blockcontroller/health.rs`,
`agentmux-srv/src/backend/blockcontroller/{mod,core,persistent,acp}.rs`,
`agentmux-srv/src/backend/blockcontroller/subprocess/{mod,host_spawn,container_spawn}.rs`,
`agentmux-srv/src/agents/failure.rs`, `agentmux-srv/src/server/mod.rs`,
`frontend/app/view/agent/failure/failure-accessory.ts`,
`frontend/app/view/agent/hooks/{useAgentFailure,useAgentControllerStatus}.ts`,
`frontend/app/view/agent/agent-view.tsx`, `frontend/types/gotypes.d.ts`,
`frontend/app/store/wps-events.ts`.
**Scope — Part 2:** `frontend/app/view/agent/components/AgentComposerStrip.tsx`,
`frontend/app/view/agent/components/AgentFooter.tsx` (the `AgentWorkingRow`
it renders), `frontend/app/view/agent/agent-view.tsx`,
`frontend/app/view/agent/hooks/useResumeRetryStream.ts`,
`frontend/app/store/agent-pane-state/{store,types}.ts` (or wherever the
`retrying`/`resolved` resume-retry state actually lives, per implementer's
own re-verification at build time).
**Related:** `docs/specs/agent-health-design.md` (original design — to be
archived), `docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md`
(§2.7/§4 — where the Restart banner shipped), `docs/specs/SPEC_UNRESPONSIVE_FALSE_POSITIVE_DURING_COMPACTION_2026_08_22.md`
(the compaction false-positive patch — to be archived; **its own header
still says "Implementation status: Not started," which is stale — the fix
is fully shipped per `VERSION_HISTORY.md:58`; flagging the drift here so
it isn't a source of confusion later**), `docs/specs/SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md`
(the broader error-classification framework this sits inside — unaffected
by this removal except for losing one taxonomy entry),
`docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md`
(§6.2 — where "Reconnecting…" was designed, by analogy to "Compacting…
Ns," Part 2 revisits this), `docs/specs/SPEC_LONG_RUNNING_PROCESS_UX_2026_06_24.md`
(§140 — the prior worry about a `Disconnected` state showing misleading
"Reconnecting" text; already avoided in the shipped `Disconnected` UI,
noted here only because Part 2 touches the same neighborhood).

**GitHub tracking history (checked 2026-08-27 — no open issue/discussion
duplicates either half of this spec; these are the PRs/threads that
produced the features being touched, kept here so reviewers can trace
"why does this exist" without re-deriving it):**
- PR #2336 — `feat(agent): surface a Restart action when an agent process
  goes unresponsive` — the original feature Part 1 removes.
- Issue #2321 — `feat(telemetry): [wave-turn] and [health] diagnostic
  logging for the Working-state watchdog` — the diagnostic instrumentation
  added to chase false positives in this same detector; also candidate for
  removal/trim alongside Part 1 if it has no other consumer (verify at
  implementation time, not yet scoped in above).
- PR #2754 — `fix(agent): stop "Agent unresponsive" from firing during
  legitimate context compaction` (merged) — same fix already cited above
  via its spec file; this is the PR number for cross-reference.
- PR #2776 — `fix(agent): surface a Reconnecting… status during
  stale-resume retry` (merged) — the origin PR for the "Reconnecting…"
  state Part 2 relocates.
- Issue #2368 (closed) — `Persistent controller: transparent stale-resume
  retry still flashes a visible "Agent encountered an error" before the
  real response` — same stale-resume-retry subsystem as the above, already
  fixed (PR #2371/#2482); a different symptom (error flash, not
  Reconnecting placement) from what Part 2 addresses, kept only as
  background.
- Discussion #2375 — `Process/turn-liveness tracking consolidation —
  long-term tracking thread` — the umbrella thread for AgentMux answering
  "is this thing alive" via several independent, drifting mechanisms
  (HealthMonitor being one of them). Part 1's decision to keep
  `turn_active`/`is_active_turn()` rather than delete it outright is
  directly in scope for that thread's Phase B (write-side consolidation,
  deferred/not started per the thread) — worth a cross-link from there once
  Part 1 ships, not just from here.
- Issue #2707 (open) — `Tool-call bursts restart the agent-pane Working
  row's type-out reveal` — a live, unrelated bug in the exact
  `AgentWorkingRow` component (`AgentFooter.tsx`) Part 2 proposes adding
  Reconnecting/Compacting sub-states into. Not a duplicate of this spec and
  not blocking, but the Part 2 implementer should read it first so the
  reveal-restart bug isn't mistaken for a side effect of the new sub-states
  (or vice versa) during testing.

---

# Part 1 — Remove the false-positive-prone unresponsive detector

## 1. Report

The "Agent unresponsive" banner (🧊, offers a "Restart" action) is driven
by `HealthMonitor` (`health.rs`) — not a simple timer, but a small
composite state machine: a 30s/120s silence watchdog (`Stalled`/`Dead`), a
5-minute sliding-window error-rate tracker (`Degraded` at ≥5 transient
errors), and a compaction-awareness override (a 600s ceiling substituted
in while `compacting: bool` is set, patched in 2026-08-22 after a
confirmed false-positive: a normal Claude Code compaction pause of 231.6s
tripped the 120s `DEAD_SECS` threshold and fired the banner mid-legitimate-
operation). Despite that patch, the underlying report that shipped the
whole feature (`REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md`)
documents **three separate rounds of reagent-flagged bugs** on the
original implementation (a lock-ordering race, misclassifying a real auth
failure as generic "unresponsive" instead of "Login Again," and the ACP
controller never actually arming the watchdog at all) plus an admitted
**no-hysteresis flicker** on recovery (a late output line arriving just
after the threshold trips causes an immediate clear-then-possibly-re-fire).
The compaction spec's own §1.1 flags a **second, unconfirmed** suspected
false-positive source (long-running `Task`/subagent dispatches going silent
long enough to trip it) that was never investigated further. This is a
plausible, well-documented explanation for "nearly never works right."

**Corrected scope from the initial "just delete `HealthMonitor`" framing:**
research before this spec confirmed `HealthMonitor::is_active_turn()` (the
`turn_active` field it computes) is **not an internal implementation
detail of the unresponsive detector** — it's the sole source of truth for:
- `agentmux-srv/src/broker/process.rs`'s `lifecycle_from()` — the public
  `Lifecycle::Running/Idle/Done/Error` enum consumed by the Agent pane,
  the Swarm pane, and `muxspect describe`.
- `frontend/app/view/swarm/swarm-model.ts:813-820`'s
  `derivedRunningStatus()` — the Swarm pane's per-agent running/idle badge
  ("For `is_agent_pane` panes, trust `turn_active` alone").
- `agentmux-srv/src/backend/subagent_watcher/scan.rs`'s stale-subagent
  reconciliation (skips reconciling while the parent's turn is active).
- `muxspect_handlers.rs`/`reactive.rs`'s diagnostic JSON output and the
  `muxspect.mjs` CLI tool.

None of that is about detecting unresponsiveness — it's a general
"is this controller currently mid-turn" signal that happens to live on the
same struct. **Deleting `HealthMonitor` wholesale would silently break the
Swarm pane's running indicator and `muxspect describe`.** This spec removes
the false-positive-prone *detection* logic and keeps the turn-active
*tracking* it's bundled with.

## 2. Goals / Non-goals

**Goals**
- Remove the silence watchdog (`Stalled`/`Dead` thresholds), the
  error-rate tracker (`Degraded`), and the compaction-awareness override
  (`compacting`/`is_compact_boundary_frame`/`set_compacting`) — the entire
  source of false positives.
- Remove the "Agent unresponsive" banner, its Restart action, and the
  `FailureClass::Unresponsive` taxonomy entry.
- Remove the dead `agenthealth` WPS event (confirmed zero frontend
  subscribers today — diagnostic-only, never wired to anything).
- Keep `is_active_turn()`/`set_active_turn()`/
  `mark_turn_active_returning_was_active()` — rename the surviving struct
  away from `HealthMonitor` (misleading name for what's left) to something
  that describes what it actually still does, e.g. `TurnActivityTracker`.
- Keep process-exit classification (`0 → Idle`, non-zero → `Exited`) and
  the shared `FailureClass`/`AgentFailure`/`persist_last_failure` pipeline
  for every OTHER failure class (auth, rate-limit, network, etc.) —
  entirely unaffected by this removal.

**Non-goals**
- Do not touch `forceControllerRefresh()`/`ControllerResyncCommand` itself
  (shared with the unrelated post-login stale-process recovery flow) —
  only remove the `"restart"` context value's one call site
  (`agent-view.tsx`) that the unresponsive banner used; the `"login"`
  context path is untouched.
- Do not touch the AskUserQuestion dead-air fallback — confirmed
  independent (its own `stdout_seq_read` counter, not `HealthMonitor`).
- Do not touch launcher/host process-supervision liveness detection
  (`SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md`,
  `SPEC_SERVICE_SUPERVISION_AND_RECOVERY_2026_05_20.md`) — confirmed a
  fully separate subsystem (monitors AgentMux's own process tree, not the
  AI CLI subprocess inside a pane); shares only the English word
  "unresponsive," nothing else.

## 3. Design

### 3.1 Backend — `health.rs` (renamed conceptually; keep the file, slim it)

Remove:
- `AgentHealth` enum's `Degraded`/`Stalled`/`Dead` variants and the whole
  `compute_health()` state-transition function that produces them.
- `ErrorTracker` (5-minute sliding window, `DEGRADED_TRANSIENT_THRESHOLD`).
- `STALL_SECS`/`DEAD_SECS`/`COMPACTING_DEAD_SECS` constants and the silence
  timer they gate.
- `compacting: bool`, `compacting_started_ts`, `set_compacting()`,
  `is_compact_boundary_frame()` — this entire subsystem existed only to
  patch false positives in the silence detector; once the detector is
  gone, there is nothing left for it to patch.
- `publish_unresponsive_failure()`/`clear_unresponsive_failure()` and the
  `evaluate_and_transition()` logic that decided between "real fatal
  error" vs. generic "unresponsive."
- `last_meaningful_ts`/`record_output(meaningful: bool)`'s health-relevant
  half — **keep** the underlying stdout-forwarding call sites in each
  controller (they still need to feed turn-active/exit tracking), just
  drop the "was this meaningful for the silence timer" classification that
  fed the now-deleted watchdog.

Keep, renamed to reflect the smaller surface (suggested: `TurnActivityTracker`,
living in the same file or a renamed `turn_activity.rs`):
- `is_active_turn()` / `set_active_turn(bool)` /
  `mark_turn_active_returning_was_active(bool)`.
- `set_exited(code)` and whatever minimal state it needs to report
  `Idle`/`Exited` (or just expose the raw exit code directly, if nothing
  besides `lifecycle_from()` reads the wrapping enum — verify at
  implementation time whether `AgentHealth` can be deleted entirely in
  favor of the two independent fields `turn_active: bool` and
  `exit_code: Option<i32>`, since `Healthy`/`Idle` were never distinguished
  from each other by any consumer once `Degraded`/`Stalled`/`Dead` are
  gone).

### 3.2 Backend — the watchdog task, controller wiring, HTTP special-case

- `core.rs`: delete `spawn_health_watchdog()` (the 5s `tokio::time::interval`
  task) — with no silence timer to check, there's nothing for it to poll.
- `mod.rs`: `Controller::health_monitor()` trait method — rename/retype to
  return the slimmed tracker (or delete entirely if the only remaining
  reason anything reached through it was the `compaction_started` HTTP
  special-case below, which itself is being removed).
- `server/mod.rs`: delete `handle_wps_publish`'s `compaction_started`
  special case (the `set_compacting(true)` call) — dead code once
  `compacting` no longer exists. Confirm nothing else in
  `agentmux-bashwrap`'s `precompact` HTTP POST path needs to keep firing
  for an unrelated reason before deleting the endpoint itself (it may be
  worth keeping the POST endpoint as a no-op vs. fully removing it,
  depending on whether `agentmux-bashwrap` has its own reason to send it
  regardless of who's listening — check before deciding).
- `persistent.rs`, `subprocess/host_spawn.rs`, `subprocess/container_spawn.rs`,
  `acp.rs`: each controller's stdout-reader loop keeps calling
  `set_active_turn`/`mark_turn_active_returning_was_active`/`set_exited`
  (renamed tracker), drops the `record_output(meaningful)` health-silence
  bookkeeping and the `health_monitor().set_compacting(...)` call sites.

### 3.3 Backend — shared taxonomy

- `agents/failure.rs`: remove `FailureClass::Unresponsive` from the enum.
  Every other variant (`RateLimited`, `Overloaded`, `UsageLimit`, `Auth`,
  `ContextExceeded`, `MaxTurns`, `Network`, `Killed`, `NoOutput`,
  `SpawnFailure`, `UnknownNonZero`) is untouched — this taxonomy continues
  to serve real exit-classification, entirely independent of this removal.
- `core.rs`'s `persist_last_failure()`/`agent:last_failure` block-meta key
  — untouched (generic, used by every remaining failure class).

### 3.4 Frontend

- `failure-accessory.ts`: remove `ICON["unresponsive"]` and the
  `case "unresponsive":` arm.
- `useAgentFailure.ts`: remove the null-data-event self-heal-clear special
  case that was scoped to `code === "unresponsive"` (the generic
  clear-on-explicit-event path for every other failure class is
  untouched).
- `useAgentControllerStatus.ts`: remove the `"restart"` literal from
  `forceControllerRefresh(context: "login" | "restart")`'s type — becomes
  just `"login"` (its remaining, unrelated caller) unless another
  future caller needs a generic restart context, in which case keep the
  union but drop only the banner's specific call site.
- `agent-view.tsx`: remove the `onRestart` wiring at the banner's
  `useAgentFailure({...})` call site (~line 1355, ~1834-1842).
- `gotypes.d.ts`: remove `"unresponsive"` from the `AgentFailure["code"]`
  union.
- `wps-events.ts`: remove the `AgentHealth: "agenthealth"` event-type
  constant — confirmed zero subscribers anywhere in the frontend tree
  today, so this is a pure dead-code deletion, not a behavior change.

### 3.5 Backward compatibility (no migration needed)

Block meta (`agent:last_failure`) is a schemaless JSON blob, not a typed
DB column — no migration required. An existing pane whose persisted
`agent:last_failure` still has `code: "unresponsive"` from before this
ships will, after the TS union member is removed, simply fall through to
the frontend's `default:` switch arm (generic `⚠`/"Retry" action) the next
time that row renders — not a crash, just a cosmetically-generic rendering
of stale data until it's next cleared.

## 4. What this does NOT remove (confirm at implementation time)

- Turn-active tracking itself (§3.1's "keep" list) — genuinely load-bearing
  for the Swarm pane, `process.rs::lifecycle_from()`, subagent
  reconciliation, and `muxspect`. Removing this would be a regression, not
  a cleanup.
- `FailureClass`/`AgentFailure`/the failure-banner pipeline itself — every
  other failure class keeps working exactly as today.
- The post-login stale-process recovery flow
  (`forceControllerRefresh("login")`).

## 5. Docs to archive

Move to `docs/specs/archive/` (or annotate "superseded by this spec," per
existing repo convention) once the removal ships:
- `docs/specs/agent-health-design.md`
- `docs/specs/SPEC_UNRESPONSIVE_FALSE_POSITIVE_DURING_COMPACTION_2026_08_22.md`
  (correct its stale "Implementation status: Not started" header on the
  way in, so the archive doesn't preserve a wrong claim)

`docs/reports/REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27.md`
and `docs/specs/SPEC_AGENT_ERROR_FRAMEWORK_2026_06_20.md` stay as-is — both
cover broader ground (the general error-classification framework, and an
incident report) that outlives this one feature's removal; leave a note in
each pointing at this spec for what changed.

## 6. Test plan

**Delete** (tests exercising the removed detection logic):
- `health.rs`'s `#[cfg(test)] mod tests` — the health-state-machine tests
  (`test_health_monitor_lifecycle`, `test_health_monitor_fatal_error`,
  `dead_transition_publishes_unresponsive_failure`,
  `dead_via_recognized_fatal_error_publishes_the_correct_class_not_unresponsive`,
  `dead_via_unrecognized_fatal_error_falls_back_to_classifys_own_default`,
  `dead_recovery_clears_the_unresponsive_failure`,
  `concurrent_transitions_never_leave_a_stale_publish`) and the entire
  "Compaction false-positive fix" test block (`is_compact_boundary_frame_*`,
  `compacting_*`, "stale compacting cleared at turn/process boundaries").
- `server/tests.rs`'s `wps_publish_compaction_started_marks_the_target_blocks_health_monitor_compacting`,
  `wps_publish_compaction_started_for_an_unregistered_block_is_a_harmless_no_op`,
  and the `CompactionTestController` test double.
- `failure-accessory.test.ts`'s `"unresponsive → Restart..."` case.
- `useAgentFailure.test.ts`'s `describe("useAgentFailure — unresponsive
  self-heal clear signal", ...)` block.

**Keep, verify still passing (they test the surviving turn-active surface,
not the removed detection):**
- `mark_turn_active_returning_was_active_reports_the_pre_call_value`,
  `mark_turn_active_is_atomic_across_repeated_calls` (rename target module,
  same assertions).
- `acp.rs`'s `send_input_marks_the_turn_active`,
  `repeated_send_input_while_active_does_not_error`.
- `persistent.rs`'s `status_snapshot_turn_active_tracks_health_monitor`
  (rename to drop "health_monitor" from the name if the struct is
  renamed) and its `is_active_turn` assertions.

**Manual, since this touches live turn-status plumbing:**
- [ ] Launch a Claude Code agent, run a long tool call, confirm the Swarm
      pane's running/idle badge still reflects turn-active correctly
      (this is the regression risk called out in §1 — verify explicitly,
      not just by code inspection).
- [ ] Confirm `muxspect describe <block>` still reports accurate
      Running/Idle/Done/Error lifecycle.
- [ ] Confirm no "Agent unresponsive" banner appears under any
      circumstance post-removal (the actual point of this work).
- [ ] Confirm a genuinely hung/killed process still surfaces the correct
      OTHER failure class (e.g. `Killed`/`SpawnFailure`) where applicable
      — this removal must not silently swallow real, still-wanted failure
      signals, only the false-positive-prone generic "unresponsive" one.

## 7. Rollout

Single PR — the removal is additive-in-reverse (every integration point
was designed to no-op cleanly when absent, per the original design's own
principle), so there's no phased rollout needed. Recommended commit
structure for reviewability: (1) backend detection-logic removal +
renamed tracker, (2) frontend banner/UI removal, (3) doc archival — as
either three commits in one PR or three small PRs, whichever this
session's reviewer throughput prefers.

## 8. Addendum (2026-08-25): confirmed — the same flaw also threatens any long-running tool call, not just compaction

Re-verified by tracing the actual code (not just the compaction spec's own
speculative §1.1 note): `classify_output_line()` (`health.rs:644-699`)
marks essentially every NDJSON line "meaningful" except `rate_limit_event`
— the silence timer only advances when a line *arrives at all*. No
controller emits (and Claude Code's own CLI protocol has no) synthetic
"still working" frame between a `tool_use` request and its eventual
`tool_result` — stdout goes genuinely silent for the tool's whole
duration. `set_compacting()` is the *only* exemption from the 30s/120s
thresholds, invoked exclusively from the `PreCompact`/`compact_boundary`
wiring; there is no equivalent `set_tool_running`/`set_task_dispatch`
guard anywhere. So a single long `Bash` call (a multi-minute `npm
install`, a blocking dev-server start) or a long `Task`/subagent dispatch
is judged purely by wall-clock silence, with the identical exposure
compaction had before its dedicated patch.

No confirmed real incident is logged for this specific case (same as the
compaction spec's own honest "speculative, not verified" framing) — but
this is exactly the point: **the compaction fix patched one instance of a
structural flaw, not the flaw itself.** Treating this as "yet another case
that needs its own `set_compacting`-style exemption" would repeat the
whack-a-mole pattern (today compaction, next whatever surfaces after
this); removing the wall-clock-silence detector entirely (Part 1 above)
resolves this class of false positive categorically rather than by
enumeration. This finding is additional evidence for the removal, not a
new decision point — it doesn't change §2-§7's scope.

---

# Part 2 — Consolidate "Reconnecting…" / "Compacting…" / "Working" into one status location

## 9. Report

Watching an agent pane, "Reconnecting…" reads as inexplicable next to
"Working…" — not because either is wrong, but because they're two
independent components answering two different questions, rendered in two
different screen locations, with no visual relationship to each other:

- **"Working"** — `AgentWorkingRow` (`AgentFooter.tsx:53-130`), rendered
  above `AgentDocumentView` (`agent-view.tsx:2191`). Means "actively
  generating a turn right now."
- **"Reconnecting…" / "Compacting… Ns"** — both live inside
  `AgentComposerStrip`'s `rightText()` memo (`AgentComposerStrip.tsx:226-253`),
  a "center stats zone" directly above the textarea — a *different* DOM
  region entirely. `Reconnecting` fires specifically when the underlying
  CLI process has already **crashed and exited** and AgentMux is
  transparently retrying with a fresh `--resume` session id
  (`persistent.rs:1610`ff, `EVENT_AGENT_RESUME_RETRY`). `Compacting… Ns`
  fires during a legitimate context-compaction pause.

These three states are not mutually exclusive by construction — they live
in two separate components that neither know about nor coordinate with
each other. A user has no way to know there even *are* two separate status
slots, let alone which one to look at, or why "Reconnecting" can appear
while "Working" is simultaneously absent (correctly so — the process is
dead, nothing is "working" in that moment — but nothing communicates that
distinction to the user in the moment they're confused).

**Why not literally just show "Working…" during a resume-retry, as
asked?** Because it would be a regression in honesty, not a simplification
— `Reconnecting` exists specifically because a resume-retry means the
process actually died (`STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md`
§6.2 shipped it by deliberate analogy to `Compacting…`, precisely to stop
that recovery window from silently reading as a hang). Collapsing the
*wording* back to "Working" would hide a real, distinct event (a crash
recovery) behind a label that means something else. The actual problem
isn't the wording — it's that this real, meaningful state is surfaced from
a completely different corner of the screen than the one users already
know to check.

## 10. Design recommendation

**Move `Reconnecting…` and `Compacting… Ns` into the SAME status slot
`AgentWorkingRow` already owns, as two additional named sub-states of
one row — not into the composer strip's separate zone.** Concretely:

- `AgentWorkingRow` (`AgentFooter.tsx`) becomes the single place a user
  checks for "is something happening to this agent right now," rendering
  one of: `Working` (existing), `Compacting… Ns` (moved from the composer
  strip), `Reconnecting…` (moved from the composer strip), or nothing.
  These four states are mutually exclusive by construction (a `Show`
  chain on one signal, not three independently-firing components).
- **Keep the wording distinct per state** (§9's "why not just say
  Working" answer) — this is a placement fix, not a wording
  simplification. A user who wants to know *why* something is happening
  can still tell compaction from a crash-recovery from active generation;
  they just now only ever have one place to look.
- `AgentComposerStrip`'s `rightText()` memo (§9 above) loses the
  `Reconnecting`/`Compacting` branches — it goes back to showing only the
  plain loading-token/elapsed readout it already had before those two
  states were added there.
- **[DECISION NEEDED, not defaulted]:** whether to also add a short
  inline explanation the first time a user sees `Reconnecting…` (e.g. a
  one-line "the process restarted — resuming your session" under the
  status row, dismissible/shown-once) — the research surfaced real user
  confusion ("makes no sense"), which a relocated-but-still-terse label
  alone might not fully resolve. Flagged rather than assumed; low
  implementation cost either way, but it's a product-copy decision, not
  an architecture one.

**Alternative considered, not recommended:** merge the three states into
one always-generic "Busy" label everywhere. Rejected — this is the same
kind of information-loss the "just say Working" instinct has, and the
compaction/reconnect states exist *because* a prior generic "is it
hanging?" ambiguity was a real, reported problem (per
`STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md`'s own
motivation) — collapsing distinct, meaningful states back into one generic
label would re-introduce the exact ambiguity that motivated adding them
in the first place. The fix belongs at the *placement* layer, not the
*vocabulary* layer.

## 11. Test plan

- Frontend: `AgentComposerStrip.test.tsx` (or equivalent) — assert
  `rightText()` no longer renders `Reconnecting…`/`Compacting… Ns` after
  the move; assert the plain loading-stats readout still renders
  correctly on its own.
- Frontend: `AgentFooter.test.tsx`/`AgentWorkingRow` tests — new cases for
  the `Compacting`/`Reconnecting` sub-states rendering with the correct
  text and mutual exclusivity against `Working`/idle.
- Manual: trigger a real compaction and a real stale-resume retry (or the
  existing dev/test harness each already has, per their own specs) and
  confirm the relocated status reads correctly in the new single location,
  with no flash of the old composer-strip text.

## 12. Rollout

Independent of Part 1 — can ship before, after, or in parallel (different
files, no shared code path: Part 1 removes a failure-classification
detector, Part 2 relocates two *already-correctly-triggered* status
displays). Single PR; no migration or persisted-state concerns (pure UI
relocation, no backend wire-format change).
