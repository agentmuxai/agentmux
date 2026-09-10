# REPORT: Agent pane progress indicators — desync, background handoff, and ambient narration

**Status:** active
**Date:** 2026-09-09
**Area:** agent pane / working state / activity dock
**Supersedes as tracking doc:** see §7 — this consolidates ~20 fragments; none are deleted, all are indexed and classified.
**Related:** [SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04](../specs/SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md) (active, Phase 2 open — the designated umbrella spec)

---

## 1. What was reported

Four complaints, from live use:

1. The **"Working…" text and the top animation bar disagree** — they are visibly out of sync.
2. After a long tool call is **backgrounded, the indicators stay lit**, and the user must press <kbd>Esc</kbd> before they can type to the model.
3. When a task is backgrounded, **the user is told nothing** about what just happened.
4. There is no general way for AgentMux to **speak into the conversation in the model's voice** when it takes an action on its own.

Plus a process ask: existing tracking docs are fragmented; consolidate.

All four are real. Three have precise mechanical causes identified below. One (§4) is a genuine design gap.

## 2. Corrections to the framing

These matter because they change what gets built. §2.3 is the definition everything
else in this document is measured against.

### 2.1 AgentMux does not own the 30-second backgrounding decision

The **Claude Code harness** decides, per call, whether a tool call runs in the background. AgentMux only *observes* the outcome, by matching a literal string in the tool result:

- `frontend/app/view/agent/activity/tool-adapter.ts:57` — `BACKGROUND_LAUNCH_ACCEPTED_PREFIX = "Command running in background with ID:"`
- `tool-adapter.ts:111` — `isAcceptedBackgroundLaunch()` requires `params.run_in_background === true` **and** `status === "success"` **and** that prefix.

Its own doc comment (`:84-110`) records that a fast-finishing call is returned synchronously *despite* the flag — the misclassification behind issue #2518's 17 stuck dock rows.

The 30-second constant that does exist is a **display promotion**, not a backgrounding threshold:

- `tool-adapter.ts:36` — `TOOL_PROMOTION_MS = 30_000`, used at `:171,185,284`. A *foreground* call gets a dock row after 30s.
- Server mirror for diagnosis only: `agentmux-srv/src/server/muxspect_handlers.rs:434` — `DOCK_STUCK_THRESHOLD_MS = 30_000`.

`effective_idle_timeout` is unrelated: `agentmux-bashwrap/src/bash_wrap.rs:322` returns `u64::MAX` for `--declared-background`, else the **600s** default (`:306`).

**No 30-second foreground→background conversion timer exists in `agentmux-srv` or `agentmux-bashwrap`.** Any design that assumes AgentMux can hook "the moment we background it" must instead hook the moment we *detect* the harness did.

### 2.2 Turn-phase and process-liveness are orthogonal — do not merge them

`docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` carries an explicit correction notice to this effect, from a 12-hour stuck-state incident. A previous attempt to collapse the two axes was wrong.

This constrains §8.1: the fix is that the two **indicators** derive from one predicate. It is *not* that the underlying axes get unified. `turn_active` (backend, CLI `result` frame) and `turnPhase`/`attachedTask` (frontend) stay distinct.

### 2.3 The indicators have exactly one meaning — and it is the input gate

Stated by the repo owner, 2026-09-09, and authoritative for this work:

> Both indicators mean: **if you type now, your message will be queued until the
> next turn.** If neither is present, the agent responds immediately. Work the
> user would not otherwise see is the **dock's** job, not the indicators'.

This is a definition, not a preference, and it settles the design:

- **The indicator predicate and the composer's input gate are the same boolean.**
  Not "derived from a shared source" — the same value. The indicator's entire job
  is to make the input gate visible before the user discovers it by typing.
- The bar and the row are **two renderings of one fact**, at different places on
  screen. Any state where they differ is a bug by construction.
- "Work you are not otherwise seeing" is **out of scope for both**. That is what
  dock rows are for. An indicator must never light for it, and must never go dark
  because the dock has taken over.

An earlier draft of this report proposed keeping two distinct meanings
(`turnInFlight` vs `unattributedWork`) and deriving both from one state object.
**That was wrong** and is withdrawn — it would have preserved the exact class of
divergence being reported, with better naming.

