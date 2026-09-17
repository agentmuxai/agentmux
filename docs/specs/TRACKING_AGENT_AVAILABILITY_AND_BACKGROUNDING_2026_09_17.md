# TRACKING — Agent availability & automatic backgrounding

**Date:** 2026-09-17
**Type:** Tracking doc — canonical location for this problem family. Not a design spec.
**Status:** Live. Update this file when anything below changes state.
**Owner prompt (verbatim, 2026-09-17):**

> anything the agent does that is long running or runs in the background gets a composer dock. if the agent is only working on those composer dock entries (not busy on anything else) if the user types in a message, the agent immediately handles it. it is simply backgrounding anything long-running automatically, keeping the agent available.

---

## 0. Why this file exists

This territory has been consolidated **twice** and never got a tracking home:

- `SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md` — "five independent mechanisms that each answer some version of *is this thing still going?*, none of which share a common source of truth."
- `REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md` §3.1 — the same finding, arrived at independently, four subsystems.

Both diagnosed it. Neither became somewhere you could look up *what is true right now*. That report's own §5 Tier 3 names the failure mode precisely:

> Of the 8 "we should fix this properly later" notes accumulated across this incident family, only 1 was ever picked back up... Worth deciding, as a team norm, whether a deferred-fix note should get a tracked follow-up item rather than living only inside a retro doc nobody revisits.

This is that item. **Tracking issue: #3338.**

---

## 1. The invariant

One sentence, and every indicator, gate and dock row is a rendering of it:

> **If the agent has nothing in the foreground that blocks a new message, a message typed now is answered immediately.** Long-running work does not count as blocking once it has been accepted as backgrounded; it stays visible as a dock row.

Encoded in `frontend/app/view/agent/working-indicator.ts` as a single predicate. The progress bar, the `Working…` row and the composer gate are three renderings of that one boolean — **any state where they disagree is a bug by construction**, not a judgement call.

## 2. The correction that keeps getting re-learned

**A dock row does not mean the call was backgrounded.** Three separate documents have had to say this; two implementations shipped against the wrong version of it.

- **Promotion ≠ backgrounding.** At `TOOL_PROMOTION_MS` (30s) a long-running call gets a dock row. That is a **display** change. The call is still in the foreground and the turn is still blocked on it.
- **AgentMux does not make the backgrounding decision at all.** The Claude Code harness does, per call — via `run_in_background: true`, or its own timeout auto-background. AgentMux learns about it *afterwards*, by string-matching the tool result against `BACKGROUND_LAUNCH_ACCEPTED_PREFIX` (`activity/tool-adapter.ts:57`).
- Therefore the dock holds **two structurally different kinds of row**, and only one of them means the agent is free:

| Dock row | Agent actually available? |
|---|---|
| harness accepted it as background | **yes** |
| promoted at 30s for visibility only | **no — turn still blocked** |

The live predicate distinguishes them correctly (`hasAttachedBackgroundWork` vs `hasBlockingForegroundToolCall`). Do not collapse them.

**Where this bit us:** `SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01` §1 shipped a stand-down keyed on promotion, on the assumption promotion meant backgrounded. It didn't. The row vanished at exactly 30s into every long Bash call while the bar kept running — the two indicators disagreed and the row under-reported a gate that was still closed. Reverted 2026-09-09.

## 3. State of play

### 3.1 Shipped and merged

| | |
|---|---|
| Background-task registry Phase A — PID capture | #2490 · `SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md` |
| Phase B — teardown survival (a dev server survives session restart) | #2492 |
| Phase C — durable registry as a real dock read-source | #2685 · `SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md` |
| Fast-finish misclassification fix | #2518 / #2519 |
| Background shell output no longer pins a turn in `Working` | #3158 |
| One predicate behind all three indicators | #3143 |
| Backgrounded tasks narrated into the conversation | #3169 |
| Turn-phase timeline logging (`muxlog phases` — the diagnostic for this family) | `SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md` |

### 3.2 Implemented but NOT landed — at risk

**The 2026-09-17 policy reversal (§2.3a) was uncommitted working-tree WIP** when this doc was written — no branch, no PR, no commit, in one clone only. This is the change that makes §1's invariant true: the composer reopens once only dock work remains. It is the single highest-value unlanded item in this family.

- **Its docs half is now preserved** — the §2.3a writeup in `REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md` rode in with this tracking commit.
- **Its code half is still uncommitted** and deliberately NOT in that commit, because it needs its own review:

```
frontend/app/view/agent/activity/tool-adapter.ts
frontend/app/view/agent/agent-view.tsx
frontend/app/view/agent/working-indicator.ts
frontend/app/view/agent/working-indicator.test.ts
```

Land those four first — everything else in §3.3 assumes them.

### 3.3 Open work

