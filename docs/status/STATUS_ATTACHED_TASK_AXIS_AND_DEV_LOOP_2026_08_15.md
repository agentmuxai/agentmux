# Status: Long-Running Attached Tasks — Rungs 1–2 Verified Shipped, the Architecture Rethink for Rungs 3–5

**Date:** 2026-08-15
**Author:** AgentA
**Status:** Report — code audit against `main` @ `44b3c6a17` (today), consolidating what's shipped since the 08-09 ladder doc, confirming #2491/#2492 are exactly the next unstarted rungs, and proposing a deeper architectural fix rather than two point patches.
**Supersedes nothing** — updates `docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_09.md` (the "ladder" doc this one continues numbering from) and folds in the two follow-on retros written after it shipped:
`docs/retro/retro-stuck-background-dock-timer-2026-08-10.md`, `docs/retro/RETRO_TASK_DEV_IDLE_KILL_FALSE_POSITIVE_2026_07_31.md`.

---

## 1. What's actually shipped since the 08-09 ladder doc (verified against today's code, not assumed)

| Rung | What | PR | Verified today |
|---|---|---|---|
| 1 | Wire the `attachedTask` axis end-to-end | #2489 (merged 08-10) | Reducer slice (`types.ts:308-310,638-643`, `reducer.ts:1241-1252`) is live — `AttachedTaskObserved`/`AttachedTaskCleared` dispatched from `agent-view.tsx:1436-1458` via `hasLiveAttachedActivity()`. **Rendering half was reverted the same day** on user feedback ("redundant with the dock's own running row") — `AgentFooter.tsx:272-276` explicitly documents this: the reducer axis stays live (feeds the watchdog + is available for Swarm), but nothing currently renders a footer "Running in background" string. This is intentional, not a regression — worth knowing before anyone "fixes" it back. |
| 2 | Thread `run_in_background` into `BashParams` + dock adapter for backgrounded harness tasks | #2502 (merged 08-10) | `BashParams.run_in_background` present; `tool-adapter.ts`'s `isAcceptedBackgroundLaunch` gives an accepted background launch an immediate dock row (no 30s wait), held until the `<task-notification>` lands. |
| 2a | Fix: fast-finishing `run_in_background` calls were misclassified as "running forever" | #2519 (merged 08-10) | `isAcceptedBackgroundLaunch` now requires the literal `"Command running in background with ID:"` prefix, not just the flag + terminal status. Closed a real production bug (17 stuck dock rows). |
| 2b | `muxspect dock`'s `bg` column — server-side visibility into genuinely-accepted launches | #2520 (merged 08-10) | Shipped, but **short-lived**: `DockSnapshotCache` evicts any snapshot past `MAX_NODE_AGE_MS` (1 hour) with no refresh push for a still-running task (see §3.1). |
| 3 | Teach bashwrap the declared-long-running/idle-timeout difference | **#2491 — not started** | Confirmed: zero references to `run_in_background`/`RunInBackground` anywhere in `agentmux-bashwrap/` (§2.1). |
| 4 | Survive session teardown | **#2492 — not started** | Confirmed: no reparent/adopt/teardown-survival code exists (`git log --grep="teardown\|reparent\|adopt"` since 08-10, one unrelated hit). |
| 5 | Windows `started_at_ms` via `NtQueryInformationProcess` | Not started, not urgent | Unchanged, independent of the above. |
| — | Feed the signal to Swarm (07-26 report's step 4) | **Not started, not tracked** | `grep -n "attachedTask\|run_in_background" frontend/app/view/swarm/*.ts*` → zero hits. Confirmed still open, not on the ladder doc's own rung list either — flagging it here so it doesn't get lost a third time. |

**Bottom line: the *detection and display* half of this problem (rungs 1–2) is genuinely done and hardened by two rounds of follow-on bugfixes.** The *operational* half — actually keeping the process alive past bashwrap's idle-timeout and past a session restart — is completely unstarted. #2491 and #2492 are correctly scoped; picking them up next is the right call.

---

## 2. New findings from this pass (beyond what the ladder doc already knew)

### 2.1 Rung 3 is more tractable than it looks — the data already exists at the hook boundary

`agentmux-bashwrap/src/hook.rs::PreToolUseInput` already deserializes the **full raw tool params** (`tool_input: Value`, `#[serde(default)]`) — Claude Code's PreToolUse hook payload includes `run_in_background` whenever the model sets it, exactly like `command`. The hook currently reads only `tool_input.command` (`hook.rs:70-73`) and discards the rest. The wrapped invocation it emits is just:

```
agentmux-bashwrap exec --tool-id=<id> --b64-cmd=<b64>
```

No flag for "the caller declared this backgrounded." `bash_wrap.rs`'s idle-timeout (`idle_kill_timeout()`, read from `AGENTMUX_BASHWRAP_IDLE_TIMEOUT_SECS`, applied at `bash_wrap.rs:1165,1369,1478`) is a single global value with **no per-invocation override at all** today.

This means rung 3 is not a design problem, it's a plumbing problem: read `run_in_background` alongside `command` in `hook.rs`, thread it into the wrapped command line as `--declared-background` (or similar), and have `bash_wrap.rs` apply a relaxed/disabled idle-timeout when that flag is present. No new detection mechanism needed — the signal already arrives at the one place that needs it, it's just dropped on the floor today.

### 2.2 The 1-hour dock-cache eviction undermines rung 4 before it even ships

`docs/retro/retro-stuck-background-dock-timer-2026-08-10.md` already flagged this as "not yet tracked as an issue": `DockSnapshotCache` evicts any node — including a genuinely-still-running `bg: true` one — after `MAX_NODE_AGE_MS` (1 hour), with no periodic refresh push to keep a long-lived task's entry alive. `task dev` sessions in this repo's own retros have run 12+ hours (the Agent1 incident) and 17+ minutes minimum before even reaching bashwrap's kill. **Even after rung 4 ships (the OS process survives session teardown), the dock/attachedTask signal chain that's supposed to show the user "this is still running" goes dark after an hour and stays dark until the process actually completes or the pane reloads and re-derives from a live source.** This is the same failure shape as the original #2518 bug (UI silently misrepresenting real state) one layer further out. Any rung-4 design needs to also address this, or it just moves the "stuck 17 dock timers" incident from "shows running forever" to "shows nothing at all" — arguably worse, since at least the old bug was discoverable.

### 2.3 The deeper pattern: every signal in this system is client-derived and ephemeral

Looking at rungs 1–5 together, a structural property falls out that none of the five individual retros/reports named explicitly, because each was scoped to one symptom:

**There is no durable, server-owned record of "a long-running task is attached to this agent."** Today's signal chain is:

```
CLI's own transcript stream (ephemeral, this session only)
  → frontend re-parses it live (tool-adapter.ts, attached-task.ts)
    → reducer slice (in-memory, this pane's tab only)
      → optionally mirrored to srv's DockSnapshotCache (1hr TTL, no refresh)
```

Every layer is either scoped to the current session, scoped to the current pane's open tab, or time-bounded. `AgentProcessRegistry` (`agentmux-srv/src/backend/process_tracker/registry.rs`) is the one genuinely server-side, OS-level piece — but it only does Job-Object *membership* polling (is this PID still a child of the agent's job), with no concept of duration, purpose, or "this one is a declared long-runner the user should be told about" (confirmed: `started_at_ms` hardcoded to 0 on Windows, §3.3 of the 08-09 doc, still true).