**This definition puts an implemented, active spec in conflict.**
`SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01` specifies that the working row
stands down when the dock takes over — which is precisely §3.1's divergence. Under
§2.3 that stand-down is a defect: input is still queued during a promoted call, so
hiding the row tells the user the opposite of the truth. That spec needs an
explicit correction, not a silent code change around it.

## 3. Problem 1 — the indicators are three copies of one predicate

There is no shared source of truth. The same idea is written out three times, and they have drifted:

| Consumer | Expression | Site |
|---|---|---|
| Top progress bar | `showingLaunchActivity() \|\| workingFromPhase(paneModel.state.turnPhase)` | `frontend/app/view/agent/agent-view.tsx:2296-2297` |
| "Working…" text | the same, **minus** `supersededByDock()`, **plus** `compacting`/`reconnecting` | `agent-view.tsx:1630-1634`; `components/AgentFooter.tsx:314` |
| Composer strip | the bar's raw expression, copied again | `agent-view.tsx:2580` |

Shared base predicates: `workingFromPhase` (`frontend/app/store/agent-pane-state/types.ts:468-471` — true for `Submitting`/`Streaming`/`Interrupting`) and `showingLaunchActivity` (`agent-view.tsx:1327`).

### 3.1 Divergence A — dock promotion (text OFF, bar ON)

At `TOOL_PROMOTION_MS` a Bash call is promoted to the Activity Dock. `workingRowSupersededByDock()` (`frontend/app/view/agent/activity/working-row-supersession.ts:44-63`) goes true; the timer that arms it is `agent-view.tsx:1611`.

The **text** reads it. The **bar** does not — `agent-view.tsx:2296` has no `supersededByDock` term.

Result: at exactly 30 seconds into any long Bash call, the text disappears and the bar keeps marching. This is structural and fires every time, not a race.

Under §2.3 the **bar is correct here and the row is wrong**: a promoted call is
still a call in flight, so input is still queued, so the indicator must stay lit.
The row is not "standing down for the dock" — it is under-reporting a gate that is
still closed. The user learns the truth by typing and being told to wait.

### 3.2 Divergence B — compacting / reconnecting (text ON, bar OFF)

`AgentFooter.tsx:314` renders the loading row for `compacting` or `reconnecting` alone, and `reconnecting` is documented as settable while `loading` is false (`AgentFooter.tsx:106-109`). The bar has no input for either.

Compounding it, the liveness watchdog is suspended entirely while compacting (`frontend/app/store/agent-pane-state/reducer.ts:327-329`), so nothing force-clears the mismatch.

Here the **row is right and the bar is wrong** — the reverse of §3.1.
`reconnecting` is set ONLY after the underlying process has already crashed or
exited, so there is nothing alive to answer a message typed then; `compacting`
means the CLI is busy with the compaction. Both are genuinely "not now".

**Fixed in #3143** (after review — the first revision of that PR closed §3.1 only
and still claimed the consumers could not disagree). Both fields are now terms in
the shared predicate, so the bar and composer strip light for them too.

One wording correction that came out of the same review: the predicate promises
"will **not be answered immediately**", not "will be **queued**". Only the
turn-in-flight case actually queues — a message sent during launch/relogin is
rejected by the auth guard, and during a reconnect there is no process to take
it. The user-facing promise is only the common factor.

### 3.3 Asymmetric timers

Every timer sits on the text side, none on the bar:

- type-out reveal `REVEAL_CHAR_MS = 28`/char (`AgentFooter.tsx:174,207-217`), phrase rotation every 30s (`:241-247`)
- promotion timer (`agent-view.tsx:1611`)
- `sessionStats != null` keeps the row mounted as "✓ Worked" (`AgentFooter.tsx:316-325`) after the bar has faded

The bar's only gate is a CSS `transition: opacity 200ms` (`agent-view.scss:261`) over an `infinite` keyframe animation (`:307`). It has no completion gate at all.

## 4. Problem 2 — backgrounding never releases the foreground

Two distinct layers, and the bug is in the seam.