| # | Item | Depends on | Notes |
|---|---|---|---|
| 1 | **Land §3.2's WIP** | — | Commit, PR, review, merge. Everything else assumes it. |
| 2 | **Pre-dispatch auto-backgrounding** | none | The `PreToolUse` hook already exists (`agent_config.rs`, `agent_handlers/input.rs`) and `muxspect.mjs` already *reads* `run_in_background` — but only to print a `bg` column. Making it *write* `run_in_background: true` for obviously-long commands is what makes the owner's sentence true by construction rather than by luck. Needs a false-positive policy first — see `SPEC_FOREGROUND_BACKGROUND_PROCESS_ABSTRACTION_2026_08_20.md` §5.1. |
| 3 | **Mid-flight toggle — feasibility spike** | none | **Blocked on an unanswered question, not on design.** Agent panes are hosted via `PersistentSubprocessController`: no PTY, stream-json, `send_input` explicitly rejects raw bytes. Claude Code's `Ctrl+B` is documented interactive-TUI-only. Whether *any* mechanism reaches a stream-json-hosted session is unverified. Time-boxed spike, then scope — or drop. §1 of the same spec. |
| 4 | **The missing `live`/`replay` wire flag** | none | The root cause behind repeated dock flicker. `subagent_watcher`'s `jsonl.rs` already computes `live: bool` and never puts it on the wire, so two teams independently built two different local workarounds for the same gap (an event-count debounce; a bespoke status-correction RPC). One generic field removes both. `REPORT_..._ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md` §3.3, Tier 1. |
| 5 | **Reconcile durable registry rows with transcript-derived dock rows** | — | The two are never reconciled, so a background task that survives a restart — the registry's whole reason for existing — may not appear as a dock row. Use the existing union-with-earliest-start-wins pattern, not a new one. §3.2 of the same report. Partly addressed by #2685; branch `agent1/activity-dock-backfill-and-registry-reconcile` is unmerged — **verify whether its content landed via squash before redoing it.** |
| 6 | **Re-validate the nine false-`Working` paths** | 1 | `REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` catalogued nine. The audit predates several fixes; #3167 status-passed them. Confirm which survive item 1. |
| 7 | **bashwrap idle-timeout vs. declared long-running tasks** | — | Open issue #2215. The 600s PTY-silence kill can't tell "hung" from "succeeded and went quiet" — forces the documented heartbeat workaround, which has now been independently rediscovered three times. `REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26.md`. |

### 3.4 Explicitly not doing

- **Unifying the four backend "what's running" subsystems into one model.** The domain split (shells / subagents / promoted tool calls / OS processes) is legitimate and the last-mile adapter shape is sound. `REPORT_..._2026_08_25.md` §3.4 is explicit: *"the actual gap is one missing signal plus two specific unreconciled duplications, not the overall shape."*
- **Making all tool calls async by default.** `Read`/`Edit`/`Grep` need a real result for the model's next step. Backgrounding is an exception for Bash-shaped work, not a default.
- **A background → foreground restore action.** Claude Code has no such mechanism; not worth inventing independently.

---

## 4. Document index

Nothing is deleted. Superseded docs keep a banner pointing here.

### Canonical
| Doc | Role |
|---|---|
| **this file** | tracking, status, open items |
| `SPEC_FOREGROUND_BACKGROUND_PROCESS_ABSTRACTION_2026_08_20.md` | the model + the §1 feasibility question. *Was stranded on a deleted branch and never merged; rescued 2026-09-17.* |
| `SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md` | active design spec for the working-state workstream (Phase 2 open). Remains the design home; this file is the tracking home. |
| `REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md` | the analysis, incl. §2.3a. Read §2.3a before §2.3. |
| `REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md` | why this area regresses; the Tier 1 recommendations |

### Superseded in part — read the banner first
| Doc | What's dead |
|---|---|
| `SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md` | §1 reverted (promotion ≠ backgrounding). §2 still current. |
| `REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md` | §2.3 + §3.1 superseded by §2.3a. |
| `SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md` | §3.2 |
| `SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md` | entirely |
| `REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md` | superseded by the 07-26 autodetect report |
| `REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` | its "`run_in_background` threading still open" note is stale — that work shipped (#2490/#2518/#2520) |

### History — accurate for their date, not current
`SPEC_PROCESS_AND_TURN_STATE_TRACKING_CONSOLIDATION_2026_07_31.md` (names the 5 trackers) ·
`SPEC_WORKING_STATE_LIVENESS_MODEL_2026_06_29.md` ·
`SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md` ·
`SPEC_PERSISTENT_TURN_END_TEXT_GATE_2026_07_30.md` ·
`SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md` ·
`SPEC_MUXSPECT_DOCK_DIAGNOSIS_AND_REMEDIATION_2026_08_06.md` (diagnosis only, not the fix) ·
`SPEC_ACTIVITY_DOCK_REFRESH_COALESCING_2026_08_23.md` ·
`REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` (the nine paths — see item 6) ·
`REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14.md` (fixed, #2575) ·
`REPORT_BASHWRAP_LONGRUNNING_PROCESS_DETERMINISM_2026_07_26.md` (see item 7) ·
retros: `retro-stuck-background-dock-timer-2026-08-10` · `retro-activity-dock-flicker-survives-debounce-fix-2026-08-24` · `retro-persistent-agent-working-status-stuck-2026-07-16` (carries the turn-phase/liveness orthogonality correction)

### Not in scope despite the name
The ~15 `SPEC_COMPOSER_STRIP_*` docs are about the strip's visual layout. The `*_REDOCK_*` and `BUG_MACOS26_DUAL_DOCK_ICON` docs are about window docking and the OS dock icon — a different "dock" entirely. `SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04` / #2977 is the OS-level background service, unrelated to tool-call backgrounding.

---

## 5. Answer to the framing question

**No, this does not need a new architecture.** The mental model in the header is already the design; §1's invariant is already encoded as one predicate; the registry that makes backgrounded work durable already shipped. What's missing is one unlanded change (§3.2), one small hook (item 2), one feasibility answer (item 3), and one wire field (item 4). The 08-25 report reached the same verdict independently and said so in as many words: not a rewrite.
