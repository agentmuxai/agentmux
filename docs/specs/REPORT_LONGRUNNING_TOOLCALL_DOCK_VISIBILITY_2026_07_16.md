# Report: detecting blocking/long-running tool calls (sleep and beyond), returning the pane to the user, and dock lifecycle — 2026-07-16

**Status:** Report — analysis + design direction, not yet implemented (one adjacent live bug found and fixed en route, see §5).
**Author:** Agent2
**Verified against:** `main` @ `3d1ce73c` (pulled 2026-07-16), with PR #2201 (dock subagent-grouping fix, same session) on top.
**Related:**
- `docs/specs/REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16.md` (Agent3, same day) — the consolidated tracker for long-running processes / subagents / Swarm. This report is a deep-dive on that report's Area 1, specifically the "detect a blocking wait and get it out of the turn's critical status path" angle the user asked about directly.
- `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` (the "Agent1" incident) — the concrete, already-diagnosed case this report generalizes from: a `sleep`-based heartbeat loop inside a backgrounded Bash task pinned a pane's status to ambiguous "Working…/Waiting…" for ~12 hours.
- `docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md` / `SPEC_LONG_RUNNING_PROCESS_UX_2026_06_24.md` — the existing dock's design docs.

## User's request (verbatim, for traceability)

> we want to refine how longrunning processes work .. we have a dock system that appears above the conversation bar. analyze the state. we need to detect when a sleep is used by the agent .. in that case, the UI would need to return conversation to the user, and a docked info showing that tool call. similar with every agent call. when they are complete, remove them from the dock.

Three asks: (1) analyze the current dock system's state — done, §1–§4 below; (2) detect a `sleep`-flavored tool call specifically and generalize to "every agent call" — design direction in §6; (3) "return conversation to the user" + dock the call + remove it on completion — mechanics analyzed in §3/§4, design in §7.

---

## 1. What "the dock" is

`frontend/app/view/agent/components/ActivityDock.tsx` — a strip **pinned above the composer** (the message input bar), listing every long-running activity a pane's agent has spawned, as uniform rows (`ActivityRow.tsx`). Two kinds exist today:

- **`shell`** — a persistent PTY the agent explicitly launched via the AgentMux **`Shell` MCP tool** (`agentmux-mcp`), represented in the transcript as a `ShellNode` (`frontend/app/view/agent/types.ts:376`). Explicitly **not** tied to a tool-call lifecycle — the shell keeps running after the tool call that launched it returns. Adapter: `activity/shell-adapter.ts`.
- **`subagent`** — a Task-tool or Workflow-tool child, sourced from the backend subagent watcher via a shared app-lifetime singleton (`activity/subagent-source.ts`) and adapted in `activity/subagent-adapter.ts`.
- **`cron`** — declared in the `ActivityKind` union (`activity/types.ts:19`) but has **no adapter**. Not built.

Row lifecycle (`ActivityDock.tsx`): running rows always show; terminal rows (`done`/`stopped`/`error`) linger for a per-status retention window (`RETENTION_MS`: done 8s, stopped 3s, error until dismissed, running ∞) then auto-drop. Ordering: running-first, then by status, then expanded-first, then newest-first. Overflow beyond `MAX_INLINE = 3` collapses behind a "N more" toggle.

**What is conspicuously absent from this list: ordinary tool calls.** A `Read`, `Edit`, `Grep`, or a plain foreground `Bash` call is a `ToolNode` in the transcript (`types.ts:210`) — it never becomes a `PinnedActivity` and never appears in the dock, no matter how long it runs.

## 2. How an in-flight tool call is represented today (the thing that isn't the dock)

Separately from the dock, the pane-state reducer (`frontend/app/store/agent-pane-state/`) tracks **one** currently-running tool per pane:

- `ToolStart { name, arg }` sets `state.currentTool` / `state.currentToolArg` and increments `toolsActive` on the `Streaming` phase (`reducer.ts:588`).
- `ToolEnd` clears both and decrements.
- `useAgentStream.ts:880` dispatches `ToolStart` straight off the NDJSON `tool_call` event; `extractToolArg` (`useAgentStream.ts:37`) pulls the human-relevant argument per tool — for Bash, that's the **raw shell command string**, already flowing to the frontend today.
- `AgentFooter.tsx`'s `AgentWorkingRow` renders `currentTool`/`currentToolArg` as the left-zone status text while `loading` (`isWorking(state)`) is true: `"Bash · sleep 30"`, indistinguishable in kind from `"Read · foo.ts"` or `"Working…"` with no tool at all.