**Backend `turn_active` behaves correctly.** `TurnActivityTracker` (`agentmux-srv/src/backend/blockcontroller/health.rs:39,57,73,82`) flips false on the CLI's `result` frame and nothing else (`persistent.rs:3036-3060`, set true at `:2691`). It is entirely orthogonal to background tasks — a backgrounded call does **not** hold it open. It is surfaced at `blockcontroller/mod.rs:172` and wins over `shellprocstatus` unconditionally in `broker/process.rs:152-159`.

**The frontend indicators are a separate axis** — `turnPhase` plus the independent `attachedTask` axis (`types.ts:357-379`, actions at `:768,799`).

**The input gate is the third copy of the predicate** (`agent-view.tsx:2580`). Because it reads the *raw* phase expression, it stays closed while `turnPhase` is working — irrespective of whether the call has been handed to the dock or to the harness's background tracker. Nothing in the handoff path tells the composer it may reopen. That is the Esc.

### 4.1 The handoff hook already exists

There is a clean attachment point at the exact moment of detection:

- `frontend/app/view/agent/stream-flush-queue.ts:68` sends `docknodestatus` with `run_in_background`
- handler `COMMAND_DOCK_NODE_STATUS` (`agentmux-srv/src/server/websocket.rs` ~1252) → `background_task_observe()` → `publish_background_task_updated()`
- broadcast `WaveEvent { event: "background-task-updated", scopes: ["block:<id>"] }` (`websocket.rs:765-773`) — payload is `{ block_id }` only, an invalidation ping
- already consumed: `frontend/app/store/wps-events.ts:69` → `hooks/useBackgroundTaskRegistry.ts:16-22`

Completion returns via `COMMAND_BACKGROUND_TASK_COMPLETION` (`websocket.rs` ~1345), parsed by `tool-adapter.ts:136`.

So both the UI release (§5.2) and the narration (§5.3) have a place to hang. No new transport is required.

## 5. Problem 3 — the stuck case, and the dead scrub

Distinct from the desync, and the strongest lead in the codebase.

`SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md` §1.1 root-caused stuck `running` ToolNodes: `toolCallToNode()` creates nodes optimistically at `status:"running"` (`stream-parser.ts` ~519); the only exit is a matching `tool_result` (`claude-translator.ts:334-350`); a harness *pre-execution rejection* arrives as a turn-terminating top-level `result` frame instead, so the node never closes.

`finalizeTurn()` (`hooks/useTurnLifecycle.ts:73`) clears "Working…" in the *pane* reducer but never touches the *document* reducer's `nodes[]`, and never calls `scrubOrphanedInProgress`. That scrub (`frontend/app/store/agent-document/reducer.ts:53-148`) runs only on session/reload boundaries — never on a turn boundary.

**`ScrubOrphanedInProgress` has zero live dispatch call sites.** The remediation was built and never wired up. That is a small, high-value fix independent of everything else here.

## 6. Problem 4 — narration: no injection facility, but the model gateway exists

### 6.1 What does not exist

There is **no production mechanism that writes an assistant/model-authored message into an agent's conversation from outside the model**. Searched `inject`, `synthetic`, `notice`, `system_message`, `assistant_message`.

What exists is adjacent but different:

- **View-only node injection**, not in the real transcript: `frontend/app/view/agent/inject-history-link.ts:25`, `inject-resume-preflight.ts`, `failure/synthetic-row.ts` (a "Not signed in" row). These splice `DocumentNode`s into `displayDocument` at render time.
- **User-side injection**: `persistent.rs:2147` `send_user_message` — the jekt/muxbus path. Writes a *user* turn and would start a new turn. Wrong role, wrong side effect.
- Assistant-role JSON is only ever parsed (`agents/translator/claude.rs:112,280`) or fabricated in tests.

### 6.2 What does exist — the Ambient Model Call gateway

Already built, already load-managed:

- `agentmux-srv/src/server/app_api/session.rs:1022` — `invoke_ambient_haiku_call(cli_path, prompt, meta, cancel)`
- spawns the Claude CLI with `--model claude-haiku-4-5-20251001` (`:1036-1038`), 15s timeout, cancellation token, token usage parsed back
- six existing callers (`:325,499,621,713,808,911`) — activity summary, ghost text, session/definition titles, subagent backlog naming
- concurrency caps at `:248,336,352` and `backend/reactive/activity_watcher.rs:48`