This is why rungs 3 and 4 both feel harder than they should: **bashwrap needs to know "is this declared long-running" at spawn time, and the teardown-survival logic needs to know "which processes should be adopted rather than killed" at session-end time — and neither of those moments has access to the ephemeral, frontend-only signal that currently answers that question.** Patching rung 3 and rung 4 independently (as two point fixes) will each need to invent their own answer to "how do I know this task is a declared long-runner" from wherever they happen to sit in the stack, duplicating logic the frontend's `hasLiveAttachedActivity`/`isAcceptedBackgroundLaunch` already implements, and drifting from it the same way the `bg` column duplicated-then-diverged from `isAcceptedBackgroundLaunch` in the #2518 incident (documented in that retro's review-round-2 finding).

---

## 3. Recommendation: a first-class, server-owned Background Task Registry

Rather than landing #2491 and #2492 as two independent point fixes (each re-deriving "is this a long-runner" its own way), promote the concept itself to a durable, server-side entity that every consumer reads from the same place:

```rust
struct BackgroundTask {
    id: TaskId,                    // stable across reconnect/reload
    block_id: BlockId,              // owning agent pane
    label: String,                  // command / dev-agent.cmd TITLE=...
    pid: Option<u32>,                // OS process, once known
    started_at_ms: u64,
    declared: bool,                  // true if run_in_background / MCP Shell tool; false if duration-promoted
    status: Running | Completed | Failed | Adopted | Orphaned,
    last_seen_ms: u64,               // heartbeat, not a hard TTL eviction
}
```

**Where it plugs into the existing pieces, one registry feeding all five consumers instead of five ad hoc derivations:**