This is the entire "in-flight tool call" UI: one ephemeral status-line string, live only while the reducer's `turnPhase.kind ∈ {Submitting, Streaming, Interrupting}`. It disappears the instant the tool ends or the turn ends; nothing about it is dockable, stoppable, or independently trackable.

## 3. Is the composer actually "locked" during a tool call? — No.

Checked directly: `AgentFooter.tsx`'s textarea has no `disabled`/`readOnly` binding on `isWorking`/`loading`. The reducer's `PendingMessageQueued` command exists specifically to let a user type and send **while a turn is already in flight** (`enqueuedWhileBusy: true` on the entry) — this is a mature, tested path, not a gap.

So "return conversation to the user" is **not** about unlocking input — it already is. The real problem is the **status narrative**: for the full duration of a tool call (including a deliberate multi-minute `sleep`), the pane pins an undifferentiated "Working…"/tool-name banner that reads as "the agent is actively doing something and I should wait," discouraging the user from treating the pane as available — even though nothing is stopping them. The fix is representational, not a lock to remove.

## 4. The exact failure mode this generalizes from

`docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md`, same day, is the concrete precedent:

- Agent1 launched its own dev stack via the Bash tool with `run_in_background: true`.
- The launched command included a **self-inflicted `sleep`-based heartbeat**: `while true; do sleep 120; echo "[heartbeat] dev still alive $(date +%H:%M:%S)"; done &` — added, per the retro's inference, to defeat AgentMux's own idle-kill of background processes.
- That heartbeat kept refreshing the pane's `lastEventMs` at a 120s cadence — under the 180s `LIVENESS_RECOVERY_MS` watchdog threshold — so the pane never force-recovered to `Idle`. It sat in an ambiguous "Working…/Waiting…" state for **~12 hours**, genuinely healthy the entire time.
- Root cause identified there: **turn-phase and attached-process-liveness are orthogonal axes, and the reducer only models one.** A `sleep`-driven keepalive loop is not "the model generating a response" — it's a live but idle-turn-adjacent background process, and today's status model has nowhere to put that.

This report's "detect sleep" ask is the sharper, more specific version of that retro's fix direction — Agent1's case was a *backgrounded* task; the user's ask also covers a **foreground, blocking** `sleep N` call sitting directly in the turn's critical path (no `run_in_background` needed to trigger the same bad UX: 30 seconds of "Working…" for a command that is, definitionally, doing nothing).

## 5. Live bug found and fixed en route: dock subagent flood (PR #2201)

While investigating the dock's current state, the user separately reported (mid-session): *"dozens of subagents appear in the dock now, but the agent only made 1 or 2 tool calls...the docked item needs to be per Agent tool call, not per subagent."*

Root cause: `subagent-adapter.ts` mapped every `ActiveSubagent` the backend watcher knows about to its own `PinnedActivity`, with zero grouping. A single Workflow-tool call can spawn dozens of subagents at once — the Swarm pane's own docs cite **45 observed live in one run** (`REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md` Finding 4) — and the Swarm tree already solved this exact problem with `groupSubagentsByWorkflow` (shared `workflow_id` → one `WorkflowGroup`; same-name loose subagents → one `NameGroup`, `frontend/app/view/swarm/swarm-model.ts:151`). The dock adapter simply never adopted that grouping.

**Fixed** (this session, PR #2201): `subagent-adapter.ts` now runs its block-filtered subagent list through the Swarm pane's own `groupSubagentsByWorkflow` before mapping to `PinnedActivity`, reusing `groupCacheKey` for row identity so the dock and the Swarm tree agree on what counts as "one call." One dock row per Agent/Workflow-tool invocation, not per subagent. 11 tests (4 new) passing; typecheck clean.

**Known simplification, left as a follow-up:** a grouped row's expanded view and tail text show only the most-recently-active member's transcript, not every member's — `ActivityRow.tsx` renders one `subagent` per row today; teaching it a real multi-member expanded view (e.g. a nested member list) is separate work, not required to fix the row-count explosion.

This is directly relevant to the rest of this report: **any new dock-entry kind this report proposes (tool calls, sleep, backgrounded Bash) must go through the same "what counts as one call" discipline** — a naive one-row-per-raw-event mapping is exactly the bug class just fixed.

## 6. Detecting "a sleep is used" — and generalizing to "every agent call"

Two independent, complementary signals exist in the data already flowing to the frontend, neither currently used for anything:

### 6a. Foreground blocking `sleep` (or `sleep`-dominated) commands — text heuristic, no backend changes needed

The Bash tool's `command` string is **already present** as `event.params.command` in the `tool_call` NDJSON event (`useAgentStream.ts:47-48`, `extractToolArg`'s `bash`/`Bash` case) and already flows into `ToolStart.arg` → `state.currentToolArg`. Detecting "this call is fundamentally a wait" is a pure frontend string match against that value — no schema change, no wrapper change, no backend round-trip. A reasonable first-pass heuristic (needs validation against real transcripts before shipping, not invented here as final):

