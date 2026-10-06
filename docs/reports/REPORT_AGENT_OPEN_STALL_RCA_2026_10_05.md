# REPORT: Opening an agent can stall for ~17 s on the history read

**Date:** 2026-10-05
**Status:** root cause found (§2a) and fixed; instrumentation (#4371), 1 MB restore (#4373), bounded reveal (#4374) and the race fix with append-only index extension (§2a) landed or in review.
**Verified against:** `agentmux` `main` @ `742cb2ff5`. Logs from the retained instances on one Windows host (v0.58.2 → v0.59.10); measurements against a running v0.59.10 backend and the local transcript store, read-only.
**Related:**
- `SPEC_AGENT_OPEN_LATENCY_2026_09_27.md` (F6: the logs can't tell where an open's time goes; §5 acceptance: first row ≤ 300 ms);
- `SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md` §6.9 (the live feed);
- `REPORT_LIVE_FEED_ROLL_OFF_TOO_AGGRESSIVE_2026_09_30.md` (the turn cap removed).

## 1. Report

Opening AgentA (a long-running agent: 1.9 M transcript lines, 1.3 GB) from My Agents took about 19 s in a v0.59.10 portable. The user expects an open to feel instant.

## 2. What the logs and measurements show

**The open.** The frontend's `[agent-open]` line:

```
total=19145 cli=55 config=87 committed=246 history_start=120 history_read=17228
history_lines=10000 parsed=17274 painted=17628 quiet=17644
```

Everything but the history read is fast: CLI 55 ms, parse 46 ms, paint ~350 ms. The pane's cover waited on the history gate (`[pane-readiness] … still assembling after 8000ms; gate(s) pending: history`).

**The read.** The backend logged `blockfile:read_range` (offset 1,896,322, limit 10,000) at 02:19:02.148 and finished bringing `output.idx` up to date at 02:19:02.458 (`duration_ms=275`). Nothing more was logged for that call; the response reached the frontend at about 02:19:19.2. During those ~17 s the frontend had no long task, and unrelated requests were answered normally.

**The same read is cheap.** The identical request, sent to the running backend over its WebSocket RPC:

| Request | Time | Size |
|---|---|---|
| last 10,000 lines, index stale (extends it) | 810 ms | 5.0 MB |
| last 10,000 lines, repeat | 64–78 ms | 5.0 MB |
| the exact slow window (1,896,322 + 10,000) | 321 ms, then 78 ms | 4.6 MB |
| the 0.58.2 slow window (1,671,186 + 5,000) | 140 ms | 2.9 MB |
| last 2,000 lines | 16 ms | 1.0 MB |

Read directly from the store, the last 10,000 lines are 5.4 MB and take 19 ms. Of those lines, 9,399 are `stream_event` deltas (2.66 MB).

**Across every retained open (13).** The history read took 0.3–1.5 s in 11 opens and ~17 s in 2 (v0.59.10 and v0.58.2, both AgentA). Both slow opens overlapped a **continuation-packet spawn** of the same agent (`continuity: carrying AgentMux's record of the conversation into the fresh session` within a second of the read: the agent could not `--resume`, having moved from another instance). No fast open overlapped one.

**Ruled out, with evidence:**

- *Data size or content:* the exact windows replay in 78–321 ms.
- *Cold disk:* the store is on an SSD; the index update in the same call read 15 MB in 275 ms.
- *The frontend:* no long tasks; `ws.ts` handles messages synchronously.
- *The WebSocket connection:* RPCs are dispatched per message; other requests were served.
- *Blocking-pool starvation:* the runtime uses Tokio's defaults (512 threads).
- *WAL checkpoints:* the background checkpointer is `PASSIVE`; the `TRUNCATE` loop first runs after 30 minutes.
- *The activity-summary sweep:* it reads 96 KB of the per-channel transcript, which was empty.
- *The startup history index:* it starts 60 s after launch; in v0.58.2 that was 7.6 s before the read returned, so it doesn't line up.
- *Another instance holding the database:* the previous instance had exited 50 s earlier.
- *The 5 s line-count poll's silence during the stall:* it doesn't start until the history load settles, or the 15 s hold timer (`HISTORY_HOLD_MAX_MS`) forces it, which is exactly when it began (02:19:17.068).

**Not yet pinned:** where inside the backend the call waited. The backend logged only when the call started (F6 of the 09-27 spec, still open on the backend side).

## 2a. Root cause: a race sends the read to the whole-file fallback

`blockfile:read_range` reads through `output.idx`. When the first indexed read finds the index stale, it extends the index to `output`'s size S1, then reads again. The second read requires the index to cover `output` exactly. If anything appended to `output` in between, it sees size S2 ≠ S1 and returns `None`, and the handler falls through to its legacy whole-file path: `FileStore::read_file` loads the entire transcript (1.3 GB for AgentA) while holding the store's connection mutex, splits all 1.9 M lines, and returns 10,000 of them. That takes about 15 s and logs nothing.

The append comes from the agent being opened. A pane open that can't `--resume` (the agent last ran in another instance) spawns a fresh session carrying the continuation packet, and that spawn writes to the transcript within milliseconds, exactly inside the window between the extension and the second read. A resumed session doesn't write at that moment, which is why only the continuation-path opens stalled.

**Fix.**
- After extending, a read that an append has outrun is served from the indexed prefix (`read_via_stale_index`): only lines whose end the index knows, from one snapshot, same generation. The newest few lines are left to the pane's live stream.
- Extending no longer reads and rewrites the whole index. It reads the header and last entry, and writes the new header and the new entries in one guarded transaction (`FileStore::patch_derived_if`). On AgentA's 15 MB index that removes about 0.5 s from every open after the transcript has grown, the gap seen between `read_range` and `output.idx rebuild starting` in every logged open.
- A whole-file read of a transcript of 16 MB or more is logged, so any remaining path to it shows up.

## 3. Why it recurs (architecture)

1. **First paint waits on one large RPC.** The cover stays up until the history gate reports. There is no cached first screen and no fallback short of the 15 s hold timer, so any backend stall is a blank pane.
2. **The restore is sized in lines, not by what the pane keeps.** It always reads `RESTORE_WINDOW_LINES = 10_000` (~5 MB here) and parses it, then the live feed rolls off all but ~1 MB of finished turns. The `tail_turns` trim (open-latency §4.5) is sent only when a turn cap is set (`agent-view.tsx`), and the turn cap became opt-in on 2026-09-30, so since then every open reads the full window.
3. **One shared store behind one lock.** Every agent's transcript on the machine is in one SQLite file (2.5 GB here). Each process reaches it through one `Mutex<Connection>` (`filestore/core.rs`). Writes take that mutex and then wait up to `busy_timeout` (5 s) for another process's write lock while holding it. `read_file` loads a whole file under it. One slow holder stalls every agent's appends and every pane's history reads in that process. Each index update also rewrites the whole `output.idx` (15 MB for 1.9 M lines) on the open path.
4. **No latency guard.** The 09-27 acceptance criteria aren't enforced by a test. Each round tuned a constant (5,000 → 10,000 lines; 6 turns / 2 MB → 15 MB → 5 MB → 1 MB) instead of changing the shape.

**Side finding:** the startup history index now scans 63,000 sessions for 130–154 s on every launch (23 s when the 09-27 spec measured it).

## 4. Changes

1. **Instrument (this PR).** `blockfile:read_range` logs where its time went, as a warning when the call takes 1 s or more: choosing the store, the generation reads, waiting for a blocking-pool thread, extending `output.idx`, the indexed read. `FileStore` warns when a caller waits 500 ms or more for the connection, or holds it that long, naming the call site (`#[track_caller]`). The next stall names its step and the lock's holder.
   **Reproduce:** quit an agent with a long transcript in one instance, then open it from My Agents in another (a fresh spawn carrying the continuation packet), and read the backend log.
2. **Open instantly.** Bound the restore by bytes (~1 MB from the tail) instead of 10,000 lines, and let the pane reveal before a slow history read returns. Later, persist a ready first screen per agent.
3. **Storage.** Serve reads from a pool of read-only WAL connections so they never wait on the writer's mutex; append to `output.idx` instead of rewriting it; never `read_file` a transcript under the lock.
4. **A CI latency test:** open an agent with a large synthetic transcript and assert the §5 first-row budget.
