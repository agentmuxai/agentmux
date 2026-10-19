# RCA: intermittent 8–25 s stalls in agent turns on a Windows 10 machine (Area54)

Date: 2026-10-19. Author: Posa@Area54. Status: **cause chain evidenced; write-versus-flush not measured** (needs an elevated trace).

## Summary

srv writes about **5.3 GB** to disk during the memory lab's 4-agent, 10-round run, for about 5.7 MB of played turns (142 KB per scripted turn × 40) and a 9.9 MB final transcript database: roughly 930 times the turn bytes. It does this in bursts of up to 177 MB/s and 20–50 thousand write operations per second. On Area54's disks (a SATA SSD that is 79% full, and two hard disks) the volume stalls from time to time; srv's one transcript-store write connection, behind one `std::sync::Mutex`, is then held for 5–14 s, every other store caller waits for it, and agent turns (and sometimes an `AgentInput` RPC) take 10–25 s instead of 4–9 s. The same 5.3 GB is written on every run, stalled or not; what changes is whether the device keeps up. On the slower hard disk, every run stalls.

## Setup

Memory lab (`scripts/perf/memory-lab`), `--panes 4 --rounds 10 --seed 1`, production portable `b1cc61df` (v0.59.34), `RUST_LOG=debug` for the srv logs. Area54: Windows 10, i7-7700K (8 logical CPUs, so 8 tokio workers), 22 GB RAM free, C: = PNY CS1311 480 GB SATA SSD (79% full), D: = ST3500820AS HDD, E: = SAMSUNG HD103SI HDD. Defender real-time protection is off (the only registered antivirus). Samplers: system counters at 1 Hz, per-process I/O counters (`GetProcessIoCounters`) at about 1 Hz, and the size of the store's WAL once a second.

## Evidence

**1. The seconds are in srv's store lock.** `TimedConn` (`crates/srv/src/backend/storage/filestore/core.rs:39-77`) logs a hold of 500 ms or more; `lock_conn` (`core.rs:193-203`) logs a wait of 500 ms or more.

| runs (disk) | runs with a round over 10 s | longest round | longest hold |
|---|---|---|---|
| da1–da12 (C:, SSD) | 4 of 12 | 23.6 s | 11.4 s |
| ca1–ca5 (C:, SSD) | 2 of 5 | 19.5 s | 14.4 s |
| da21–da23 (D:, HDD) | **3 of 3**, every round 10–25 s | 25.0 s | 8.5 s, with 39–94 holds per run |
| earlier runs without debug logging (C:) | 4 of 6 | 17.1 s | n/a |

Runs on C: without a stall had no hold over 2 s (0–4 holds of 500 ms or more); a stalled run always had one of 3.7–14.4 s. The holder differs from run to run (`blockcontroller/shell/file_ops.rs:291`, `agent_session/helpers.rs:31`), so no one function is at fault: whoever holds the lock while the volume stalls holds it for the length of the stall.

**2. `AgentInput` waits behind it, sometimes.** In da9 srv's own `agentinput` handler took 10 364 ms with `dispatch` at 0.01 ms (`[rpc-perf]`, `rpc/engine.rs:562`), finishing about 2 s after a 9.4 s hold released; the lab's `slowEvals` recorded the same call (10.5 s). In da7, da11 and ca3 the `AgentInput` calls were fast (under 100 ms) and the extra round time was the agents' turn writes waiting behind the lock.

**3. The writer is srv itself.** Per-process I/O over the 40 s around the ca3 stall (08:58:14–08:58:56 UTC): srv 1 162 MB in 460 032 write operations; the lab's host process 103 MB; everything else under 3 MB. Over a whole run srv writes 5 283–5 435 MB and 1.93–1.94 million operations, in all 8 runs measured (ca1–ca5, da21–da23), clean or stalled. The operator's own AgentMux srv on the same disk wrote about 2 MB/s at the time.

**4. The stall is the volume, not only the store file.** During ca3's 14.4 s hold, a logger that only appends one line a second to a different file on C: (and `stat`s the WAL) froze for 16 s (08:58:35–08:58:51). In ca5 it froze for 3 s and 8 s around the 18 s round. Other C: logger gaps: ca1 12 s (about a minute into the run; its longest round was 9.9 s, so a volume stall does not always show as a long round), none in ca2/ca4 (clean).

**5. The stalls follow the device (answer to step a).** The same 5.3 GB written to D: (hard disk) gives 39–94 long holds and 20–25 s rounds in all 3 runs, against 0–4 holds and 2 stalled runs in 5 on the SSD. This shows that how often and how long the lock is held depends on how fast the disk absorbs srv's writes. It does not show that the SSD itself has a defect; a slower device is simply worse. narko (Loap-3) does not stall.

**6. The WAL.** The WAL file jumps in large steps (ca3: 0.24 MB at 08:58:16 to 84.5 MB at 08:58:20, 119.8 MB by 08:58:23) and ends each run at 183–386 MB for a 9.9 MB database (`filestore.db-wal` vs `filestore.db`). Each transcript append rewrites a whole 64 KB part (`checkpointer.rs` header comment), once in the block's store and once in the global transcript store. Background checkpoints are PASSIVE on their own connection (`checkpointer.rs:55`) and log only on failure, so I could not line individual holds up with individual checkpoints.