**The narration feature is a new caller of an existing gateway, not new infrastructure.** The cost is the injection path, not the model call.

## 7. Consolidation

~20 documents cover this territory. **No document anywhere treats the working row and the top bar as one system** — which is why §3 survived this long. The bar has an entirely separate lineage that is never cross-referenced with the row.

`SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md` is **active** with **Phase 2 (working-state vs long-running-process desync) explicitly not attempted**. It is the designated umbrella and the right home — but it currently has no awareness of the progress bar at all.

**Proposal:** this report is the consolidated analysis; the umbrella spec absorbs the design and gains a bar-aware Phase 2. Nothing is deleted — the fragments below stay as history, classified.

### 7.1 Index

**Working row / status text**
| Doc | Status | Disposition |
|---|---|---|
| `SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01` | Implemented | ACTIVE — current row behaviour, incl. dock stand-down |
| `SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08` | Draft (shipped) | Reference for the animation |
| `SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14` | Draft | Historical — original design |
| `SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24` | Implemented, §3.2 superseded | Partial history |
| `SPEC_AGENT_WORKING_ROW_TYPOGRAPHY_REFRESH_2026_09_03` | Proposed, no code | Open, unrelated to this bug |
| `SPEC_AGENT_WORKING_ROW_TOOL_BURST_REVEAL_INTERRUPT_2026_08_21` | Proposed | Open; + paired retro |

**Top progress bar** — *never cross-referenced with the above*
| Doc | Status |
|---|---|
| `SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10` | Implemented |
| `SPEC_AGENT_PANE_PROGRESS_BAR_OVERLAY_NO_GAP_2026_08_25` | Draft, partial reversal |

