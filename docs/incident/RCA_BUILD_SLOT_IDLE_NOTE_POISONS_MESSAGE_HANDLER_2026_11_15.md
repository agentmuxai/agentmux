# RCA: one idle-slot note crashed a thread and stopped all agent messaging until restart

**Status:** implemented. The fix is in the PR that carries this file (task #2577) and ships in 0.60.29. RCA by AgentY@narko, 2026-11-15; fix by AgentA@Area54.
Severity: P0. On the affected machine, every agent-to-agent message, worker spawn and Kanban load failed from the first crash until AgentMux was restarted, about 2 hours in this case.
Versions: 0.60.27 (seen), 0.60.28 and main as of 2026-11-15 (same code, not fixed).

## What the person saw

- The Kanban did not load.
- Agents stopped hearing from each other. Messages to agents failed with `task ... panicked with message "called Result::unwrap() on an Err value: PoisonError"`, and some MuxAPI calls with "connection closed before message completed".
- A request to spawn agents failed.
- AgentMux itself kept running, and its health check kept answering. Nothing on screen said anything was wrong.

## Timeline (UTC)

| Time | Event |
|---|---|
| 2026-11-14 | #7239 merges: a build-slot holder that has used no CPU for 10 minutes is told once, by a system note. It ships in 0.60.27. |
| 11-14 22:44:10 | The watch tells one agent its slot is idle. That agent was in the middle of a turn, and the note is delivered without trouble. |
| 11-15 00:45:20.676 | The watch tells another holder, which had no turn in flight. Delivering the note opens a turn (`[turn] opened`, `turn_active flip active=true`). |
| 00:45:20 | `thread 'build-slot-idle' panicked at crates/srv/src/backend/blockcontroller/persistent/status.rs:91:9: there is no reactor running, must be called from the context of a Tokio 1.x runtime` |
| 00:45:21 on | `panicked at crates/srv/src/backend/reactive/handler.rs:1416:27` and `:1444:27`, `PoisonError`. About 30,000 panics over the next two hours, one per message, list or delivery attempt. |
| ~02:32 | Found while sending messages for a release. Diagnosed from the log and the code. |
| (pending) | AgentMux restarted, which cleared it. |

## Cause

1. `crates/srv/src/backend/build_slot_idle.rs` `install()` starts a plain OS thread (`std::thread::Builder::new().name("build-slot-idle")`). Once a minute it calls `get_global_handler().deliver_system_note(...)`.
2. `ReactiveHandler::deliver_system_note` takes the handler's `inner` mutex and delivers while holding it.
3. Delivering to an agent with no turn in flight starts a turn. Starting a turn calls `spawn_status_heartbeat`, which calls `tokio::spawn` (status.rs:91).
4. `tokio::spawn` panics on a thread with no Tokio runtime. The panic unwinds while the handler mutex is held, so the mutex is **poisoned**.
5. Every `ReactiveHandler` method does `self.inner.lock().unwrap()` (21 sites on main), so after that each one panics. Message send, inject, held-message delivery, system notes, the agent list (used by the Kanban and by spawn) and the audit log all fail. Nothing resets the lock short of restarting the process.

It needs a narrow condition, which is why it wasn't caught: an idle-slot note sent to an agent that is **between turns**. The first note, on 11-14, went to an agent in the middle of a turn, so no turn was started and no `tokio::spawn` ran.

## Why it was not caught

- #7239's tests call `check()` (the decision whether to notify) and not the delivery, on a plain thread, into an agent with no turn in flight.
- A crash in one delivery taking down every later one is a property of the handler (`lock().unwrap()` everywhere), not of #7239. Any panic anywhere under that lock, on any thread, has the same effect. #7239 was the first code to call the handler from outside the runtime.
- Nothing watched for it: health stayed 200, and panics go only to the log.

## Fix (one PR)

1. **The watch delivers inside the runtime.** Capture `tokio::runtime::Handle::current()` in `install()` (it runs during srv start, inside the runtime) and run each delivery through it, or make the watch a Tokio task that uses `spawn_blocking` only for the process snapshot. Add a test that delivers an idle note to an agent between turns from the watch's thread.
2. **A crash in one delivery never stops messaging.** One helper takes the handler lock and recovers from poisoning (`unwrap_or_else(|e| e.into_inner())`), with one warning in the log. All 21 sites use it. Check that the handler's state is consistent after a panic in the middle of a delivery. If some step can leave it half-done, make that step safe to repeat.
3. **Look for the same pattern elsewhere:** other plain threads that reach `tokio::spawn` through a delivery or turn path.

## Follow-ups

- **Make it visible:** when srv panics more than a few times in a minute, show it, both in an `[AgentMux]` note to agents and in the app's own status. In this case two hours passed before anyone looked.
- **Health should report the messaging path,** not only that the process answers.
- **Release:** the fix goes in a 0.60.29 patch. Until then any machine on 0.60.27 or 0.60.28 can hit it, and a restart of AgentMux clears it. A PC restart is not needed.

## What the fix did, and what it found

Written by AgentA@Area54 with the fix (task #2577).

1. **The watch delivers inside the runtime.** `build_slot_idle::tell_holder` enters the runtime handle that `install()` captures, for the delivery, and catches a panic in it, so one failed note no longer ends the watch thread. The test `the_idle_note_reaches_an_agent_between_turns_from_a_thread_with_no_runtime` runs the delivery from a plain thread: called directly it panics (the old behaviour), through `tell_holder` the spawned task runs.
2. **The lock no longer stays closed.** `reactive/handler_lock.rs` wraps the mutex. `lock()` and `try_lock()` recover from poisoning: they clear it, reset the delivery context and write one warning. The count above was low: the handler's lock was taken at 28 sites in `handler.rs` (21 on one line, 7 split over several) and at 42 in the reactive module. All of them go through the wrapper, and the compiler enforces it: the field's type has no `lock` that returns a `Result`, so a missed site does not build.
3. **A half-finished delivery left state behind.** `inject_message_inner` puts the sender's UID, the WAN verdict, the LAN-claimed UID and the trust label of the delivery in flight into the handler and restores them afterwards. A panic in between skipped the restore, so after a recovery the next audit entries would have carried another delivery's sender. Recovery resets those five fields (nothing is in flight once the thread that held the lock is gone). The registry maps are never changed between a call into a callback and the end of the change, so they need no repair. A test covers this (`a_jekt_that_panics_mid_delivery_leaves_no_sender_on_the_next_audit_entry`).
4. **The spawn path failed in its own way.** `try_register_agent_full` answered "reactive handler mutex poisoned" and `try_get_agent_by_block` answered nothing, for the rest of the process's life. That is why spawns failed. Both recover now.
5. **Other plain threads.** Searching srv's `std::thread` starts against the files that reach the handler found one more: `transcript-damage-note` calls `hold_system_note`, which queues a note and starts no turn, but takes the lock, so the recovery covers it. The other callers of `deliver_system_note` that were looked at run on Tokio tasks or `spawn_blocking`, which have the runtime. A thread that reaches the handler indirectly, through another module, was not traced.

Not in this change, and still open from the follow-ups above: showing repeated panics to agents and in the app, and a health check that covers the messaging path.
