# INCIDENT 2026-09-07 — srv 0.55.37 wedged: reactive-handler self-deadlock, then a stranded WebSocket task

**Severity:** Critical. Every agent-to-agent path on the host dead; UI frozen; all health signals green.
**Affected:** v0.55.37, srv PID 19280 (`C:\Users\asafe\Downloads\agentmux-0.55.37-x64-portable\`, ports 60237/60238).
**Also vulnerable:** v0.55.38 (what the operator moved to) and `main` @ `9b62c7a75`. Not fixed anywhere.
**Status at writing:** 19280 still wedged (~6h). Minidump captured: `C:\Users\asafe\workspace\srv-19280-wedged.dmp` (156 MB).
**Investigated from:** v0.55.38, by Claude (claude-0823b). Primary source for stage 1: Manpo's own postmortem, written from inside the wedged instance — `~/.agentmux/agents/manpo-0906k/POSTMORTEM-same-host-sendmessage-wedge-2026-09-07.md`. Every claim in it that I could check against source and logs checked out.

Times are UTC. Host local = UTC−7.

---

## 0. Corrections to earlier drafts of this document

Two claims in previous revisions were wrong and are retracted:

- **"Renderer PID 16696 stopped reading the socket."** 16696 is a CEF *utility* process — Chromium's network service, which owns every socket in the browser. Its ownership of the WS connection is unconditional and says nothing about any renderer. No client-side stall is supported by any evidence.
- **"The agent pipeline is wedged; the agent produced nothing for five hours."** The zero-indexing signal I relied on (`blockfile:line_count` → `output.idx rebuild`) is a *frontend poll*, so it stopped because the UI stopped, not because the agent did. Manpo's agent kept working for eight more minutes after the freeze — it wrote its postmortem to disk at 07:10:34 — then finished its turn at 07:10:48 and has been legitimately idle since.

The corrected picture below is stronger, not weaker: the operator's symptom "agents don't respond to anything" is real and has a single cause, but the agents themselves are fine.

## 1. One-paragraph summary

At 07:00:21 the agent Manpo sent a same-host `SendMessage` to the agent Vmer, whose CLI process had exited two minutes earlier. That routed the message through the "controller not yet spawned" fallback, which runs the spawn *on the same thread while holding the reactive handler's `std::sync::Mutex`*; the spawn path then re-locks that same mutex from another module, and a std mutex is not reentrant. The reactive handler was permanently deadlocked from that instant. Two minutes later the operator typed into Manpo's pane; the resulting RPC handler, running on a tokio worker, took the same mutex on its registration tail and blocked that worker forever — with the WebSocket connection's task stranded in that worker's non-stealable LIFO slot, where it had just been placed by a wake from the very same handler. From then on the srv never wrote another byte to the UI. The uptime counter froze on the last tick it had received (`3:28:25`), the status dot stayed green because it measures connectedness rather than progress, and after 256 silent seconds the bounded telemetry lane filled and began logging one identical WARN per second, 18,000 times, changing nothing.

## 2. Timeline, verified to the second

| UTC | Event | Source |
|---|---|---|
| 03:33:54.559 | srv 19280 starts; `sysinfo loop started`; monotonic uptime anchor | log |
| 06:49:27 | Manpo (block `3439a3c5`) persistent process spawned → PID 11296 | log |
| 06:57:16 | Vmer (block `7bfc1d56`) spawned; 06:57:51 given a task | log |
| 06:58:18.98 | Vmer `persistent stdout reader finished`; 06:58:19.00 `turn_active flip (process exited)`. **Vmer now has no live controller.** | log |
| 07:00:21.233 | Manpo calls `mcp__agentmux__SendMessage(to="Vmer")` | Manpo transcript, line 611 |
| 07:00:21.239 | srv: inject received → `reactive delivery: persistent controller not yet spawned — starting a turn instead` / `starting agent turn (no PTY fallback)` | log; `bootstrap.rs:1958`,`:2017` |
| 07:00:21.501 | `persistent process spawned` (PID 21920); `assign_process failed … AssignProcessToJobObject` (WARN) | log; `persistent.rs:2689` |
| 07:00:21.5 | **Stage 1 deadlock.** `muxbus: auto-registered persistent agent` (`persistent.rs:2808`) never logs. Manpo's tool returns `request failed: error sending request`. | log absence; transcript |
| 07:02:19 | Last sysinfo tick delivered to the UI. Displayed uptime `3:28:25` = `07:02:19 − 03:33:54` exactly. | arithmetic |
| 07:02:20.027 | Operator submits to Manpo's pane → `AgentInput block_id=3439a3c5` over WS conn `f90eaee1` | log; `input.rs:861` |
| 07:02:20.031 | `turn_active flip active=true was_active=true` (Manpo was mid-turn) | log |
| 07:02:20.045 | `emitted agent-message-accepted` — the last frame the srv ever tried to send that connection | log; `persistent.rs:1526` |
| 07:02:20.05 | **Stage 2.** Handler blocks its worker in `Mutex::lock`; WS task stranded. | inferred, see §4 |
| 07:02:20 → 07:06:35 | 256 sysinfo ticks queue in the bounded background lane. **Silent.** | `BACKGROUND_LANE_CAPACITY = 256` |
| 07:06:36.559 | Lane full → `ws egress lane full … consumer stalled, dropping event`, 1/s, 18,230+ times | log |
| ~07:06 | Manpo kills PID 21920. No effect — the wait is on a mutex, not the child. | Manpo |
| 07:07:02 | Operator launches 0.55.38 | launcher log |
| 07:10:34 | Manpo writes its postmortem to disk — **still working, not wedged** | file mtime |
| 07:10:48 | Manpo `turn_active` transition; idle since (0 CPU over a 6s sample) | log; live |
| 12:52+ | 19280 alive, 12 of 16 workers healthy, `mem_attribution`/`identity`/`lan_discovery`/`ui-liveness` all normal, reactive layer dead | live |

Two independent numbers tie the two stages together: `07:02:19 − 03:33:54 = 3:28:25` (the frozen display) and `07:06:36 − 07:02:20 = 256 s` (the lane capacity at 1 Hz). Neither was fitted; both fell out of the logs.

## 3. Stage 1 — the reactive handler deadlocks itself (proven)

**Trigger:** any same-host `SendMessage` to a persistent agent whose CLI process is not currently running. Persistent agents exit after each turn, so this is the *normal* state of an idle agent, and (per the code's own comment) the state of every agent after any srv restart until a human sends it something from the UI. This is the common case, not an edge.

**Chain, every link verified in source at `main` @ `9b62c7a75` and byte-identical in `v0.55.37` and `v0.55.38`:**

1. `backend/reactive/handler.rs:6` — `use std::sync::{Mutex, OnceLock}`. `:1076` — `inner: Mutex<Handler>`. **Non-reentrant.**
2. `handler.rs:1181-1182` — `pub fn inject_message(&self, req) { self.inner.lock().unwrap().inject_message(req) }`. **The guard is held for the entire delivery.** A test (`reactive/tests.rs:2622-2640`, `the_handler_lock_is_held_across_the_message_sender`) *pins this as intended behaviour.*
3. `bootstrap.rs:1958` — target has no live controller → fallback. `:2019` — `tokio::task::block_in_place(|| handle.block_on(run_agent_turn(…, TurnRegistration::Skip)))`. **Same thread, guard still held.**
4. `Skip` guards exactly one re-lock: the caller's own tail at `server/agent_handlers/input.rs:720` (`get_global_handler().register_agent`). The author documented this precisely (`input.rs:249-270`: "*LOAD-BEARING … would block the thread on a lock it already holds, wedging the reactive handler process-wide*").
5. `run_agent_turn` → `persistent_ctrl.send_message()` (`input.rs:529`) → controller spawn path → **`persistent.rs:2801`: `get_global_handler().register_agent_with_nonce(…)`** → `handler.rs:1119` → `self.inner.lock()`. Same thread. Already held. **Deadlock.** The very next log line in that function (`:2808`) is the one that never appears.
6. Killing the spawned child does nothing; the thread is waiting on a mutex. Only a srv restart clears it.

**Introduced by** `a394a4580` (2026-09-03, "fix(reactive): deliver to persistent agents that have not spawned yet", #2960), an ancestor of `v0.55.37`. The fallback it added is what routes an idle target into the held-lock spawn. No commit since mentions it; `git log` for deadlock/re-lock/reentrant on the surface files is empty.

**Why review missed it:** the guard was written for the one re-lock visible from the call site. The second lives ~2,000 lines away in a different module, on a path only reached when the target is idle. Any integration test of the fallback against a real persistent controller would have hung rather than failed — and there is none.

**Blast radius of stage 1 alone:** every request path that takes this mutex hangs forever. I enumerated 30+ call sites; the ones that matter operationally: `/agentmux/reactive/inject` (`reactive.rs:850`), `/agentmux/discovery` (`mod.rs:735`), `/agentmux/reactive/agents` (`reactive.rs:1198`), the WS `bus:inject` command (`websocket.rs:598`), the fleet API (`fleet.rs:115`), the cloud subscriber (`cloud_subscriber.rs:559`), and both periodic watchers (`activity_watcher.rs:90`, `progress_watcher.rs:512`) — each of which pins one more tokio worker on its next tick. Unauthenticated `/` still answers in 1 ms because it never touches the lock, so any liveness probe reports healthy.

## 4. Stage 2 — the WebSocket task is stranded (mechanism inferred; timing exact; dump preserved)

Stage 1 alone does not freeze the UI: sysinfo kept arriving for two more minutes. What froze it was the operator's next keystroke.

**What the code does, in order, with no yield between steps 3 and 5 on the persistent-agent path:**

1. The `agentinput` RPC frame arrives on the WS task for conn `f90eaee1`. `websocket.rs:703` → `engine.handle_message()` → `engine.rs:277` **`tokio::spawn`** of the handler. In tokio's multi-thread scheduler a task spawned from a running task goes to **that worker's LIFO slot**. The WS task yields back to its `select!`.
2. The handler runs on the same worker: logs `AgentInput` (07:02:20.027), builds env (`identity: injected CLAUDE_CONFIG_DIR`, .0279).
3. `run_agent_turn(…, TurnRegistration::Register)` → `persistent_ctrl.send_message()` → `emit_message_accepted()` (`persistent.rs:1509`) → `broker.publish()` (`wps.rs:388`, **synchronous, on the caller's thread, no channel or spawn**) → eventbus `try_send` on the priority lane → **wakes the WS task**. A task woken from inside a running task on a worker is placed in **that worker's LIFO slot**. (07:02:20.045)
4. Between `:529` and `:720` on the persistent path there is **no `.await`** (the only two in that range are inside the container-agent branch). The handler proceeds without yielding.
5. `input.rs:720` — `get_global_handler().register_agent(…)` → `handler.rs:1106` → `self.inner.lock()`. **The worker's OS thread blocks forever**, with the WS task sitting in its LIFO slot.

The LIFO slot is not work-stealable (tokio 1.52.3, stock `#[tokio::main]`, `disable_lifo_slot` not set anywhere). `block_in_place` would have handed the core — and the slot — to a fresh thread; a raw `std::sync::Mutex::lock()` does not. The WS task is therefore never polled again: no sysinfo forwarding, no priority events (Manpo's remaining eight minutes of output never reached the screen), no pings, no `agent-message-accepted` promotion. The server simply stops writing. The client has nothing to react to, so the TCP connection sits `ESTABLISHED` indefinitely with the same conn id — exactly what was observed six hours later.

**Why this mechanism and not the alternatives:**

- *Blocked socket write / client stopped reading* — nothing client-side is stalled (§0), and 16696 is the network service, not a renderer.
- *`biased;` starvation from a priority-lane flood* — would burn CPU; the srv used ~0.5% over 8h.
- *Runtime-wide worker exhaustion* — 16 workers, four pinned (inject's `block_in_place` thread costs none; the RPC handler, and the two watchers on their next tick); everything on the other twelve kept running, which is precisely the selective survival in the logs. If all workers were dead, `/` would hang too. It answers in 1 ms.
- *The WS task itself took the lock* — the only inline lock on the WS path is `bus:inject` (`websocket.rs:598`), and the frontend never sends it (its complete wscommand set is `rpc`, `blockinput`, `setblocktermsize`).

**What would confirm it, for whoever has WinDbg and the 0.55.37 PDB** (none ships in the portable; no debugger on this host): in `srv-19280-wedged.dmp`, expect (a) one thread in `Mutex::lock` under `register_agent_with_nonce ← spawn_process ← send_message ← run_agent_turn ← block_on ← block_in_place ← inject_message_inner`; (b) a second thread in `Mutex::lock` under `register_agent ← run_agent_turn ← handle_request`; (c) two more under `list_agents` from the watchers; (d) the WS connection future for `f90eaee1` resident in thread (b)'s core LIFO slot and absent from every run queue. If (d) is absent, the mechanism is something else and this section should be revised.

**Note the determinism.** Once stage 1 has happened, *every* UI submit to a persistent agent strands the WS task of the connection that carried it, because the `Register` tail always takes the lock right after the accepted-event wake. The first keystroke after the deadlock freezes the window. That is why the operator saw "agents don't respond to anything" rather than "one message failed".

## 5. Stage 3 — why nothing noticed

1. **Status ≠ progress.** The status dot reflects process/connection liveness. Both were genuinely fine for six hours.
2. **The frontend holds the last value by design.** `resolveUptimeSecs` returns `null` on a missing tick and the caller keeps the previous value "rather than flashing a zero" (`frontend/app/statusbar/backend-uptime.ts`). A frozen plausible number conceals this; a zero would have screamed.
3. **256 seconds of pure silence.** The lane buffers before it drops. The first WARN came four minutes after the UI froze.
4. **18,230 identical WARNs, no escalation.** The bounded lane worked exactly as designed and correctly identified a stalled consumer. Nothing was listening.
5. **The liveness probe measures the wrong process.** `[ui-liveness] UI thread alive rtt=0ms` passed throughout. It probes the browser-process UI thread, which was fine. It does not traverse the WS data path, the srv's async runtime, or the reactive lock.
6. **`try_send` drops the newest telemetry.** For latest-wins data this retains 256 stale readings and discards every fresh one.
7. **The warning text misdirects.** "consumer stalled" pointed diagnosis at the client for hours. The consumer was fine; the producer's task was stranded.

## 6. Invariants that must exist (requested by the operator)

> **I1. A monotonic counter presented as live MUST advance.** `status == up` with `uptime_secs` unchanged across N expected ticks is an emergency, not a state. Key on *last change*, not last receipt.

> **I2. An active turn MUST show progress.** `turn_active = true` with no output bytes and no index events past a bounded interval is a wedged turn. Surface it as such, distinct from "thinking".

> **I3. No lock may be held across a delivery, spawn, or any operation that can re-enter the module.** Enforce with a hold-time watchdog (log at 5 s, `reactive:wedged` event at 15 s) and a lint/test asserting `get_global_handler()` is unreachable from anything `inject_message_inner` invokes.

> **I4. Health endpoints MUST NOT take application locks**, and MUST report lock age. `/` answering while the reactive layer is dead is a false negative by construction.

> **I5. Never block a tokio worker on a std mutex from async context.** Either the mutex is `tokio::sync::Mutex`, or the call is wrapped in `block_in_place`/`spawn_blocking`, or it uses `try_lock_for` with a timeout that returns an error instead of hanging. The LIFO-slot strand in §4 is the direct consequence of violating this.

## 7. Fixes

**P0 (either alone stops stage 1; do both):**
- `bootstrap.rs` inject fallback: resolve agent→block under the lock, **drop the guard**, then spawn and start the turn. If ordering matters, use a per-block async lock, never the global sync one.
- Thread `TurnRegistration::Skip` into `PersistentSpawnConfig` so `persistent.rs:2801` skips its own registration when the spawn originates from an inject (the agent is registered by construction there).

**P0 (stops stage 2 regardless of stage 1):** make `input.rs:720` and every other async-context caller of `get_global_handler()` non-blocking per I5. The cheapest global fix is converting `ReactiveHandler.inner` to `parking_lot::Mutex` with `try_lock_for(5s)` returning an error — a wedge then degrades to failed requests instead of pinned workers.

**P0 (test):** `inject_message` to a registered-but-unspawned persistent controller must return within a bounded time, under `#[tokio::test(flavor = "multi_thread")]` inside a timeout, so a regression fails instead of hanging CI.

**P1:** I1–I4 above; lane eviction drop-oldest for latest-wins telemetry; rate-limit and escalate the first `TrySendError::Full` per (conn, lane), naming the lane; `InjectionResponse.delivered_via` so the MCP stops reporting "sent" for an enqueue (Manpo's separate finding, `ISSUE-sendmessage-false-success.md`); fix `AssignProcessToJobObject` failing on every spawn on this host (job-object tracking is silently absent, `kill_tree` is a no-op).

**Operational, until a fix ships (0.55.37 and 0.55.38):**
- Do not `SendMessage` to a same-host agent unless it has a live turn. If its controller is unspawned, write a file into its workspace and have a human poke it from the UI.
- After any srv restart, a human sends each persistent agent one message from the UI before agents talk to each other.
- Recovery from a wedge is a srv restart. Killing children does nothing. Every UI submit into a wedged srv strands one more connection.

## 8. Evidence index

- Launcher log: `~/.agentmux/logs/agentmux-launcher.log` (contains NUL bytes; use `grep -a`).
- Manpo transcript: `~/.agentmux/shared/identities/…/projects/C--Users-asafe--agentmux-agents-manpo-0906k/3ca084c3-….jsonl`, line 611.
- Manpo's postmortem and issue: `~/.agentmux/agents/manpo-0906k/`.
- Minidump of the wedged srv: `C:\Users\asafe\workspace\srv-19280-wedged.dmp`. **Capture more before restarting 19280 if you have better tooling; this state is perishable.**
- Source clone at `main` @ `9b62c7a75`: `C:\Users\asafe\workspace\agentmux`. Tags `v0.55.37` = `5d65cbe58`, `v0.55.38` = `b3ea05181`.
- Live state at writing: srv 19280 `Responding=True`, 165 s CPU total; WS conn `127.0.0.1:60238 ← 127.0.0.1:62943` `ESTABLISHED`; 0.55.38 srv 8524 clean (zero egress warnings).

## 9. What went right

The bounded lane capped memory instead of growing without limit, and hitting the cap was — as its own comment says — the signal that the consumer wasn't keeping up. The signal was accurate. Manpo, from inside a srv whose tools no longer answered, reconstructed stage 1 from the launcher log and source in ten minutes and left the write-up on disk where it could be found. That file is the reason this document has a root cause instead of a hypothesis.