**Desync / stuck working state**
| Doc | Status | Disposition |
|---|---|---|
| `SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04` | **active, Phase 2 open** | **UMBRELLA — extend this** |
| `SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31` | partial | Names 5+ independent "still going" trackers |
| `SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29` | Design note | Background |
| `SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27` | Implemented | History |
| `SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30` | Implemented | History |
| `REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27` | Not implemented | **9 distinct false-"Working" paths catalogued** |
| `REPORT_WORKING_STATE_REGRESSION_AND_STUCK_QUESTION_PANEL_2026_07_27` | Not implemented | `settled` grace flag never built |
| `REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14` | Fixed (PR #2575) | History |
| `retro-persistent-agent-working-status-stuck-2026-07-16` | — | **Carries the §2.2 orthogonality correction** |

**Turn lifecycle**
| Doc | Status |
|---|---|
| `SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18` | Implemented — ACTIVE; `muxlog phases` is the diagnostic for this bug |
| `SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02` | active — `attachedTask` as a sibling axis |

**Backgrounding / dock**
| Doc | Status |
|---|---|
| `REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26` | Largely implemented; names `TOOL_PROMOTION_MS` |
| `REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16` | Superseded by the above |
| `SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06` | Implemented (PR #2432) — diagnosis only, **not** the fix |
| `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15` | — |
| `SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20` | Proposed |
| `SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23` | Implemented |
| `retro-stuck-background-dock-timer-2026-08-10` | Fixed #2519/#2520 |
| `REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25` | Analysis — "why the agent pane keeps regressing" |

### 7.2 Genuinely undocumented before this report

- The row and the bar as one system, and the requirement that they agree.
- Dock promotion desyncing them at 30s.
- Backgrounding leaving the input gate closed (the Esc). The only Esc doc, `SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06` (Draft), covers steering, not unblocking.
- Any notion of narrating an autonomous AgentMux action to the user.

## 8. Recommendations

Ordered by value-to-risk. P1 and P2 are independently shippable.

### 8.1 P1 — one boolean, three consumers

Per §2.3 there is exactly one meaning, so this is simpler than the three-copy
situation suggests. Define **one** predicate — "input typed now will be queued" —
export it from the pane model, and have all three consumers read that same value:

- the top bar (`agent-view.tsx:2296`)
- the working row (`agent-view.tsx:1630-1634`)
- the composer gate (`agent-view.tsx:2580`)

The composer's behaviour is the **definition** of the predicate, not a third
consumer of it: whatever decides that a typed message queues is the thing the two
indicators must render. If those ever disagree, the indicators are wrong by
construction.

Concretely this means:

- **Delete `supersededByDock()` from the indicator path.** Per §2.3 and §3.1 it is
  the divergence, not a feature. `SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01`
  must be corrected to match rather than worked around.
- **Fold `compacting`/`reconnecting` in properly** (§3.2): if input queues during
  them, both indicators light; if it does not, neither does. Today the row lights
  and the bar does not, which cannot both be right.

Model the extraction on `frontend/app/workspace/redock-arming.ts` — a gate that was
hand-implemented twice, collapsed into one tested module, with the duplicated
fallbacks deleted rather than retuned. Same shape of problem, same remedy.

Constraint from §2.2: this unifies the **indicator predicate only**. `turn_active`
and `turnPhase`/`attachedTask` remain distinct axes underneath.

### 8.2 P2 — backgrounding releases the foreground

On the existing `background-task-updated` path (§4.1), when every in-flight foreground call for a block has been either completed or accepted as backgrounded, the composer gate must open and the bar must stand down — with the dock row remaining as the live indicator.

The dock is already the right place to show ongoing background work; the pane simply has to stop *also* claiming the foreground.

Also wire `ScrubOrphanedInProgress` into `finalizeTurn()` (§5). It is built, tested, and dead. Smallest fix in this document.

### 8.3 P3 — ambient narration

New caller of `invoke_ambient_haiku_call` (§6.2), triggered where `publish_background_task_updated` already fires (`websocket.rs:765`).

**Honesty requirement — the part I would not ship without a decision.** The request was for a message that "appears like it is coming from the model". A Haiku-authored line rendered indistinguishably from the model's own output is a small lie told by the UI, and it will eventually mislead someone debugging a transcript — the model did not say it and its transcript will not contain it.

Recommendation: render it in the conversation flow, in the model's voice, but **visually and structurally marked as ambient** — a distinct node type carrying its provenance, the way `failure/synthetic-row.ts` already marks synthetic rows. It reads as continuous narration without claiming to be a model turn.

Implementation, cheapest first:
1. **View-only synthetic `DocumentNode`** mirroring `injectHistoryLink` (`inject-history-link.ts:25`) — never enters the CLI transcript, cannot corrupt context, disappears on reload. Recommended for v1.
2. Translator plumbing — real transcript entry. Higher cost, and it changes what the model sees on the next turn. Not recommended without a specific reason.

Cost control is already solved: the gateway has a 15s timeout, cancellation, and concurrency caps. Narration must be **best-effort and non-blocking** — if Haiku is slow or fails, the UI change in P2 must happen anyway. Never gate a UI state transition on a model call.

#### 8.3.1 Concrete landing points (researched 2026-09-10, not yet built)

**The trigger must come from the frontend, not the backend.** The obvious hook —
`COMMAND_DOCK_NODE_STATUS` (`agentmux-srv/src/server/websocket.rs` ~1257), where
`publish_background_task_updated` already fires on the background transition —
turns out to be unusable on its own: `CommandDockNodeStatusData`
(`agentmux-srv/src/backend/rpc_types/block.rs:145-158`) carries `tool_name` but
**not the command text**. The backend knows a Bash call went background; it
cannot say *what* went background, which is the entire content of the message.
The frontend holds the `ToolNode` params.

So the facility is an RPC the frontend calls with context, not an event the
backend originates:

1. **`COMMAND_AMBIENT_NARRATE`** — new constant in
   `agentmux-srv/src/backend/rpc_types/commands.rs` (pattern: `:67`), with
   `CommandAmbientNarrateData { block_id, kind, context }`. `kind` is what
   selects the prompt, and is what makes this reusable rather than
   background-specific.
2. **Handler** modelled on `generate_pushed_activity_summary`
   (`agentmux-srv/src/server/app_api/session.rs:295-316`): take an
   `AmbientCallKey` under a NEW purpose constant (do not share a purpose with an
   existing caller — see the `_PUSHED` comment at `:270-275` for why two callers
   sharing one purpose cancel each other), resolve `cmd` from block meta, call
   `invoke_ambient_haiku_call` (`session.rs:1022`).
3. **Broadcast** `ambient-narration` scoped `block:<id>`, mirroring
   `publish_background_task_updated` (`websocket.rs:765-773`) — but carrying the
   text, since unlike that invalidation ping there is no list query to re-read.
4. **Frontend** subscribes in `frontend/app/store/wps-events.ts` (pattern at
   `:69`) and renders a synthetic node via the `injectHistoryLink` mechanism
   (`inject-history-link.ts:25`) — render-time only, never dispatched into the
   document reducer.

**Trigger on the acceptance predicate, and do not add a survival floor.**
An earlier revision of this section claimed `isAcceptedBackgroundLaunch`
misclassifies fast-finishing calls and therefore needed a survival floor on top.
**That inverts the fact** (codex P2 on #3161). `isAcceptedBackgroundLaunch` is
the *fix* for #2518, not a victim of it: the raw `params.run_in_background` flag
was the unsafe signal — 11 of the 17 rows in that issue's own session were
fast-finishers wrongly treated as detached — and this predicate exists precisely
to exclude them by additionally requiring the acceptance prefix in the result
text (`tool-adapter.ts:84-115`). Narrating once per accepted launch inherits no
such noise, and a survival floor would only delay or suppress narration for
genuinely short-lived detached tasks. Dedupe per `node_id` (a node can be
re-observed) and nothing more.

**A cross-block concurrency cap IS required**, and does not come for free
(codex P2 on #3161). `AmbientGateway` deduplicates and cancels only within a
single `(block_id, purpose)` key, so N panes backgrounding at once means N
concurrent Haiku subprocesses. The pushed-summary caller this plan is modelled
on is bounded by a semaphore *external* to it
(`backend/reactive/activity_watcher.rs:129-137`); an RPC-driven caller inherits
none of that. Acquire a narration-specific semaphore before spawning — the 15s
timeout bounds each call's duration, not how many run at once.

### 8.4 P4 — the general facility

P3's node type, provenance marking and injection path *are* the general facility. Backgrounding is its first consumer. Other candidates already exist in-tree: the resume preflight and "Not signed in" rows are hand-rolled versions of the same idea and could migrate to it.

Do not build this abstractly ahead of P3. One real consumer first.

## 9. Verification

No test currently asserts the two indicators agree. That absence is why this regressed repeatedly.

| # | Scenario | Expected |
|---|---|---|
| 1 | Bash call crosses `TOOL_PROMOTION_MS` | **Neither indicator changes** — the call is still in flight, input still queues. Dock row appears alongside. |
| 2 | Compacting begins with no turn in flight | Both indicators reflect whether input queues — never one without the other |
| 3 | Call accepted as backgrounded | Composer gate opens; bar stands down; dock row live; **no Esc required** |
| 4 | Backgrounded task completes | Notification lands; indicators do not re-light spuriously |
| 5 | Harness pre-execution rejection (§5) | Orphaned node scrubbed at turn end; no permanent `running` row |
| 6 | Haiku unavailable / times out | UI transition still happens; narration silently absent |
| 7 | Esc mid-turn | Both indicators clear together, and input is accepted immediately afterwards |
| 8 | Type while either indicator is lit | Message queues ("wait until next turn") — the indicator never lies in either direction |
| 9 | Type while both are dark | Agent responds immediately |

1, 2, 5 and 6 are unit-testable against the extracted predicate and the reducers. 3, 4 and 7 need the pane. `muxlog phases` (`SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18`) merges the frontend `[wave-turn]` log with the backend `[health] turn_active flip` and is the tool for the manual passes.

## 10. Open questions

- ~~Should the bar and row mean the same thing?~~ **Answered (§2.3): yes, identically — they are the input gate made visible.** This also means `SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01` needs an explicit correction; see §2.3.
- **Should narration be persistent?** View-only (recommended) means it vanishes on reload. If a user is meant to scroll back and find out why a pane went quiet an hour ago, that argues for persistence — and for the higher-cost option 2.
- **Frequency.** Every backgrounded call, or only those that stay alive past some threshold? Narrating a task that finishes two seconds later is noise. The `isAcceptedBackgroundLaunch` misclassification (#2518) is precedent for how noisy the naive rule gets.
- **Do the nine false-"Working" paths in `REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27` still reproduce?** That audit predates several fixes. Re-validating it would tell us whether §8.1/§8.2 close them or whether more remain.