- Bare `sleep <N>` (optionally trailing `&& <cmd>` / `; <cmd>` / `|| <cmd>`) — the command's *primary* action is the sleep, whatever follows is incidental.
- Extend later to `timeout <N> <cmd-that-is-really-a-wait>`, `wait`, and shell keepalive-loop shapes (`while true; do sleep <N>; ...; done`) if real usage shows they matter — Agent1's own heartbeat is exactly this shape, so it's worth prioritizing.

This is the narrowest, cheapest, most directly-responsive-to-the-ask fix: it requires zero backend changes and could ship as a frontend-only heuristic layered on data that already exists.

### 6b. `run_in_background` — the general, already-identified, architecturally correct signal

Claude's actual Bash tool schema carries a `run_in_background: boolean` parameter (confirmed live in Agent1's own transcript: `run_in_background: true` on the dev-stack launch). **Nothing in AgentMux threads this through anywhere:**

- `BashParams` (`frontend/app/view/agent/types.ts:121`) has only `{ command: string; timeout?: number }` — no `run_in_background` field. If the raw NDJSON event carries it, it survives only as an untyped, unused property on the params bag.
- `ShellNode` — the dock's only pre-existing "attached long-running thing" representation — has **no background flag** (confirmed: `grep run_in_background` across `frontend/app`, `agentmux-srv`, `agentmux-bashwrap` returns zero hits anywhere except the retro's own prose).
- The consolidation report (Agent3, same day) already flags this precisely: *"No per-agent tracking of `run_in_background` Bash tasks anywhere… a backgrounded dev stack surfaces (if at all) as an undistinguished shell."*

Unlike text-sniffing for "sleep," this signal is **self-declared by the agent's own tool call** and covers every backgrounded task regardless of what it runs — the general form of "every agent call" the user asked for. It requires: (1) typing the field through `BashParams`/the tool-call event, (2) a `background: boolean` (or similar) flag on whatever node represents the resulting task, (3) wiring that into the dock adapter.

### Recommendation

Ship 6a first (cheap, immediately addresses the literal "detect sleep" ask, zero backend risk), land 6b as the architecturally-correct generalization (addresses "every agent call," properly distinguishes Agent1-class incidents). They are not mutually exclusive — a `run_in_background: true` call whose command is itself `sleep`-shaped should be dock-tracked either way; 6a is strictly the narrower, currently-uncovered subset (foreground, blocking) that 6b's flag doesn't reach because a foreground sleep is never backgrounded in the first place.

## 7. UX design: dock entry + "return conversation to the user"

Given §3 (composer already unlocked) and §4/§6 (what to detect), the actual product change is:

