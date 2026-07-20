# REPORT — why every subagent under a long-running pane shows "Interrupted"

**Date:** 2026-07-20
**Trigger:** Live observation after landing #2231/#2232/#2233 tonight — opened
the agent pane "Lzop" (a long-running session with many prior Agent-tool
calls) in the two-bucket Swarm pane and every single row displayed
"Interrupted," including ones the user could confirm had actually finished
their work cleanly. User asked: what are the possible states, do subagents
ever show as genuinely done, and can a row be manually dismissed once it is.

**Scope:** Diagnosis only — no code changes. Companion implementation plan:
`SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20.md`.

---

## 1. The three backend states

`SubAgentStatus` (`agentmux-srv/src/backend/subagent_watcher.rs:100-112`):

| Variant | Set by | When |
|---|---|---|
| `Active` | `process_jsonl_change` | Default, the instant a subagent's JSONL file is first observed |
| `Completed` | `process_jsonl_change` (subagent_watcher.rs:1363-1374) | The last event read for that subagent is a `"type":"result"` line — checked live, on every filesystem-watcher tick and every backfill scan, **no reopen required** |
| `Abandoned` | `reconcile_stale_subagents` (subagent_watcher.rs:1062-1123) ONLY | The parent block's turn is confirmed not-active (`turn_active: false` via `get_block_controller_status`) and the subagent is still `Active` |

**So yes — subagents do complete, and completion detection itself works
correctly and live.** A subagent that finishes cleanly gets `Completed` the
moment its `result` line is read, with no dependency on reopening anything.

## 2. What the UI actually shows

`subagentDisplayStatus` (`frontend/app/view/swarm/swarm-view.tsx:209-215`):

```ts
export function subagentDisplayStatus(sub: ActiveSubagent, parentAgentStatus: "running" | "idle"): AgentDisplayStatus {
    if (sub.status === "abandoned") return "interrupted";
    if (sub.status === "active") {
        return parentAgentStatus === "idle" ? "interrupted" : "working";
    }
    return "idle"; // completed
}
```

Three outputs reachable for a subagent row: `"working"`, `"idle"`,
`"interrupted"`. Critically: **`Completed` always renders `"idle"`,
unconditionally** — a genuinely-finished subagent can never display
"Interrupted." The only path to "Interrupted" is a subagent still sitting as
backend `Active` (`Abandoned` also maps there, but that variant is itself
only ever produced by the same reconciliation gap described below).

## 3. Root cause: reconciliation only runs at pane reopen

`reconcile_stale_subagents` has exactly **one** production call site —
`scan_session_subagents` (subagent_watcher.rs:1034), itself called from
exactly one place: `handle_reactive_register` (`server/reactive.rs:350`),
the reactive-registration handshake that fires when a pane (re)opens or
reconnects. **Nothing else calls it** — no timer, no per-turn hook, no
live trigger of any kind. Confirmed by exhaustive grep of both files.

This means: any subagent whose JSONL never got a `Result` line read while
its `SubagentWatcher` in-memory record was alive stays `Active` forever,
until the next time its pane is closed and reopened. On a **long-running
pane like Lzop's** — open continuously, never reopened — every subagent
that got orphaned this way (crashed, was actually cut off, or simply hasn't
had its completion re-checked) sits `Active` in memory indefinitely.

The frontend has a documented client-side backstop for exactly this gap
(`swarm-view.tsx:196-208`, doc comment cites
`docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md` Open
Question 1 by name): whenever `sub.status === "active"` and the *parent*
agent's own turn is currently idle (`parentAgentStatus === "idle"`), display
"Interrupted" rather than a misleading "working." This is correct given the
information available — a subagent literally cannot still be running once
its parent's turn has ended, since a Task-tool call is synchronous within
the parent's own turn. But a parent pane spends the **overwhelming majority
of its lifetime idle-between-turns** (that's the normal, expected steady
state, not a sign of anything wrong) — so this backstop fires essentially
every time a user looks at a long-running pane's Swarm rows, for any
subagent the backend hasn't gotten around to reconciling.

**Net effect on Lzop:** a long-open pane accumulates `Active`-forever
subagent records (some genuinely abandoned, some possibly still correctly
`Active` momentarily, some that may have actually completed but whose
completion detection lagged/missed for some reason not yet isolated), and
every one of them displays "Interrupted" continuously because Lzop itself
is idle between turns essentially all the time the user is looking at it.

## 4. This is a previously-identified, explicitly-deferred gap

`docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md` designed
and shipped most of this system eight days ago — the `Abandoned` variant,
`reconcile_stale_subagents`, and the frontend backstop described above are
all exactly what that spec proposed (§6.1-6.3, §6.5). Its **Open Question
1** explicitly named the tradeoff being hit right now:

> "should `ParentTurnEnded` fire from the exact moment `persistent.rs` flips
> `turn_active` to false (real-time...), or is it sufficient to only
> reconcile at `scan_session_subagents` time (reopen/backfill only, simpler,
> but **leaves a subagent stuck "Active" for the rest of the CURRENT
> session between the parent turn ending and the next pane reopen**)?
> Recommend starting with the reopen-time-only version... and evaluating
> whether the live case is common enough in practice to warrant the
> real-time wiring as a fast-follow."

The reopen-only version shipped; the real-time fast-follow never did. Lzop
tonight is the "is the live case common enough in practice" question
answered empirically: yes — any pane left open across multiple subagent
spawns without being reopened will show this.

## 5. Retire/dismiss: confirmed net-new gap

Grepped the whole Swarm pane frontend (`swarm-model.ts`, `swarm-view.tsx`)
for "retire"/"dismiss"/"archive" — the only hits are the *Workflow dispatch
grouping* label ("Active"/"Retired" chip on a `WorkflowDispatchRow`, driven
purely by `DispatchStatus`), not a per-row user action. There is no
mechanism anywhere to manually remove a subagent or dispatch row from the
Swarm pane view. The prior spec's own Open Question 2 ("is `Abandoned`
worth a visible status, or should it fall into idle" — SPEC_SUBAGENT_
LIFECYCLE_RECONCILIATION_2026_07_12.md §9.2) explicitly considered and
rejected silently hiding it, but never proposed an explicit user-driven
dismiss action either way. This is a genuinely new ask, not something
previously scoped and dropped.

## 6. Summary for the implementation plan

Two independent, separable pieces of work close this out — detailed in
`SPEC_SUBAGENT_LIVE_RECONCILIATION_AND_RETIRE_2026_07_20.md`:

1. Close Open Question 1 for real: wire `ParentTurnEnded` reconciliation at
   the exact moment a turn ends (`persistent.rs:921-922`, the
   `set_active_turn(false)` call site — already identified and cited by
   name in the 07-12 spec as the future hook point), not just at reopen.
2. Add an explicit Retire/dismiss action for terminal-status
   (`Completed`/`Abandoned`) rows — net-new UI surface, no prior design to
   build on.
