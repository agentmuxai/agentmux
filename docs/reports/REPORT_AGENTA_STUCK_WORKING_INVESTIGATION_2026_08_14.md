# Report: AgentA stuck showing "Working…" — root cause, rigorously confirmed

**Date:** 2026-08-14
**Author:** Manoz
**Verified against:** `main` @ `8c30af9a` (pulled fresh this session).
**Status:** Root cause confirmed via direct log/source/git-history evidence. One residual detail (exact reason the watchdog timer stopped firing) narrowed to a single strong hypothesis, not yet confirmed live — flagged explicitly in §6. No code changes made; no remediation applied.
**Triggered by:** user report — "AgentA in your same instance is stuck in 'Working...'"; user later directed "keep going, don't stop to ask, get to root cause, and write rigorous report to file."

## 0. Tooling / methodology notes (read this before trusting any `muxlog` output)

- `muxspect`/`muxlog` are shell tools shipped inside AgentMux (`~/.agentmux/shell/{muxspect,muxlog}.mjs`), not MCP tools — reachable via an agent tool-call shell's `$AGENTMUX_LOCAL_URL`/`$AGENTMUX_AUTH_KEY`.
- **`muxlog`'s default "most-recently-active instance" picker is unreliable and was caught returning data from a completely unrelated AgentMux instance** (different block IDs, inconsistent timestamps) with no visible error — it silently succeeded against the wrong target. Do not trust `muxlog`'s default target selection for anything precise; either pass `-i <substr>` with a verified instance identifier, or (what actually worked here) locate the exact log file on disk yourself and grep it directly.
- **How I found the right instance:** my own AgentMux channel is encoded in my own session's working-directory path — `~/.agentmux/channels/<my-channel>/`. Cross-referencing that against `~/.agentmux/channels/*/versions/*/logs/agentmux-host-*.log.*` on disk found the exact log file for the instance I'm actually running inside. This is the reliable method; `muxlog`'s picker is not.
- Plain `muxlog fe`/`muxlog host` (no subcommand) defaults to **follow/tail mode and hangs** — always pass `cat` or `grep <re>` explicitly.

## 1. Identifying "AgentA"

Exactly two agent panes exist in this instance (`muxspect list`):
- `47991e56-0728-49a9-acb4-ab391f3bd720`
- `60cd667d-1607-4d4c-ae46-04d80d59b929` — confirmed to be **my own pane** (`Manoz`): its `muxspect dock` tool-node timings line up exactly with my own tool calls throughout this investigation.

`47991e56` is therefore AgentA by elimination (still not directly confirmed via a name→block mapping — none was found in logs or `/api/v1/blocks`/`/api/v1/agents`, both empty). All findings below are about `47991e56`.

## 2. Prior art (not re-derived)

- `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` — Agent1, ~12h stuck Working, root cause a genuine `run_in_background` process (different mechanism from this incident).
- `docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` — catalogs 9 false-positive "Working" paths, prescribes telemetry. **Confirmed on current `main`: the frontend `[wave-turn]` transition/watchdog-reasoning telemetry (§3.1-3.2, `agent-pane-state-store.ts`) and backend `[health] turn_active flip` telemetry (§3.3, `health.rs`) are already shipped** — both cite the July 27 report directly in their own source comments. This shipped telemetry is what made the rest of this investigation possible.
- **This incident is a live, confirmed instance of that report's risk #7**: *"`StreamFlushObserved`'s broad re-promotion rule ... intentionally promotes `Idle`/`Disconnected`/`Done.completed` back into `Streaming` on any flush ... so a late/stray flush after what looked like completion can silently re-arm 'Working.'"* — see §3 below for the exact confirmed occurrence.

## 3. The confirmed trigger — a complete, unbroken transition timeline

