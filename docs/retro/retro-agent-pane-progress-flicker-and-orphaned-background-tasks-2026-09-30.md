# Retro: the agent pane's progress ring flickers mid-turn, and background tasks show as running for 17 hours

**Date:** 2026-09-30
**Author:** Maricon (charlie)
**Status:** retro — root causes found; §5 fixes 1 and 2 shipped (#4035, #4040), 3–5 open
**Severity:** Medium. The ring is the main "the agent is still working" signal, and
it now goes dark while the agent is busy. The stale rows mislead anyone reading the
Activity Dock or Swarm, and they keep the ring flickering.

---

## 1. Symptoms

1. **The ring goes on and off during a turn.** While an agent is plainly still
   working (its turn hasn't returned), the progress ring around the pane
   repeatedly switches off and back on. It used to stay on for the whole turn.
   The look of the ring (#3371, #3396, #3443) is fine; only its on/off state is
   wrong.
2. **Background tasks shown running for 17 hours.** Opaz's pane listed two
   background tasks as running since 2026-09-29 01:41 and 02:44 local
   (08:41 and 09:44 UTC). Both had long since stopped existing.

The two are linked: symptom 2 makes symptom 1 permanent for the affected pane (§3.3).

## 2. Root cause 1: "no tool call running" was taken to mean "the agent is idle"

The ring is `agent-pane-progress-bar--active` when `paneBusy()` is true
(`frontend/app/view/agent/agent-view.tsx:2541-2549`). `paneBusy` is
`paneBusyForInput` (`agent-view.tsx:1727-1737`), which since #3340 (2026-09-17,
shipped in v0.56.5) goes false for a Streaming turn when
`turnHeldOnlyByBackgroundWork` holds (`frontend/app/view/agent/working-indicator.ts:113-138`):

```ts
if (input.turnPhase.kind !== "Streaming") return false;
if (!input.hasAttachedBackgroundWork) return false;
return !input.hasBlockingForegroundToolCall;
```

`hasBlockingForegroundToolCall` is true only while some tool node is
`running`, `pending_approval` or `awaiting_answer`
(`frontend/app/view/agent/activity/tool-adapter.ts:130-154`).

The intent was right, and owner-confirmed on 2026-09-17: a turn that is still
open **only** because of accepted background work shouldn't read as busy, so the
user can talk to the agent (§2.3a of
`docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md`).

The detection is wrong. "No foreground tool call is running at this instant" is
also true **every time the model is thinking or writing text between two tool
calls**. So for any agent with background work attached:

| Moment in the turn | Tool node running? | Ring |
|---|---|---|
| Model generating (thinking, text) | no | **off** (wrong) |
| A tool call running | yes | on |
| Model generating the next step | no | **off** (wrong) |

That is the on/off pattern. It only happens while background work is attached,
so how often a pane shows it depends on how much background work (background
Bash, `Monitor`, background subagents) its agent uses. In a pane with an orphaned
`running` row (§3) it happens on every turn.

**Why tests didn't catch it:** `working-indicator.test.ts:98-109`
("is NOT busy: Streaming, backgrounded, and nothing else blocking") encodes the
carve-out with the only inputs the predicate has. `WorkingIndicatorInput` has no
field for "the model is generating right now", so "idle, waiting on background
work" and "between tool calls, mid-thought" are the same input, and the test
asserts the first reading for both.

## 3. Root cause 2: the registry never hears that a subagent's background tasks ended

### 3.1 What the registry shows

`db_background_tasks` for Opaz's block (`cdf22b5f…`), queried 2026-09-30 01:58 UTC:

| Tool use id | Label | Started (UTC) | Last seen | Status |
|---|---|---|---|---|
| `toolu_01NRT4HYRocdyZdnV1k4rHsH` | `agentmux-bashwrap exec …` (a CEF ninja build) | 09-29 08:41 | 08:41 | `running`, 17.3 h |
| `toolu_01QvtBBNoDBFVBUWmkC5n3Qm` | "cef 154 linux ninja build attempt-3 progress/completion (continued 2)" (`Monitor`) | 09-29 09:44 | 09:44 | `running`, 16.2 h |

`pid` is null on both. The table's only other `running` row was a 30-minute-old
wait that was genuinely live at the time.

### 3.2 What the CLI actually said

Both tasks belong to a **background subagent** of Opaz ("Build and verify CEF 154
Linux runtime", `agent-aead97a65aa665a43`). Opaz's raw stream (the block's
`output` file) shows:

1. `task_started` for `brsbiubq6` (the Bash call), then
   `task_updated {is_backgrounded: true}` when it passed its 120 s timeout and the
   harness moved it to the background. `background_task_feed.rs` created the row.
2. `task_started` for `bbd9d926l` (the Monitor, `is_backgrounded: true`,
   `owned_by_subagent: true`). Row created.
3. At 09:44:05 the subagent ended its turn ("Waiting for the next notification.").
   The CLI sent `task_updated {status: completed}` and `task_notification` for the
   **subagent** (`aead97a…`), and **no end event for either of its two tasks**.
4. The CLI then started a new session segment (`init`, then the
   `SessionStart:resume` hooks). The next `background_tasks_changed` snapshot
   lists only the subagent. Both tasks are simply gone from it.

Neither task ever got a `task_updated` with a status, or a `task_notification`,
in Opaz's stream or in the subagent's transcript. The srv log shows no srv restart
or controller respawn at 09:44; the turn just ended (`turn_active flip false`
09:44:07). I haven't established why the CLI dropped the tasks silently rather
than ending them. What's certain is that **the only record that they ended is
their absence from `background_tasks_changed`**.

### 3.3 Why AgentMux keeps them running

- `background_task_feed.rs` (#3953, 2026-09-27) closes a row only on
  `task_updated {status}` or `task_notification`. It never reads
  `background_tasks_changed`; nothing in the repo does.
- The renderer's own path (`parseTaskNotification`) needs a `<task-notification>`
  in the pane, which never comes for a subagent's task (#3953 spec §1).
- The backstops for "tasks the CLI never reports ending" are Phase 4 of
  `docs/specs/SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md` (§5):
  bashwrap `terminal` events and a pid liveness sweep. **They're not started**,
  and a liveness sweep couldn't help here anyway, because the pid is null.
- Nothing puts a lifetime on a row. A `running` row with a `last_seen_ms` 17 hours
  old is shown as running.

The connection to root cause 1: `useBackgroundTaskRegistry.ts:74-76` sets
`registryAttachedTaskSince` whenever **any** `running` row exists for the block.
That feeds `hasAttachedBackgroundWork` (`agent-view.tsx:1733-1734`). So from 09:44
UTC on, Opaz's pane satisfied the carve-out's precondition on every turn, and its
ring flickered on every turn for the rest of the day.

## 4. What went well, and what didn't

- **Went well:** both changes were well documented (#3340's commit message, the
  #3953 spec). The raw stream, the registry and the transcripts were all on disk,
  so the root cause could be proven from data rather than inferred.
- **Didn't:** #3340 changed what a user-facing signal *means* using an input that
  can't express the distinction. Its tests cover the predicate's inputs, and the
  commit message is about the send path it also changed; nothing in the PR shows
  the ring being watched through a real multi-step turn with background work
  attached.
- **Didn't:** #3953 fixed the "stuck running" class for the cases it measured
  (10 of 11 ended with `task_notification`, 1 with `task_updated`), but treated
  the CLI's end events as complete. The same stream also carries
  `background_tasks_changed`, the one signal that covers silent drops.

## 5. Proposed fixes

In priority order. Each is small and independently shippable.

1. ✅ **Done in #4035.** **Busy while the model is generating.** Add a `modelGenerating` input to
   `WorkingIndicatorInput`, true from the stream's first `status: requesting` /
   `message_start` of a turn step until that step's assistant message completes.
   `turnHeldOnlyByBackgroundWork` returns false while it is true. The carve-out
   then applies only in the real case: the model has stopped and is waiting on
   background work. Tests: "Streaming, backgrounded, model generating → busy",
   plus the existing "model idle → not busy" case.
2. ✅ **Done in #4040.** **Reconcile the registry against `background_tasks_changed`.** In
   `background_task_feed.rs`, treat each snapshot as the CLI's full set of live
   tasks for that stream. A `running` row this reader has seen start, whose
   `task_id` is missing from a later snapshot, becomes `stopped` with the
   snapshot's time. On a new `init` for the same block, rows from the previous
   session segment that are absent from the first snapshot get the same
   treatment. Test with the exact sequence in §3.2.
3. **A staleness ceiling in the dock and in `registryAttachedTaskSince`.** A row
   with no pid and a `last_seen_ms` older than N hours (for example 2) shouldn't
   count as attached work, and the dock should show it as "unknown" rather than
   "running". This protects against the next silent-drop path we haven't found.
4. **Phase 4 of the #3953 spec** (bashwrap `terminal` events, a pid liveness
   sweep) remains worth doing for the CLI-dies-mid-task case; fix 2 doesn't cover
   that.
5. **Clean up the two existing orphans** once fix 2 or 3 ships, or by hand
   (`status = 'stopped'`) if the pane needs relief sooner.

### 5.1 As built

- **Fix 1 (#4035)** is a flag on the `Streaming` phase, `modelEndedTurn`, rather than a separate input: set by the main agent's `message_delta` with `stop_reason: end_turn`, cleared by its next `message_start`. Measured on 87 real turns in two agents, the CLI sends `result` 2–3 lines after that `end_turn`, so a Claude turn now stays busy until it ends.
- **Fix 2 (#4040)** doesn't end a task when it leaves the snapshot: every task that ended normally (36 of 36) left the snapshot one line **before** its end event. A task that leaves is marked vanished, and settled as `stopped` only if its end event hasn't come by the next snapshot or turn boundary (`result`, or `init`).
- **Fix 5 still applies to Opaz's two rows.** Fix 2 needs the feed's in-memory record of a task, which a row created before the fix (or before an srv restart) doesn't have. Those rows need fix 3, or a manual `stopped`.

## 6. Lessons

- **A predicate that changes what the user sees needs an input that captures what
  the user sees.** "Is a tool call running?" isn't "is the agent working?". When a
  test asserts a UI state, check that its inputs can tell the cases apart that the
  user can tell apart.
- **Snapshots are the backstop for event streams.** If a source sends both
  lifecycle events and periodic full snapshots, reconcile against the snapshot;
  events get dropped.
- **Any "running" state without a pid needs a lifetime.** Something that can't be
  checked for liveness must eventually stop claiming to be alive.