## Code path (answer to step c)

The `agentinput` RPC runs as its own task on the runtime workers (`rpc/engine.rs:378-380`, `tokio::spawn`), on a multi-thread runtime with Tokio's default worker count, one per logical CPU (`main.rs:52-56`), so 8 on Area54.

- **Directly on a worker thread:** `run_agent_turn_joining(...).await` (`server/agent_handlers/input.rs:1468`) is not wrapped in `spawn_blocking`. Inside it: `surface_refusal` → `handle_append_block_file` (`input.rs:884`) → `append_transcript_lines` (`shell/file_ops.rs:286-291`) → `lock_conn`, on refusal paths only; `cli_notice::append_to_pane` (`input.rs:1482`) for a hidden re-delivery card; and the persistent controller's spawn path, which calls `FileStore::stat`/`read_at` (`core.rs:540`, `lock_conn` on a cache miss) via `pane_history_session_id` and `has_prior_transcript`.
- **Handed off:** the persistent controller's user-line write goes through `blocking::off_async_worker` → `tokio::task::block_in_place` (`persistent/queue.rs:662`, `blocking.rs:28-36`). That frees the worker's other tasks, but the handler still waits for the lock, so its own latency includes the wait. The subprocess controller writes the user line on a dedicated OS thread (`subprocess/host_spawn.rs:276-296`), not awaited by the handler.
- **Not confirmed:** which connection `read_at` takes, whether the production stores have the reader pool (`core.rs:151`), and the store use inside `build_persistent_spawn_env`, `check_admission`, and the ACP and App Server `send_message` bodies. So I cannot say that a stalled lock delays *unrelated* RPCs; the code hands the main write off the worker, and the logs show some unrelated commands (`install.check`, `session:next_prompt_suggestion`) at 1–2 s during stalls, which is a hint, not proof.

## What a user sees (answer 2 of the brief)

`AgentInputCommand` is sent without its own timeout (`useAgentCommands.ts:924`), so the page waits; srv's default timeout is 5000 ms (`DEFAULT_TIMEOUT_MS`, `rpc/engine.rs:24`). A blocking `std::sync::Mutex` wait cannot be interrupted by `tokio::time::timeout`, and in da9 the 10.4 s call returned with no `EC-TIME`: the user sees a wait of about 10 s. When an await follows the blocked section, the timeout fires and the user gets an error: the pane logs it, the pending message is removed (`PendingMessageRejected`) and a turn the send started is rolled back (`TurnStartFailed`); that was the `EC-TIME: timeout (5000ms)` seen once in b2. Whether the cut-off handler leaves a message part-delivered was not checked.

## Maricon's three checks

1. **Do long holds line up with a WAL checkpoint?** Not determined. The WAL grows in steps of tens of MB and never shrinks during a run, and the checkpointer does not log a successful pass, so holds and checkpoints cannot be matched from the logs. srv's writes are the same in clean and stalled runs, which points at the device's capacity rather than at a rare checkpoint event.
2. **Write or fsync?** Not measured; an ETW/xperf file-I/O trace needs an elevated session (`xperf` and `wpr` are installed; I did not elevate). A hint: with `synchronous=NORMAL` in WAL mode (`store.rs`, `core.rs:249`) commits do not fsync, and an unrelated plain append to a different file on C: also froze for 16 s, which fits write-behind throttling of buffered writes on a saturated volume rather than one fsync.
3. **Is the writer srv?** Yes: 5.3 GB per run, 1.9 million operations, 20–50 thousand operations per second in bursts; no other process comes close.

## Not tested

- A real Claude agent (all runs use the lab's fake CLI). The store is shared by all agents, so I expect the same, but this is inference.
- Another machine with a fast disk, and the same load on the SSD with less written.
- Whether the lock wait stalls unrelated RPCs (see Code path).

## Where the evidence points (for the srv owner; no product change made)

- The store rewrites a 64 KB part per appended line, twice (block store and global store): about 930 times the bytes played. Writing fewer bytes per append is the direct lever. A single writer that groups appends (group commit) would also shorten how long the one lock is held. Which of these is right is Maricon's call; the timeout is deliberately unchanged.
- Measuring a fix on Windows: the lab's `result.storeLock` (longest hold, holds, waits; #5142) and the srv bytes written per run (the per-process sampler in this RCA's data) give a before/after on this machine; round time on the D: disk is the most sensitive indicator (every round slow).

## Raw data

On Area54, in `lab/`: `out3/` (da1–da12, debug logs), `out4/` (ca1–ca5), `D:/posa-lab-out/` (da21–da23, deleted after the numbers were taken), `io-ca.csv` and `io-da.csv` (per-process I/O, rows of 0.3 MB/s or more), `wal-*.txt` (WAL size once a second), `sys-sampler.csv`, `matrix-*.log`. The result.json files and srv excerpts are with Loap-3.

Agent: Posa@Area54