1. **Creation** — a task enters the registry the moment it's recognized as long-running, from *either* signal already detected today: `run_in_background: true` accepted by the harness (rung 2's existing detection), or the existing 20–30s duration-promotion heuristic (rung 1) for anything that forgot to declare itself. This closes the "amux should know immediately whether to delegate a long-runner to background" ask directly — today, a plain foreground Bash call that turns out to be a dev server only gets *dock-promoted* after 30s; it still isn't in any durable registry, still isn't exempt from bashwrap's idle-timeout, and still dies at teardown. Feeding both detection paths into one registry entry means duration-promoted tasks get the same durability guarantees as explicitly-declared ones, not a second-class subset.
2. **Bashwrap (rung 3)** — at spawn, `hook.rs` checks/creates the registry entry (or, simpler for v1: just threads the boolean through as in §2.1 — the registry doesn't have to exist before rung 3 ships, see phasing below) and passes a flag down; `bash_wrap.rs` reads it and skips/relaxes the idle-timeout for that invocation specifically, not globally.
3. **Session teardown (rung 4)** — the launcher/srv, not the per-session agent process, owns entries in this registry. On agent-session restart, anything still `Running` in the registry gets reparented (Job-Object adoption, consistent with isolation invariants I1–I6 in this repo's `CLAUDE.md`) instead of dying with the old session. This is the piece that has no existing analog to build on — needs its own design pass, flagged as the highest-uncertainty rung.
4. **Dock / attachedTask axis / footer** — become *readers* of the registry (via a subscription, same WPS-event pattern `AgentProcessRegistry` already uses for `agent:process-added`/`-exited`) instead of re-deriving state by replaying the transcript. Removes the 1-hour TTL problem (§2.2) entirely — the registry is the source of truth, not a cache of a stream that might have been evicted.
5. **Swarm pane** — same subscription, trivially closes the still-open "feed the signal to Swarm" gap (§1's last row) as a consequence of everything reading one source, not a sixth bespoke wire-up.
6. **Reconnect/history-restore** — the frontend queries the registry directly on mount instead of replaying/scrubbing transcript state (`scrubOrphanedInProgress`'s session-boundary-only cleanup, `docs/retro/RETRO_TASK_DEV_IDLE_KILL_FALSE_POSITIVE_2026_07_31.md`'s stuck-`sleep 45`-entry finding) — a task's real status is always one query away, not dependent on which transcript events happened to survive.

### Why this over two point fixes

The point-fix path (ship #2491 and #2492 exactly as scoped, independently) is faster short-term and not wrong — it would genuinely fix the two operational symptoms. But it leaves the structural gap from §2.3 in place: the *next* consumer that needs "is this task a declared long-runner" (Swarm, a future notification system, a future `muxspect` command) will face the same choice #2520 faced — duplicate the detection logic and risk drifting from it, exactly as happened once already in this same feature area. A shared registry is the version of "designed deep into the state machine" that actually generalizes, per the ask.

### Phasing (keeps each step independently shippable, per this repo's own norm)

1. **#2491 as scoped** (bashwrap idle-timeout exemption) — ship it now, via the direct plumbing in §2.1. Don't block it on the registry; it's a real, narrow, low-risk fix on its own and unblocks the immediate `task dev` pain today.
2. **Stand up the registry as a thin layer first** — even before teardown-survival exists, having ONE server-side table that the dock/attachedTask/Swarm all read from (instead of three independent derivations) is valuable on its own and removes §2.2's TTL problem immediately. `AgentProcessRegistry` is the closest existing analog to extend rather than build from scratch.
3. **#2492 (teardown survival) lands on top of the registry**, once it exists — the registry is exactly the "which processes to adopt" list rung 4 needs and doesn't have today.
4. **Swarm surfacing + rung 5 (`started_at_ms`)** — cheap, additive, once the registry is the shared source.

---

## 4. Live smoke-testing — proposed, not yet executed

This report is grounded in a full code audit (git history, current `main`, all five related retros/specs/reports) but **not yet in a live dev-build repro**, per this repo's own dev-server rules (`CLAUDE.md`: warn before triggering a full rebuild; the last three retros in this exact feature area were each triggered by a real `task dev` session, so a live pass is the right next step, not optional polish).

Proposed smoke-test plan, once ready to execute:

1. **Baseline repro of the still-open bug**: launch `task dev` via the documented heartbeat-wrapped pattern (`scripts/dev-agent.cmd` + 120s heartbeat, per `CLAUDE.md`/the 07-31 retro), confirm bashwrap still kills it after ~10 min with no exemption (expected — rung 3 unshipped) — this reconfirms §1's audit against live behavior rather than static code reading alone.
2. **Confirm rungs 1–2 actually work live**: a real `task dev` launched with plain `run_in_background: true` (no heartbeat) should now show a dock row immediately and the turn should return to idle without the 12-hour "Working…" symptom — verifying the theoretical analysis in §1 against a real pane, since neither #2489 nor #2502's own PR bodies record a completed live check (#2489: "Live verification still needed"; #2502: "worth confirming in a running build").
3. **After #2491 lands**: repeat step 1, confirm the process survives past the old ~10min kill point.
4. **After #2492 lands**: kill/restart the agent session mid-`task dev`, confirm the dev instance survives and reattaches.
5. **Reconnect test for §2.2**: leave a background task running >1 hour, reload the pane, confirm current behavior (dock/attachedTask goes dark) vs. post-registry behavior (still shows running).

This needs an actual `task dev` (or `task package`) build cycle — flagging the cost before spending it, per this repo's Rust-build-is-slow norm, rather than kicking one off silently. Want me to proceed with step 1–2 now to ground rung 3's implementation in a fresh live repro, or move straight to implementing #2491 per §2.1's already-concrete plan and verify live once the code exists?
