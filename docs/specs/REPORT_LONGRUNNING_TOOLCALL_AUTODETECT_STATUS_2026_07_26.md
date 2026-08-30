# Report: auto-detecting long-running tool calls (sleep and beyond) and docking them — status refresh, 2026-07-26

**Status:** Report — audit + design synthesis, all open questions resolved (§4). **Largely implemented since; see the status table below (refreshed 2026-08-30).** This is **not** a from-scratch analysis: it verifies and consolidates two existing same-topic reports against `main` as of today, and updates them with what has (and hasn't) shipped in the 10 days since.

### §3 implementation status (verified against `main`, 2026-08-30)

| Step | State | Where |
|---|---|---|
| 1. Duration-threshold promotion | **shipped** | `activity/tool-adapter.ts`, `TOOL_PROMOTION_MS = 30_000` |
| 1a. `sleep <N>` → immediate dock + "~Ns left" | **shipped 2026-08-30** | `activity/sleep-detect.ts` — see the note below |
| 2. `run_in_background` end-to-end | **shipped** | issues #2490, #2518, #2491, #2492 |
| 3. Two-axis pane status | **shipped** | `activity/attached-task.ts`, `SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md` |
| 4. Feed the signal to Swarm | **partial** | `shellRows`/`cronRows` exist; dock-promoted Bash calls are still absent |
| 5. Generalize beyond Bash | **not started** | `isBashToolNode` still gates on `tool === "Bash"` |

**On 1a (§4.2's "cheap UX polish"), with the measurement §4.2 called for.**
That section asked for "real-transcript validation, not just Agent1's one
heartbeat example, before it ships." Done — 7,761 foreground Bash calls across
12 production transcripts:

- The **naive** rule §4.2 rejected ("command starts with `sleep`") matches 270
  calls, of which **204 (76%) are not waits** — `sleep 90; tail -30 <log>`,
  `sleep 60; ls <dir>`. The rejection was correct.
- Restricting to commands that are a wait **and nothing else** matches 66, every
  one genuine, median 61s. No false-positive surface: there is no second clause
  to be wrong about. 18 of the 66 finish under 30s, i.e. the duration rule
  misses them entirely.
- All 204 compound sleeps ran 28–100s, so duration already catches every one —
  ignoring them here costs nothing.

Shipped as exactly what §4.2 specifies: an optimization layered on top of
duration promotion, never the classifier. Also measured, and **rejected**:
lowering `TOOL_PROMOTION_MS` to 10s. Median foreground Bash is 7.5s, so 10s
lands on the steepest part of the distribution — 3.5× the dock rows, 21% of all
calls newly promoted, and identical commands flipping across the line depending
on cache state.
**Author:** Agent3
**Verified against:** `main` @ `45155f864` (pulled 2026-07-26).
**Supersedes/updates:**
- `docs/specs/REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md` (Agent2, 2026-07-16) — the direct prior version of this exact question, with a concrete 6a/6b design already worked out. **Everything in that report's §1–§7 was independently re-verified against today's code by a fresh audit and still holds true — nothing there is stale.**
- `docs/specs/REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16.md` (Agent3, same day) — the sibling report covering the Swarm-pane and subagent-watcher angles the user asked about here ("close collab with swarm pane").
- Background specs: `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md`, `SPEC_LONG_RUNNING_PROCESS_UX_2026_06_24.md` (the dock's existing design), `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` (the concrete incident this all generalizes from).

## User's request (verbatim, for traceability)

> lets refine long-running processes. we need to detect when the agent uses Bash sleep or anything like that. they would then become docked as long running task, and the "Working.." would be gone, and the user free to enter more commands. not sure how the simul processes work. audit the system currently, it includes close collab with swarm pane.

This is, almost word-for-word, the same request Agent2 investigated 10 days ago (see the quoted request in that report). §1 below is "what's changed since," §2 is a from-scratch-feeling but actually-just-consolidated current-state audit (touching the two things the prior reports split across two documents: the dock/tool-call side, and the Swarm-pane side), and §3 is one synthesized recommendation rather than two separate reports' worth of options.

## 1. What's shipped in the 10 days since the prior reports (2026-07-16 → today)

Checked via `git log --since=2026-07-16 -- frontend/app/view/agent/activity/ frontend/app/view/agent/components/ActivityDock.tsx frontend/app/view/swarm/`:

- **#2293** — auto-clear error activity-dock rows after 15s, landing/departure flash. A retention-timing polish on the *existing* dock, unrelated to detecting new activity kinds.
- **#2203** (already described in the 07-16 report as "landed same day," confirmed still the shipped shape) — subagent flood fix: one dock row per Agent/Workflow-tool call, not per individual subagent, via `groupSubagentsByWorkflow`.
- **#2232 / #2208** — Swarm pane's own "two-bucket Agent Tool / Workflow row model" — a Swarm-side grouping refinement, same spirit as #2203 but on the Swarm tree.

**None of this touches the actual ask.** All three are quality-of-life fixes to the *existing* shell+subagent dock/Swarm mechanisms. The specific gap both prior reports identified — nothing detects a long-running/backgrounded **ordinary Bash tool call** and turns it into a dockable, Swarm-visible activity — is completely untouched. `BashParams` (`frontend/app/view/agent/types.ts:122-125`) still has only `{ command: string; timeout?: number }`; `ShellNode` (`types.ts:377-388`) still has no background/duration flag. Re-verified directly today, not assumed from the old report.

## 2. Current-state audit (consolidated)

### 2.1 How a Bash call actually runs — AgentMux never sees inside it

AgentMux doesn't implement Claude's Bash tool — Claude Code (or Codex/Gemini/Qwen) runs as a full CLI subprocess over a real PTY (`agentmux-srv/src/backend/blockcontroller/shell/lifecycle.rs`, `ShellController::start`), and the CLI's own tool-calling — including its Bash tool, including `run_in_background` — happens *inside* that opaque subprocess. AgentMux's spawn call returns immediately once the PTY task is running; there is no per-tool-call subprocess boundary AgentMux controls or waits on. Everything AgentMux "knows" about an in-flight Bash call is re-derived by parsing the CLI's own stream-json NDJSON output line-by-line (`useAgentStream.ts`), not by owning the process.

Concretely, the Bash command string is **already flowing to the frontend today** as `event.params.command` on the `tool_call` NDJSON event (`extractToolArg`, `useAgentStream.ts:47-48`), landing in `state.currentToolArg` via the `ToolStart` reducer command. **Detecting "this call is a sleep" needs zero backend/wire changes** — it's a pure frontend string match against data already present.

### 2.2 `run_in_background` — exists only in the CLI's own tool schema, nowhere in AgentMux

Grepped fresh across `agentmux-srv/src`, `agentmux-mcp/src`, `agentmux-common`, and `frontend`: **zero hits** for `run_in_background`/`RunInBackground`. It's a parameter of the *outer coding agent's* Bash tool (confirmed live in the Agent1 incident's own transcript, see the retro) — AgentMux's own code never reads, threads, or reacts to it.

AgentMux's own answer to "I want a long-running thing" is a **different, explicit tool**: the `Shell` MCP tool (`agentmux-mcp/src/main.rs:57-71`, "Start a long-running shell process... for build systems, watchers, dev servers"), which the agent must deliberately call instead of Bash. This produces a `ShellNode`, which is what the dock already tracks. So today there are **two structurally different ways an agent creates a long-running thing**, and only one (the explicit `Shell` tool) is visible anywhere in the UI. A `sleep 300` or a `run_in_background: true` dev-server launch via the *ordinary* Bash tool is invisible to AgentMux end-to-end — not just undocked, genuinely untracked.

### 2.3 Process tracking (`AgentProcessRegistry`) — membership only, no duration/classification

`agentmux-srv/src/backend/process_tracker/registry.rs`: `AgentProcessRegistry` is a `block_id → RegistryEntry` map (one Job Object/cgroup/pgroup per agent pane), polled every ~2s, diffing PIDs and emitting `agent:process-added`/`agent:process-exited` WPS events. `TrackedProcess { pid, command, rss_bytes, started_at_ms }` — but `started_at_ms` is **hardcoded to `0` on Windows today** (`windows.rs:198`, explicitly "deferred; skip for v1"), so process *age* isn't even populated yet, let alone used to classify anything. `TrackingConfidence { High, BestEffort, None }` is about mechanism reliability (can we trust the list is complete), not about the process's nature. This is the mechanism behind `AgentComposerStrip.tsx`'s "⚙N" badge (click → opens Swarm) — **a third, separate pipe** from both the dock (shell/subagent) and the Swarm pane's own agent/subagent roster. Today all three surfaces answer overlapping-but-different questions and none of them talk to each other.

`InstanceStatus` (`agentmux-srv/src/backend/storage/agents.rs:150-156`: `Running/Paused/Stopped/Crashed/Detached`) is a coarse instance-lifecycle status, orthogonal to "busy in a tool call right now" — no help here either.

### 2.4 Turn-phase / "Working…" — tied 1:1 to the CLI's own turn boundary, no early-exit

`TurnPhase` (`frontend/app/store/agent-pane-state/types.ts:133-171`): `Idle | Submitting | Streaming | Interrupting | Done | Disconnected`. `workingFromPhase` (`types.ts:332-335`) is `true` for exactly `Submitting | Streaming | Interrupting`. The only path out of a working phase is `TurnEnd`, fed exclusively by the CLI's own top-level `result` event (via `claude-translator.ts`'s `session_end` → `useAgentStream.ts`'s `finalizeTurn()`) — i.e. the *whole turn's* own boundary, not any individual tool call's.

`ToolStart`/`ToolEnd` (`reducer.ts:560-580`) only mutate `currentTool`/`currentToolArg`/`toolsActive` on the `Streaming` phase's payload — **they never end the phase**. So a `sleep 300` inside an otherwise-ordinary turn just makes the entire `Streaming` phase, and `AgentWorkingRow`'s "Working…" banner, run for 300 seconds. The three bounded watchdogs that *can* force a phase transition (`SubmitTimeoutElapsed` 30s, `InterruptTimeoutElapsed` 5s, and the liveness-recovery arm of `StreamWatchdogTick` at `LIVENESS_RECOVERY_MS = 180_000` / 3 min) are failure-detectors, not this feature's mechanism — and the liveness one **explicitly refuses to fire while any tool is active** (`toolsActive === 0` gate, `reducer.ts:309`), so a running tool call — sleep included — is treated as proof-of-life and can pin "Working…" indefinitely. This is exactly the Agent1 incident's root cause (a `sleep`-based heartbeat kept a pane "Working…/Waiting…" for ~12 hours, healthy the entire time).

**Important, already-resolved sub-question:** is the composer actually *locked* during this? **No.** `AgentFooter.tsx`'s textarea has no `disabled`/`loading` gate; `PendingMessageQueued` (`enqueuedWhileBusy: true`) already lets a user type and send while a turn is in flight, rendered in the existing amber "Queued" zone of `PendingMessagesPanel.tsx`. So "free the user to enter more commands" is **not** an input-unlocking problem — it already works. The actual gap is purely the *status narrative*: the pane visually reads as "busy, don't bother it" for the full duration of what might be a deliberate multi-minute wait, discouraging use of an already-available composer.

### 2.5 The dock — already built, already does the "docked info, remove on completion" lifecycle, just for the wrong set of activities

`ActivityDock.tsx` + `activity/{types,shell-adapter,subagent-adapter}.ts`: a strip pinned above the composer, unifying `shell`/`subagent` (and a declared-but-unbuilt `cron`) kinds into one `PinnedActivity` abstraction, with running-first ordering, per-status retention (`RETENTION_MS`: done 8s, stopped 3s, error until dismissed, running forever), and an "N more" overflow toggle. **This is already exactly the "docked info" + "remove when complete" mechanism the user is asking for** — it just has no adapter that reads `ToolStart`/`ToolEnd` for an ordinary Bash call, so a `sleep` or a backgrounded dev-server launch via the plain Bash tool never becomes a `PinnedActivity` no matter how long it runs.

### 2.6 Swarm pane — the "close collab" the user specifically flagged

`frontend/app/view/swarm/swarm-view.tsx`/`swarm-model.ts`: shows a roster of tracked agent panes (`AgentTrackedBlocksCommand`, refreshed on `agent:process-added`/`-exited`/`agent:reactive-registered`/`-unregistered`) plus their subagents (`subagent:spawned`/`-completed`), each row's status derived from `ControllerStatus`/`turn_active` — i.e. it renders the **same turn-phase axis** §2.4 describes, with the same blind spot. The "14 idle" pattern seen in a screenshot is a context-token badge next to the status chip, not a process count — worth noting since it's easy to misread as "14 background processes." The Swarm pane today has **zero visibility into attached long-running processes or backgrounded Bash tasks** — neither a shell, nor a `run_in_background` task, nor even the `AgentProcessRegistry`'s own `processCount` surfaces there directly (the `⚙N` badge just opens the Swarm pane; it doesn't hand off *which* process to look at).

This is exactly the gap the sibling 07-16 consolidation report named: **the pane, the dock, and Swarm each answer "what is this agent doing" from a different, non-composing vantage point**, all missing the same attached-process axis. Fixing it in one place (the dock) without also feeding Swarm leaves the fleet-wide view blind to precisely the thing a user managing several agents most wants to spot at a glance: "which of my agents is sitting on a live dev server / sleep / background task right now."

## 3. Synthesized recommendation

Both prior reports converge on essentially the same phased shape; here it is as one sequence, reflecting today's still-open state and revised per §4's resolutions:

1. **Duration-threshold promotion (~20–30s, reusing the number `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md` §5 already specified) → minimal new dock kind.** Any Bash tool call still running past the threshold gets promoted to a `PinnedActivity` via a new `activity/tool-adapter.ts` mirroring `shell-adapter.ts`, regardless of command text — no regex classification needed as the primary mechanism (§4.2). Remove on `ToolEnd`, inheriting the dock's existing retention lifecycle for free. Layer the `sleep <N>` text heuristic on top as a pure UX nicety (show "~Ns remaining" immediately for the unambiguous case instead of waiting for the threshold), not as the classifier. Cheapest slice, ships independent of everything below, and generalizes "beyond sleep" from day one instead of needing a separate later phase.
2. **Thread `run_in_background` through, end to end** (prior report's "6b" — the CLI's own self-declared "this is backgrounded" signal, complementary to (1) since a backgrounded task's *initiating* tool call resolves near-instantly and would never cross (1)'s duration threshold on its own — see §4.1). Now confirmed tractable with no protocol unknowns: `BashParams` gains `run_in_background?: boolean` read straight off the already-flowing tool_call params; two new `extractToolArg` cases for `BashOutput`/`KillShell`; a `background: boolean` flag on whatever node represents the resulting task; wired into the same dock adapter as (1).
3. **Two-axis pane status.** Add the attached-process axis `workingFromPhase` is missing: while ≥1 dock-tracked activity is live and the turn itself is `Idle`, `AgentFooter` shows a calm "Running: `<task>` (elapsed)" instead of nothing, and — critically, this is the Agent1 fix — **such panes are exempted from the liveness watchdog's quiet-means-hung recovery**, so a legitimately-waiting pane is never mislabeled hung, and a `sleep`-based heartbeat can no longer masquerade as "the model is generating."
4. **Feed the same signal to Swarm** — this is the piece the user specifically called out and neither prior report fully closes: once (1)–(3) produce a real "this agent has N live background/long-running activities" signal per pane, surface it as a chip on the Swarm pane's per-agent row (same data, same adapter output, new renderer) so the fleet view answers "which agents hold live dev stacks/sleeps right now" without opening each pane. Given `SwarmViewModel` already merges tracked-block + subagent data from the same WPS-event family the dock uses, this is additive rendering on an existing model, not a new pipe.
5. **Generalize beyond Bash** — step 1's duration threshold is tool-agnostic by construction (it keys on "still running," not on being a Bash call specifically), so extending promotion to any long-running tool call (not just Bash) once (1)–(4) prove the pattern out is a small follow-on, not a redesign.

Steps 1–3 are almost verbatim the prior report's own §7/§8 (still valid, still unimplemented); step 4 is new synthesis directly answering the Swarm-collaboration half of this session's request, which the 07-16 split across two documents never fully closed the loop on.

## 4. Open questions — resolved

### 4.1 Where does a `run_in_background: true` task's handle surface in the stream?

Resolved by re-reading `claude-translator.ts` and the Agent1 retro's own transcript excerpt together — no live repro was needed, the answer was already on disk in two places:

- **The retro's transcript analysis already confirms the shape**: *"the tool call returns a task handle immediately (correct)"* — a `run_in_background: true` Bash call is **not** a tool call that stays open/pending for the task's lifetime. It completes near-instantly in the transcript sense (handle returned), the turn continues normally, and the spawned process keeps running fully detached from that tool call's lifecycle. There is no "poll a live handle mid-stream" problem to solve, because nothing in the stream stays mid-flight.
- **The ongoing liveness signal is NOT stream-based at all** — it's the OS process itself, already tracked (separately, today, for cleanup purposes) by `AgentProcessRegistry` (§2.3). The ask isn't "read a handle out of the NDJSON stream," it's "notice this pane has a live tracked OS process while its turn is otherwise idle" — which is exactly step 3's two-axis status, not a new stream-parsing problem.
- **Whatever the CLI puts in that initiating call's `tool_use_result` survives untouched to the frontend.** `buildToolResults` (`claude-translator.ts:308-346`) passes the raw structured sibling through as-is (`result: useStructured ? structuredResult : fallback`) — `BashResult`'s `{stdout, stderr, exitCode}` shape is a TypeScript *annotation*, not a runtime filter, so extra fields (a task-handle string, if Claude's CLI includes one) aren't stripped, just untyped and unrendered.
- **The one genuine, confirmed-today gap**: `extractToolArg`'s switch (`useAgentStream.ts:59-82`) has no case for `BashOutput`/`KillShell` — Claude's own companion tools for polling/killing a backgrounded task — so if the model ever calls them, they fall through to the generic default (which tries `file_path`/`path`/`command`/`query`/`pattern` — none of which match `BashOutput`'s `bash_id`/`KillShell`'s `shell_id` params) and render with no useful arg text. This is a one-switch-case fix, not a protocol integration.
- **Remaining, honestly-unresolved uncertainty**: the *exact* field name the CLI's `tool_use_result` uses for that task handle on the initiating call isn't confirmed from a literal JSON example in this repo (the retro quotes the *command*, not the raw result object). Low-risk to leave open — step 2's implementation only needs `params.run_in_background === true` on the **input** side (confirmed present — the retro's own transcript quotes it verbatim on the tool call), not the result shape, to flag a task as backgrounded.

**Conclusion: step 2 is more tractable than the original report assumed.** No backend/protocol investigation needed — it's `BashParams` gaining `run_in_background?: boolean` (read straight off the already-flowing tool_call params) plus two new `extractToolArg` switch cases, feeding the same dock adapter as step 1.

### 4.2 Sleep-heuristic false positives

Re-examined against a concrete real example already on file: this session's own `( while true; do sleep 25; echo "[heartbeat]..."; done & task dev ...)` wrapper (used earlier in this same conversation to keep a dev server's background task alive across the sandbox's idle-output timeout) — structurally identical to the Agent1 incident's heartbeat shape. Both are genuine "this is fundamentally a wait, loop forever" commands that should dock; both would also **still be running** past any reasonable fixed text-only regex's confidence — which points at a better design than pure text-matching:

**Recommendation: promote on duration, not text pattern, as the primary mechanism** — "any tool call still running past ~20–30s" (the existing `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md` §5 already specifies "~30s" for its own, still-unbuilt "overrun promotion" — reuse that number for consistency) **regardless of command text**. This sidesteps the false-positive problem almost entirely: `sleep 2 && rm -rf /tmp/staging` finishes in ~2s and never crosses the threshold, so it's never misclassified as a long wait, with zero regex tuning required. A genuine `sleep 300` or a dev-server launch crosses the threshold at exactly the point where showing "Working…" stops being informative, whatever the command's text says.

The text heuristic (bare `sleep <N>`, `timeout <N> sleep`, etc.) is still worth keeping, but demoted to an **optimization, not the classifier**: when a command is recognizably `sleep <N>` for a large `N`, the dock row can show "~Ns remaining" immediately instead of waiting for the duration threshold to elapse blindly — a nicer UX for the unambiguous case, layered on top of, not instead of, the duration-based promotion that correctly handles everything else (dev servers, long builds, unrecognized wait-shaped commands, and any future heuristic-evasion case) with no false-positive surface at all. Original report's §6a becomes "phase 1a, cheap UX polish"; duration-based promotion becomes the actual correctness mechanism, generalizing "beyond Bash and sleep" (§3 step 5) for free instead of needing a separate later phase.

### 4.3 Coexist or replace: `AgentWorkingRow` vs. the dock, once tracked

Resolved: **the dock takes over, `AgentWorkingRow` goes calm/neutral for that pane** — directly matching the user's own wording ("the 'Working..' would be gone"). Rationale: Claude Code (like the other supported CLIs) runs one tool call at a time within a turn — once the pane's single in-flight tool call is promoted to a dock row, there is no *other* concurrent activity left for `AgentWorkingRow` to usefully describe; showing "Working · sleep 300" in the working row **and** "sleep 300 (42s)" in the dock simultaneously is exactly the duplication the original report's §7.2 flagged as an open question, now resolved: don't duplicate, hand off. Concretely: `workingFromPhase` stays `true` at the reducer level (the turn genuinely hasn't ended — this must not regress `TurnEnd`/session bookkeeping), but `AgentWorkingRow`'s **rendering** should suppress its own text once the currently-tracked tool has a live dock entry, falling back to no banner (or the existing `Done`-state visual) rather than repeating the same information twice.

### 4.4 Tracker hygiene — checked directly, one item already resolved

- **#101** ("Swarm Orchestration" epic) — **already closed**, as of 2026-07-24 (2 days before this report), confirmed via `gh issue view 101`. The consolidation report's "close as superseded" recommendation has already been acted on by someone; no action needed.
- **#1814** ("tracking: agents managing long-running commands") — confirmed still **open**, last updated 2026-06-27 (a month stale). Read its full body directly: it's the right umbrella issue (Shell MCP tool phases, ActivityDock, cron/loop robustness), but every item on it is scoped to the **explicit** `Shell`/Loop/Cron tools — it does not yet mention auto-detecting an *ordinary* Bash call at all. Recommendation unchanged from the consolidation report: refresh this issue (check off shipped sub-items, add a new section for the auto-detect/two-axis-status/Swarm-collab work this report and its predecessor describe) rather than opening a second, competing tracker. Not actioned in this pass — editing a shared, visible GitHub issue is a call for whoever picks up implementation, not something to do unilaterally while still at the report stage.

## 5. Key files

| File | Role |
|------|------|
| `frontend/app/view/agent/components/ActivityDock.tsx` + `activity/{types,shell-adapter,subagent-adapter}.ts` | The dock — already the target abstraction for a new tool-call/background kind |
| `frontend/app/store/agent-pane-state/{reducer,types}.ts` | Turn-phase state machine; `ToolStart`/`ToolEnd`, `workingFromPhase`, the liveness watchdog thresholds |
| `frontend/app/view/agent/useAgentStream.ts` | `extractToolArg` (Bash command already flows here), `ToolStart` dispatch site |
| `frontend/app/view/agent/components/AgentFooter.tsx` | `AgentWorkingRow` — today's only "what's running" UI; where a calmer/two-axis status lands |
| `frontend/app/view/agent/types.ts` | `BashParams` (no `run_in_background`), `ShellNode` (no background flag) — both still-open gaps, re-verified today |
| `agentmux-srv/src/backend/process_tracker/{registry,mod,windows}.rs` | OS-process membership tracking — no duration/classification, `started_at_ms` unpopulated on Windows |
| `frontend/app/view/swarm/swarm-view.tsx` / `swarm-model.ts` | Swarm roster + status derivation — the collab surface this session's request specifically named |
| `agentmux-mcp/src/main.rs` (`Shell` tool) | AgentMux's existing, explicit "long-running process" tool — the only backgrounded-thing kind visible anywhere today |
| `docs/specs/REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md` | Prior deep-dive this report verifies and updates — §6/§7/§8 (heuristic design, UX design, phasing) still fully valid |
| `docs/specs/REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16.md` | Sibling report — the Swarm/subagent-watcher half of this same initiative |
| `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` | The Agent1 incident — concrete precedent for step 3's watchdog-exemption fix |