1. **New dock-entry representation for a detected blocking/background tool call.** Reuse the `PinnedActivity` abstraction (§1) — it already generalizes across kinds by design (`activity/types.ts`'s own doc comment: *"Anything long-running an agent spawns … maps onto this contract"*). Candidate: extend `ActivityKind` with a `tool` (or `background-task`) variant, sourced from a new adapter reading `ToolStart`/`ToolEnd` (or a dedicated background-task stream once 6b lands) instead of `ShellNode`/`ActiveSubagent`. Title = tool name + abbreviated arg (reuse `AgentFooter.tsx`'s existing `abbreviateArg`); status = running while the tool is in flight, done/error on `ToolEnd`/tool_result.
2. **Stop pinning the turn's headline status to a raw "Working…" for the tracked call's duration.** Once an activity is dock-tracked, `AgentWorkingRow`'s left-zone text should not need to keep repeating the same tool name/arg the dock row already shows — the working row can fall back to a calmer default (or be suppressed) while the dock carries the specific detail, similar to how "Rate limited — retrying…" already got its own distinct sub-state instead of overloading "Working…" (§4's retro cites this as the established precedent pattern).
3. **Exempt a pane with a live dock-tracked long-running/background activity from the liveness watchdog's "quiet = hung" recovery** (`LIVENESS_RECOVERY_MS`, `STUCK_THRESHOLD_MS` in `agent-pane-state/types.ts`) — this is the Agent1 retro's fix-direction §3 verbatim, and is required or the watchdog will eventually mislabel a legitimately-waiting pane as hung (or, worse, never had a chance to because the sleep's own heartbeat kept defeating it — the two failure modes are two sides of the same missing-axis problem).
4. **Remove from the dock on completion** — already the dock's existing, correct behavior via `RETENTION_MS`/`ActivityDock.tsx`'s `visible` memo (§1): a `done` row lingers 8s then auto-drops, no new mechanism needed. The ask's "when they are complete, remove them from the dock" is already how every existing dock kind behaves; a new tool-call kind inherits it for free by conforming to `PinnedActivity`.

## 8. Phased implementation sketch (not started)

1. **6a — foreground sleep heuristic + minimal dock kind.** Frontend-only: detect the pattern in `ToolStart`'s Bash arg, promote to a `PinnedActivity` (new `activity/tool-adapter.ts`, mirroring `shell-adapter.ts`'s shape), remove on `ToolEnd`. Smallest, most directly responsive slice.
2. **6b — `run_in_background` threading.** `BashParams` gains the field; the wrapper/stream event carries it through; a `background: boolean` flag lands on the resulting task representation (whatever node the Bash tool's background-task handle becomes — needs its own investigation into how `run_in_background`'s task-id handle is currently surfaced, if at all, since today's `ShellNode` is Shell-MCP-tool-specific and a backgrounded Bash task is a different code path entirely).
3. **Two-axis pane status + watchdog exemption** — per §7.2/§7.3, and per the consolidation report's own phase 3 ("surface `Running: <task> (elapsed)` in `AgentFooter` when ≥1 live background task exists while the turn is Idle; exempt such panes from the liveness watchdog").
4. **Generalize beyond Bash** — a duration-threshold promotion (any tool call still running past N seconds gets a dock row automatically, tool-agnostic) is the cleanest reading of "similar with every agent call" beyond sleep/background specifically; sequence after 1–3 prove the pattern out on the highest-value case.

## 9. Open questions

- **Where does a `run_in_background: true` Bash task's live output/handle currently surface, if anywhere?** Not confirmed in this pass — needs a repro (launch one, inspect the NDJSON stream / block meta) before 6b's design can be finalized. If nothing currently tracks it server-side either, 6b is a bigger lift than the frontend-only framing above assumes.
- **Sleep heuristic false-positive risk.** A command like `sleep 2 && rm -rf /tmp/staging` is mostly "do the rm," not "wait" — the regex needs real-transcript validation, not just Agent1's one heartbeat example, before it ships.
- **Should the dock's new tool-call kind coexist with the inline `AgentWorkingRow` text, or replace it while tracked?** §7.2 leans toward "calmer status while dock carries detail," but the exact wording/precedence needs a design pass, not a code-first decision.

## 10. Key files

| File | Role |
|------|------|
| `frontend/app/view/agent/components/ActivityDock.tsx` | The dock itself — rendering, ordering, retention |
| `frontend/app/view/agent/components/ActivityRow.tsx` | Per-row chrome; kind-dispatches for expanded view |
| `frontend/app/view/agent/activity/types.ts` | `PinnedActivity`/`ActivityKind` — the abstraction any new kind extends |
| `frontend/app/view/agent/activity/shell-adapter.ts` / `subagent-adapter.ts` | Existing adapters — the pattern a new tool-call adapter mirrors |
| `frontend/app/store/agent-pane-state/{reducer,types}.ts` | Turn-phase state machine; `ToolStart`/`ToolEnd`, `currentTool`/`currentToolArg`, the liveness watchdog thresholds |
| `frontend/app/view/agent/useAgentStream.ts` | `extractToolArg` (already surfaces the raw Bash command), `ToolStart` dispatch site |
| `frontend/app/view/agent/components/AgentFooter.tsx` | `AgentWorkingRow` — today's only "what tool is running" UI, and where a calmer status / new sub-state would land |
| `frontend/app/view/agent/types.ts` | `BashParams` (no `run_in_background` field — the 6b gap), `ShellNode` (no background flag — same gap, per the consolidation report) |
| `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` | The Agent1 incident — concrete precedent, fix direction this report builds on |
| `docs/specs/REPORT_LONGRUNNING_SUBAGENT_SWARM_CONSOLIDATION_2026_07_16.md` | Sibling report, same day — broader tracker this deep-dive feeds into |
