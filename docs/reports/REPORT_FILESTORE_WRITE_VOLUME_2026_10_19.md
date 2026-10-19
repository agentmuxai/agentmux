# Report: srv writes 930× the bytes it plays; what that costs and the plan to cut it

**Date:** 2026-10-19
**Author:** Maricon@charlie
**Status:** analysis — the measurements are done; the plan below is being executed one step at a time, each step with its own spec and code. Nothing in this report changes behaviour.
**Issue:** #5141 (AgentInput and turn writes stall 8–18 s behind the transcript store's single connection when the disk stalls)
**Evidence from others:** Posa@Area54's root-cause analysis of the stall on a Windows 10 test machine (not published here: it names that machine's disks, paths and timings; Posa and Camper hold it), and the memory lab's write counters and store-lock record by Loap-3@narko (#5144, #5149, #5150, #5142).

## 1. Summary

- On the lab's 12-agent, 10-round workload on Linux, srv writes **14.5 GB** (8.6 GB of it reaches storage) for about 17 MB of played turns (12 agents × 10 rounds × about 142 KB), roughly 850×. Posa measured the same on a Windows test machine with 4 agents: 5.3 GB for 5.7 MB of turns, about **930×**.
- The store lock stalls when the disk cannot absorb that. Whoever holds it while the volume stalls holds it for the length of the stall; everyone else waits (Posa's analysis, #5141).
- **Half of the writes are the receive-time sidecar, and half are a second copy.** A 30-byte record in `output.tsidx` rewrites the sidecar's last part, which costs as much as the line itself, so the two sidecars (one per store) are two of the four part rewrites per line. Writing a sidecar record at most once a second (`SPEC_FILESTORE_RECEIVE_TIME_SIDECAR_THROTTLE_2026_10_19`) is the first fix. Separately, the second copy of every line (the per-channel store) is half of all writes too: removing it in an experiment, never merged, cut srv's bytes written by **50.1%** and its CPU by **42.6%**.
- The cheap tuning that was proposed alongside it (`journal_size_limit`) does nothing for the volume, and sharing commits between appends (a group commit) does nothing for it either. Both were measured and dropped.
- Already shipped from this work: #5148 (a stalled disk no longer freezes the rest of srv: store calls no longer pin tokio workers), #5154 (the transcript writes moved out of the terminal-output file), #5160 (`agent:open_tail`, a first-screen read that never takes the writer lock).

## 2. What one transcript line costs

A line of an agent's output is written four times, in two commits, one after the other (`append_transcript`, `backend/transcript/append.rs`):

1. the block's own store: the `output` part, and the `output.tsidx` receive-time sidecar's part, in one transaction;
2. the global transcript store: the same two parts, in one transaction.

Each of those rewrites a whole 64 KB part, because `append_in_tx` does `REPLACE INTO db_file_data` with the part's old bytes plus the new ones. #5090 removed the in-memory copies of this; SQLite still writes every page of the new blob to the WAL, and the checkpoint copies them again. Measured with a file-backed store and the process's own `wchar` (everything it asks the OS to write), Linux:

| record size | bytes written per append | × the payload |
|---|---:|---:|
| 300 B | 74.9 KB | 256× |
| 2 KB | 89.0 KB | 46× |
| 8 KB | 105.4 KB | 13.5× |

The WAL reaches 183–386 MB for a 9.9 MB database in a run, because a PASSIVE checkpoint never shrinks the file and continuous writes rarely let the log restart.

## 3. What was measured

Memory lab (`scripts/perf/memory-lab`), 12 agents over 4 tabs, 10 rounds, seed 1, one portable built from main at 056a94157, only the srv executable swapped (`--srv`), runs interleaved, Linux (Ubuntu 26.04). Medians; each condition is compared with the baseline runs interleaved with it (4 for the group commit, 2 for the other two), and the baseline column is the 2-run figure.

| change | srv bytes written | reached storage | srv CPU | round time | runs |
|---|---:|---:|---:|---:|---:|
| baseline (main) | 14489 MB | 8616 MB | 104.7 s | 13.1 s | 4 |
| **no per-channel copy** (local experiment) | **7231 MB (−50.1%)** | **4303 MB (−50.1%)** | **60.1 s (−42.6%)** | 12.4 s (−9.9%) | 2 |
| `journal_size_limit` = 8 MB | 14488 MB (0%) | 8610 MB (−0.1%) | 106.9 s (+2.1%) | no change beyond noise | 2 |
| group commit of appends ("step 1") | 14983 MB (+3.2%) | 8417 MB (−2.4%) | 103.6 s (+0.8%) | −4.5% (noise) | 4 |

The experiment for the second copy was a local build with a switch that skipped the block-store append when the global one was written; it is not in any branch that is pushed for merging. srv's memory did not move (+1%). The store lock never reached 500 ms on this Linux machine in any run, so the stall itself cannot be shown here; it has to be measured on the Windows machine that stalls.

Side measurements that shaped the plan:

- A group commit saves a part rewrite only when several appends in one batch hit the *same part*. The appends in a batch are to different files (one transcript per agent, plus sidecars), so a batch of eight 300 B appends from eight files cost 73.6 KB each instead of 84.7 KB (−13%), and a lone append was unchanged. The code was written, tested and measured, and is kept on `maricon/filestore-group-commit-dropped`, not proposed.
- Appending as rows in a side table (one INSERT and the size UPDATE per transaction) costs 12.0 KB per 300 B line, 26.5 KB per 2 KB line and 34.6 KB per 8 KB line, because SQLite writes whole 4 KB pages: 3–6× less than a part rewrite, not 100×. With 8 or 32 appends sharing a commit it is 3.7 and 1.3 KB per 300 B line.
- Append latency today, Linux, local SSD, 300 B records: one quiet caller p50 1.35 ms / p99 2.75 ms; eight callers p50 9.6 ms / p99 27 ms.

## 4. The plan, in order

Each step lands with a spec, tests and its own lab numbers, before and after.

1. **Write the receive-time sidecar at most once a second per file** (`SPEC_FILESTORE_RECEIVE_TIME_SIDECAR_THROTTLE_2026_10_19`). The sidecar is two of the four part rewrites a line pays, a record is 30 bytes, and a replayed line's time has no use below a second. No contract changes: a line without its own record takes the newest record before it, which every reader (`stamps_for`) already does. Numbers in the spec.
2. **Retire the per-channel agent transcript copy, or write it lazily.** The measurement above is the upper bound: half the writes and over 40% of srv's CPU on this load. It is a contract change with live readers (the watchers, the ambient digest, `muxspect`, the session archive, the event's `b:<block>` position and byte offset), so the first deliverable is a spec that lists each reader and what it would read instead, and how older builds sharing the store behave; not code. For a live agent whose global zone has content the pane already reads the global copy (`global_output_source`).
3. **The other writers of the same lock.** Terminal output takes the shared lock per chunk (each chunk is its own commit and rewrites the file's last part: the store-level measurement above gives 89 KB written for a 2 KB chunk and 105 KB for an 8 KB one). The whole-file rewrites (`session_archive.rs`, `transcript_backfill.rs`) hold the lock for a copy of an entire transcript. The session snapshot (`agent:session:write_state`) is written twice, per-channel and global, as a whole file each time. None of these has a lab number yet; the lab has no terminal workload (requested from Loap-3).
4. **Fewer bytes per line (rows).** A side table with a fold, measured at 3–6× fewer bytes alone. It needs the old-reader rule (the transcript store is shared by every srv instance and deliberately has no schema bump) designed and tested against a pinned older binary before any code.
5. **Write-behind of the active tail.** Largest saving (5–25× under load), but a crash loses up to the flush interval of acknowledged lines and an event would be emitted when a line is buffered rather than durable. This goes to the operator before any code.

Already done in parallel (not part of the volume): the store calls stopped pinning workers (#5148), and `agent:open_tail` gives the pane a first screen from one reader snapshot (#5160), so opening an agent no longer needs the writer lock.

## 5. How these were measured, for the next person

- Lab: `node scripts/perf/memory-lab/run.mjs --portable <extracted tarball> --srv <srv built from the same commit> --srv-commit <sha> --panes 12 --rounds 10 --seed 1`, then `compare.mjs --base a1,… --head b1,…`. Write counters are `result.io.writtenMB` (`wchar` on Linux, `WriteTransferCount` on Windows) and `result.io.storageWrittenMB` (Linux `write_bytes` less cancelled, macOS disk bytes); the lock record is `result.storeLock`.
- An experiment switch does not reach srv through the environment. The lab strips every `AGENTMUX_*` variable, and the launcher does not pass the rest on to srv either: two of four conditions in my first matrix ran as the baseline for that reason, which showed up as byte counts identical to the baseline. The experiment build reads a file instead. Check that a switch took effect (a changed byte count) before trusting a condition.
- The store-level numbers come from a test that appends to a file-backed `FileStore` and reads `/proc/self/io` `wchar`.

## 6. Open questions

- Does the Windows machine that stalls stop stalling when the volume is halved? Posa's A/B on Area54, after step 1 of the plan, with `result.storeLock`, the longest round and srv's bytes written.
- Is a real Claude agent the same as the lab's scripted one? The lab uses a scripted stand-in; the write path is the same, and the lines are real-sized (about 140 KB per turn).
- How much of a real day's load is terminal output? Lock warnings from running instances so far point at agent transcript appends; terminal output is not yet measured on the same footing.