Reconstructed from `[wave-turn]` transition logs (`frontend/app/store/agent-pane-state-store.ts`'s `console.info`, which fires on every `turnPhase.kind` change) in the correct instance's own host log — every single transition for block `47991e56`, all day 2026-08-14, with **zero gaps**:

The pane cycled through many completely normal turns all day (00:10, 01:22, 05:38, 06:03, 07:43, 12:57 UTC), each cleanly reaching `Done`. The final normal cycle, immediately preceding the incident:

```
13:28:56.335982Z  Streaming → Done       cmd=TurnEnd
13:28:56.338329Z  Done → Streaming       cmd=ToolStart          toolsActive=1 currentTool=Bash
13:29:07.248523Z  Streaming → Done       cmd=TurnEnd
13:29:07.294850Z  Done → Streaming       cmd=TokensOut           toolsActive=0
13:29:07.322289Z  Streaming → Idle       cmd=ReconcileTurnActive
13:29:07.328741Z  Idle → Done            cmd=TurnEnd
13:29:32.191078Z  Done → Streaming       cmd=StreamFlushObserved toolsActive=0 currentTool=-
```

**`13:29:32.191078Z` is the last `[wave-turn]` line for this block anywhere in the log — for the rest of the day (confirmed through at least `13:58:48Z`, the end of the queried window, i.e. effectively "now" relative to this investigation).**

Reading this precisely: the turn genuinely, cleanly finished at `13:29:07.328Z` (`Idle → Done`). Then, 25 seconds later, a **stray/late stream-flush event** arrived and was processed by `StreamFlushObserved`'s re-promotion rule — which exists deliberately, to handle legitimate multi-round tool continuations where a `Done` pane needs to re-enter `Streaming` for a genuine follow-up round. In this case there was no genuine follow-up: nothing else ever arrived. The pane has sat in `Streaming` with `toolsActive: 0` ever since — **the exact "Working" visual state the user reported, confirmed independently by both the reducer's own transition log and the frontend's separate `[agentActivity]` busy-aggregator** (`frontend/app/store/agentActivity.ts`, which reads the same `workingFromPhase(turnPhase)` signal and has logged `47991e56` continuously present in the global busy-panes set from `13:29:32` onward, oscillating only around my own pane joining/leaving).

Cross-checked against the backend (`agentmuxsrv` log, `[health] turn_active flip`): the backend's own last activity for this block is a clean `active:false` at `13:29:07.307Z` — matching the frontend's own `Idle → Done` 21ms earlier. **The backend has done nothing else for this block since.** The stray flush that caused the stuck re-promotion left no corresponding backend signal at all (consistent with it being exactly what the July 27 report's risk #7 describes: a late/stray flush, not a real new turn).

## 4. Why this became *permanent* instead of self-healing in ≤180s

This class of stuck state is not supposed to be able to persist: `reducer.ts:331`'s `StreamWatchdogTick` handler unconditionally recovers **any** `Streaming` phase with `toolsActive === 0` back to `Idle` once `LIVENESS_RECOVERY_MS` (180s) of inactivity has elapsed — no exemption, no gating on `compacting` in this case (only rate-limit waits extend the threshold, and this pane wasn't rate-limited). This is dispatched by a 5-second `setInterval` armed inside `useTurnLifecycle` (`frontend/app/view/agent/hooks/useTurnLifecycle.ts:233-239`), itself invoked from `useAgentStream`'s `onMount` (`useAgentStream.ts:263`).

**If that interval were actually ticking for this pane, we would see it in the log — and we don't.** The frontend's own logging is edge-triggered but still decisive here: `stream-stuck` only logs once per stall episode (`agent-pane-state-store.ts`'s `slot.stuckLogged` gate), so a *ticking-but-quiet* watchdog would still have produced exactly one `watchdog: no recovery` line around `13:30:17Z` (45s after the stray flush) — and, since nothing ever refreshes `lastEventMs` again, an unconditional `watchdog: FIRED — force-recovered to Idle` line around `13:32:32Z` (180s bound; this branch has no exemption for `toolsActive: 0`). **Neither line exists anywhere in the rest of the day's log for this block.** The only explanation consistent with total silence for 29+ minutes (and counting, as of the last queried timestamp) is that `StreamWatchdogTick` was never dispatched for this pane during that entire window — the interval itself stopped ticking, or was never armed for this specific pane instance.

### Ruled out during this investigation

- **Reducer logic bug** — ruled out; `reducer.ts:331`'s bound is unconditional for `Streaming`+`toolsActive===0`, correctly implemented, unit-tested.
- **Pane component unmounted while backgrounded** (my initial hypothesis) — **ruled out directly**: `frontend/app/workspace/workspace.tsx:22`'s own comment: *"Inactive tabs are hidden via `display:none` — no unmount/remount."* `useAgentStream`'s `onMount` (and therefore the watchdog `setInterval`) fires once when the pane is first opened and is never torn down by a tab switch.
- **A JS crash/exception around the trigger moment** — checked `[uncaught-error]` lines in the host log for the surrounding window; found only benign `ResizeObserver loop completed with undelivered notifications` warnings (a well-known harmless Chromium quirk) at `13:28:03-07Z`, before and unrelated to the `13:29:32Z` trigger. No crash evidence at or after the trigger.

### Leading hypothesis (not yet confirmed live)

With component-unmount and reducer-logic both ruled out, the remaining plausible mechanism is **renderer-level timer throttling of a backgrounded window** — Chromium/CEF can throttle or suspend `setInterval` timers for a renderer that isn't the OS-focused window, independent of intra-window `display:none` tab-hiding (which `workspace.tsx` confirms does NOT itself stop timers). If AgentA's pane lives in a separate CEF `BrowserWindow` from whichever window currently has OS focus, that window's own timers — including this specific watchdog — could be suspended by the browser engine itself, silently, with no application-level log signal at all (since the throttling happens below the level anything in this codebase instruments). This would fully explain the total silence: not a bug in the reducer or the mount lifecycle, but the watchdog's own delivery mechanism (a plain `setInterval`) being an insufficient primitive for a background-window timer in a multi-window Chromium/CEF app.

**This is a strong, evidence-consistent hypothesis, not a confirmed fact.** Confirming it requires live inspection (Ctrl+Shift+D diag panel on the actual running instance, or checking CEF window-focus/throttling state directly) — not yet done in this investigation.

## 5. Complete root-cause chain (summary)

1. A stray/late `StreamFlushObserved` event arrived 25s after a legitimately-completed turn, and the reducer's own by-design re-promotion rule (needed for real multi-round continuations) treated it as a new round — re-arming `Streaming` with `toolsActive: 0`. **This part is a known, accepted trade-off, not itself a bug** (July 27 report's risk #7, already documented as an accepted risk pending its own watchdog coverage).
2. The system's own designed safety net for exactly this shape of stuck state (`StreamWatchdogTick` → 180s unconditional recovery) **should have** self-healed this within 3 minutes, with zero user-visible impact.
3. It didn't, because the periodic tick that drives that safety net was never dispatched for this pane during the stuck window — confirmed by the total absence of any of its logging (edge-triggered, but still guaranteed at least one line if ticking). Component-unmount and reducer-logic bugs are both ruled out as the cause; renderer/window-level timer throttling in the multi-window CEF architecture is the leading remaining explanation, unconfirmed pending live inspection.
4. **Net effect: what is designed to be an invisible, self-correcting 3-minute blip became a permanent, user-visible "stuck Working" state**, because the one thing standing between "harmless known edge case" and "permanently broken pane" — the watchdog's timer — silently stopped running for this specific pane.

This also means: **this is not unique to AgentA or to this specific incident.** Any pane that (a) receives a stray/late stream flush after a completed turn (risk #7, already known to be reachable — e.g. from network reordering, backend buffering, or a race during rapid turn cycling like the `13:28:01-13:29:07` cluster seen just before this incident, which itself involved a `RequestStop`/respawn/retry sequence) AND (b) is in a pane/window whose watchdog timer isn't currently ticking for whatever reason, will get stuck the same way, indefinitely, with no self-recovery — this is a structural gap, not a one-off.

## 6. What was NOT done / remaining open items

- **Not confirmed live**: the exact reason `StreamWatchdogTick` stopped ticking (§4's leading hypothesis — CEF background-window timer throttling — vs. any other unconsidered mechanism). Needs the Ctrl+Shift+D dev diagnostic panel or direct CEF process inspection on the live instance.
- **Not confirmed**: `47991e56` ↔ "AgentA" name mapping (inferred by elimination only).
- **No mutating action taken** — no `muxspect dock clear`, no message sent to AgentA, no code changes, no live pane reload. A live reload/refocus of AgentA's pane would very likely clear the immediate symptom (consistent with `ReconcileTurnActive`'s own focus-triggered-reconcile design, `reducer.ts:372-380`) but would not fix the underlying gap — it would recur on the next stray flush + timer-stall coincidence.
- Did not chase `60cd667d`'s own earlier `13:41:38-13:42:25` "stale --resume session id" / `assign_process failed` episode — confirmed unrelated (that's my own pane, a separate and already backend-resolved episode, well before and unconnected to AgentA's `13:29:32` incident).

## 7. Suggested remediation (not implemented — root-cause investigation only, per explicit scope)

Ranked by leverage:

1. **Give `StreamFlushObserved`'s re-promotion rule (risk #7) its own bounded safety net independent of `StreamWatchdogTick`'s `setInterval`** — e.g., a short-lived, freshly-scheduled timer/deadline created specifically at the moment of re-promotion (mirroring `SUBMIT_TIMEOUT_MS`'s "schedule on entry" pattern; note that mechanism has its own, separate, already-known gap — see below), rather than relying on an already-running long-lived interval that this incident shows can silently stop.
2. **Confirm and fix the timer-liveness gap directly** — if §4's CEF-background-window-throttling hypothesis is confirmed, the fix is architectural (e.g. drive the watchdog off a source that's guaranteed to tick regardless of window focus — a backend-side heartbeat via WPS the frontend merely listens to, rather than a frontend `setInterval`).
3. **Separately, and lower-priority for this specific incident**: this investigation also newly discovered (while reading the reducer/spec for context, git history on `fa001eac`/`1e57efa6`) that the *other* two bounded-timeout safety nets in this same state machine — `SUBMIT_TIMEOUT_MS` (`Submitting` phase) and `INTERRUPT_TIMEOUT_MS` (`Interrupting` phase) — were **both shipped reducer-only** (PR D #994 and PR C #991, 2026-05-23), with their commit messages explicitly deferring the actual dispatch-side `setTimeout` wiring in `useAgentStream.ts` to "a separate follow-up." A repo-wide grep (case-insensitive, whole repo, not just `frontend/`) confirms **that follow-up has never landed for either one** — `SubmitTimeoutElapsed`/`InterruptTimeoutElapsed` and their `schedule-*-timeout` events exist only in `types.ts`/`reducer.ts`/tests, with zero consumers anywhere in the actual application. This is a separate, pre-existing, wider-blast-radius gap (a pane stuck in `Submitting` after a lost backend ack, or stuck in `Interrupting` after a failed Stop, currently has **no recovery path at all** — not "slow," `none`) — not the direct cause of this specific incident (this pane was in `Streaming`, not `Submitting`/`Interrupting`, when it got stuck), but worth its own tracked follow-up given it was fully reducer-tested (99/99 passing) and so reads as "done" to anyone checking test coverage alone.

## Key files

| Concern | File |
|---|---|
| `[wave-turn]` transition + watchdog telemetry (shipped) | `frontend/app/store/agent-pane-state-store.ts` |
| `StreamWatchdogTick` / 180s liveness-recovery bound | `frontend/app/store/agent-pane-state/reducer.ts:280-365` |
| `StreamFlushObserved` re-promotion rule (risk #7 source) | `frontend/app/store/agent-pane-state/reducer.ts` (search `StreamFlushObserved`) |
| Watchdog `setInterval` (5s tick) | `frontend/app/view/agent/hooks/useTurnLifecycle.ts:233-239` |
| `onMount` call site | `frontend/app/view/agent/useAgentStream.ts:249-263` |
| Confirms tabs never unmount on background (`display:none`) | `frontend/app/workspace/workspace.tsx:22` |
| Global busy-panes aggregator (independent confirmation signal) | `frontend/app/store/agentActivity.ts` |
| Backend `[health] turn_active flip` telemetry (shipped) | `agentmux-srv/src/backend/blockcontroller/health.rs` |
| `ReconcileTurnActive` — deliberately excludes `Submitting`, only covers `Streaming` | `frontend/app/store/agent-pane-state/reducer.ts:367-436` |
| `SUBMIT_TIMEOUT_MS` defined, dispatch-side wiring deferred and never landed | `frontend/app/store/agent-pane-state/types.ts:517-525,905`; commit `fa001eac` |
| `INTERRUPT_TIMEOUT_MS` — same gap | commit `1e57efa6` |
| Prior telemetry audit (source of the already-shipped `[wave-turn]`/`[health]` logging) | `docs/reports/REPORT_WORKING_STATE_TELEMETRY_AUDIT_2026_07_27.md` |
| State machine spec (§8 = the timeout designs referenced above) | `docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md` |
