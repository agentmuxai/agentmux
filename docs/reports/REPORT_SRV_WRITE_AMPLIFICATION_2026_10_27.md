# Report: where srv's 7.2 GB of writes go in one lab run (macOS)

Date: 2026-10-27. Author: Maop@starpower. Asked by AgentY after the macOS baseline (`REPORT_MACOS_PERFORMANCE_BASELINE_2026_10_27.md`).
Build: `origin/main` `ef3028bda` (main is now `26d148f7a`: one commit later, which touches no storage code except `work_board.rs`). A production build of the app, with its srv replaced by a copy built from the same commit (ad-hoc signed, to allow the write counter; an unstripped build for the call stacks). No source was changed.
Load: the memory lab, 10 scripted agents over 3 tabs, 10 rounds, seed 1, fake Claude, isolated data folder per run. Apple M2, macOS 26.5.2.

## Summary

srv wrote **7.2 GB** in a run that stores **21.8 MB** of transcript (40,870 lines, about 560 bytes each): 331 times the payload. Three causes account for 98% of it, and all three are in how an appended transcript line reaches `db_file_data`:

| # | Cause | Bytes in the run | Share |
|---|---|---:|---:|
| 1 | Every appended line commits a rewrite of the whole 64 KB tail part, so SQLite writes about 15 WAL pages (62 KB) for a 560-byte line | 4,753 MB (WAL of the two stores) | 66% |
| 2 | The transcript is written to two stores (`agent:transcriptsinglecopy` is off): this doubles #1 and #3 | 3,391 MB less when switched on (7.2 to 3.8 GB, srv CPU 54 to 34 s) | 47% (overlaps #1 and #3) |
| 3 | `REPLACE INTO db_file_data` makes SQLite keep a statement journal that is bigger than its 64 KB in-memory limit, so it spills to a new temp file in `$TMPDIR` for every append | 2,331 MB | 32% |

Everything else is small: checkpoints writing the database files 138 MB (2%), `objects.db-wal` 4 MB, pipes 2 MB, the log 1 MB. Per-delta writes, `tsidx`, line-count and blockfile paths are not separate costs here (see "The causes you listed").

The simplest fixes remove most of it: turn single-copy on (-47%), replace the `REPLACE` by an in-place write of the tail (-87% of what remains in a model). Projected together: about 0.7 GB for this run (-90%), see "Fixes".

## How it was measured

No root, so no `fs_usage`. A small interposer library (`/tmp/maop-perf/wr/wrcount.c`, loaded into srv with `DYLD_INSERT_LIBRARIES` through a wrapper in the srv slot) counts every `write`, `writev`, `pwrite`, `fsync`, `F_FULLFSYNC` and `ftruncate`: bytes and calls per file path (looked up with `F_GETPATH` on every call), by call size, and it records the call stack of every 8th write call (weighted). The stacks were named from an unstripped build of the same commit.

Check: the counter's total against the lab's own per-process disk-write counter for srv:

| run | counter | lab (`io.writtenMB.srv`) |
|---|---:|---:|
| 2 (stripped srv) | 7,617 MB | 7,602 MB |
| 3 (symbolized srv) | 7,230 MB | 7,306 MB |
| 5 (symbolized, + sync stacks) | 7,250 MB | n/a (not read) |
| 4 (single copy on) | 3,839 MB | 3,905 MB |

They agree within 1.1%. Run 1 had a stale-file-number cache in the counter and is not used. Runs 3 and 5 agree to 0.3%.

## Where the bytes go, per file (run 3)

| file | MB | write calls | notes |
|---|---:|---:|---|
| `shared/agents/transcripts/filestore.db-wal` | 2,478 | 1,261,327 | 630,663 frames of 4,096 + 24 bytes: every frame is two writes |
| SQLite temp files in `$TMPDIR` (`etilqs_*`) | 2,331 | 247,757 | thousands of files, each 28 calls: statement-journal spill |
| `<channel>/data/db/filestore.db-wal` (the block's own copy) | 2,275 | 1,157,794 | the second copy |
| `shared/.../filestore.db` | 70 | 17,938 | checkpoint writes into the database |
| `<channel>/data/db/filestore.db` | 68 | 17,379 | same |
| `<channel>/data/db/objects.db-wal` | 4 | 2,286 | |
| pipes, sockets, the srv log, small json files, identity and wan stores | 4 | | |
| **total** | **7,230** | | |

By call size, the WAL files are 100% 4,096-byte pages plus 24-byte headers. 40,870 lines went into each store, and each store's WAL took 630,663 frames: 15.4 pages per appended line, 62 KB written per 560-byte line (111 times, for each store).

fsyncs are not a byte cost: 490 `fsync` in the run, all from WAL checkpoints (the 250 ms checkpointer), none per commit (`synchronous=NORMAL`).

## Cause 1: the whole tail part is rewritten on every line (4,753 MB)

`append_in_tx` (`storage/filestore/counter.rs:461`) reads the file's last part (up to 64 KB), appends the new bytes in memory, and writes the whole part back with `REPLACE INTO db_file_data` (`counter.rs:562`). The blob is stored over an overflow chain of 4 KB pages, so a part holding on average 32 KB (the tail of a growing file) rewrites about 8 pages for a 560-byte line, in a transaction of its own. Add the b-tree page(s) of the primary-key index, the `output.tsidx` stamp (its tail part is rewritten the same way, in the same transaction), and the `db_wave_file` row that carries `size` and the line counter, and the 15.4 pages per line add up (8 + 2 + 2 + 2, my arithmetic; the commit is measured as one, I did not separate the pages by table).

The sampled stack for the WAL bytes is `unixWrite < walWriteOneFrame < pagerWalFrames < sqlite3PagerCommitPhaseOne < vdbeCommit < sqlite3VdbeHalt`: the commit writing dirty pages. It is not a checkpoint and not a stray copy.

This is what the lab pays on any build that has the tail-part design; it is not tied to the framer (which never ran: 0 frames, see the baseline report).

## Cause 3: `REPLACE` spills a statement journal to a temp file on every append (2,331 MB)

The temp-file stack, from the symbolized run: `unixWrite < memjrnlWrite < subjournalPageIfRequired < pager_write < allocateBtreePage < sqlite3BtreeInsert < sqlite3VdbeExec < ... < Connection::execute < FileStore::append_in_tx`. SQLite keeps a statement (sub-)journal for a statement that may need to be undone half way, and `REPLACE` is one: it deletes the old row and inserts the new one. The journal holds the original content of every page the statement touches (4,100 bytes each). Held in memory up to 64 KB; a 64 KB blob row replaced is about 17 pages, 70 KB, so it crosses the limit and moves to a temp file, written in 61,504-byte chunks (15 pages) plus 4-byte page numbers. The file is thrown away at the end of the statement; the next append creates another (1,795 distinct temp files in run 2, and the table of that run overflowed; so the real count is higher). The bytes are pure overhead: the journal is never read.

It scales with the tail part's size, so it is worst when the tail is full. In my model (below) it is 28% of the cost of `REPLACE`.

## Cause 2: the transcript is stored twice (3,391 MB, overlaps the other two)

With `agent:transcriptsinglecopy` off every line is committed to the shared store and to the block's own store, each with causes 1 and 3. Switching it on (run 4, `--setting agent:transcriptsinglecopy=true`, confirmed in the app's config):

| | off (run 3) | on (run 4) |
|---|---:|---:|
| srv bytes written (counter / lab) | 7,230 / 7,306 MB | 3,839 / 3,905 MB |
| shared `filestore.db-wal` | 2,478 | 2,476 |
| `<channel>` `filestore.db-wal` | 2,275 | 1 |
| temp files | 2,331 | 1,288 |
| srv CPU seconds | 54.2 | 33.8 |
| round time, median | 6.3 s | 5.9 s |

It removes 47% of the bytes, 38% of srv's CPU, and one commit per line. It is a setting that already exists (SPEC_AGENT_TRANSCRIPT_SINGLE_COPY_2026_10_19, step B); this run only measures what it saves.

## The causes you listed

| Asked | Finding |
|---|---|
| the double copy | cause 2, 47% |
| per-delta writes | none: the framer's deltas apply to frames, and 0 frames were written (it frames only data over 24 h old and 8 MiB behind the end) |
| WAL churn and checkpoints | the WAL churn is cause 1 (commits); the checkpoints are 138 MB into the database files (2%) and 490 fsyncs |
| `tsidx` and index rewrites | part of the same transaction as the line (its tail part is rewritten like the output's); about 2 of the 15.4 pages by arithmetic; not measured apart |
| the `line_count` and blockfile paths | the line counter is a `db_wave_file` row update in the same transaction (about 2 pages); the blockfile `blob/` files are 3 KB each, 10 in all |

## Smaller findings

- **F_FULLFSYNC, 704 in a 5-minute run.** Not bytes, but each is a disk-cache flush that takes milliseconds on macOS: 373 from `registry::atomic::write_atomic` called by `LeaseStore::renew` (`leases.rs:398`; lease renewal writes and fully syncs a file about once every 0.8 s while agents run), 91 from `Workdir::write` in `realign_workspace_mcp_json` (`.mcp.json` rewritten on turns), 83 more attributed to the srv event log, and the rest by `claim_as`. These are for the platform owner: a lease renewal that is only a heartbeat probably does not need F_FULLFSYNC.
- The srv event log and the main log are small (1 MB); the log is not a cause.

## Fixes (proposals only, nothing was changed)

I replayed the append pattern against the real schema (triggers included) in a harness with the same counter: 2,000 appends of 500 bytes to one file, WAL mode, `synchronous=NORMAL`, SQLite 3.51.3, per-append commit unless stated. Bytes written (WAL plus temp files):

| variant | MB | vs now |
|---|---:|---:|
| now: `REPLACE` of the whole 64 KB tail part | 144 (103 WAL + 40 temp) | |
| A: `UPDATE` of the tail row (or `INSERT ... ON CONFLICT DO UPDATE`) | 91 (no temp file) | -37% |
| B: tail row created pre-sized (`zeroblob(65536)`); each append writes its bytes in place with incremental blob I/O | 19 | -87% |
| C: B and 8 appends per commit | 5 | -97% |
| D: A with 8 KB parts instead of 64 KB | 30 | -79% |
| E: A with 8 appends per commit | 12 | -92% |

1. **Turn `agent:transcriptsinglecopy` on by default.** -3.4 GB for the lab run, no code. What it needs is in the spec's step B (readers of the block's own copy, the older builds that read it); I did not check those.
2. **Stop using `REPLACE` for the tail** (variant A, a few lines in `append_in_tx` and `write_at`): removes cause 3 entirely (-2.3 GB) and costs nothing in behaviour. The revision triggers on `db_file_data` bump the file's `rev` only when existing bytes change (the `BEFORE INSERT` trigger for `REPLACE`, `BEFORE UPDATE OF data` for an `UPDATE`, both comparing the old content as a prefix of the new), so a pure append bumps nothing either way; the replay in the harness used the real triggers.
3. **Write the appended bytes in place** (variant B) is the fix for cause 1: it keeps the one-commit-per-line latency and touches the 1 or 2 pages that hold the new bytes instead of the whole part. What to verify before it: readers must accept a stored part longer than the file's size (the code already trims to `size`, and treats a longer stored part as a torn append, so this looks supported; the `unframe` and `lost_reads_blank` paths need a read); `sqlite3_blob_write` fires no triggers, so the `rev` bump on a changed prefix (`db_file_data_rev_update`) would be skipped: right for an append, which never changes a prefix, so the in-place path must only ever be used for appends; the framer must still skip the tail part (it does). The alternative that needs no format thought is **group commit** (variant C or E): collect the lines of a zone for a few milliseconds and write them in one transaction; it saves less per line at this arrival rate (about 70 lines a second per agent in the lab, so a 25 ms window collects about 2) and adds latency to every transcript event, which the code comment (`Phase 5a-3`) says matters.
4. Not recommended first: smaller parts (D) change the on-disk layout the framer and the readers share.

Projection for this lab run (model ratios applied to the measured bytes, not a measurement): single copy on, `UPDATE` instead of `REPLACE`, in-place tail writes: the shared store's 2,478 MB of WAL and its share of the temp bytes (about 1.2 GB) go from about 3.6 GB to about 0.5 GB, plus about 0.1 GB of checkpoints and the rest: about **0.7 GB, -90%**. With only the two no-risk steps (1 and 2): about 2.5 GB, -65%.

## Measured after PR 1 (UPDATE instead of REPLACE)

The same lab run (10 agents, 10 rounds, same app and frontend, only srv replaced; `origin/main` `0e5737719` plus the change; the storage code is unchanged between `ef3028bda` and that main), counted the same way:

| | before (runs 3 and 5) | after (run `pr1b`) |
|---|---:|---:|
| srv bytes written (counter / lab) | 7,230 and 7,250 / 7,306 and 7,304 MB | 4,016 / 3,999 MB |
| SQLite temp files | 2,331 / 2,338 MB | 0 |
| shared `filestore.db-wal` | 2,478 / 2,485 MB | 2,055 MB |
| channel `filestore.db-wal` | 2,275 / 2,278 MB | 1,822 MB |
| srv CPU seconds | 54.2 / 54.3 | 32.6 |
| round time, median | 6.3 / 6.2 s | 5.8 s |

-45% of the bytes (the model said -37%): the temp files are gone, and the WAL took 17% fewer pages too, because `REPLACE` also allocated a fresh overflow chain and freed the old one for every append. Cause 1 (the whole tail part rewritten per line) is untouched: 3.9 GB of the 4.0 GB is still the two stores' WAL.

## Limits of this measurement

- One workload: synthetic lines of about 560 bytes played by the fake Claude as fast as it goes. Real agents stream smaller deltas more slowly and will have a different arrival pattern; the per-line cost (60 KB) should hold for any line, the lines per second will not.
- macOS only. The Windows nightly shows the same order (8.6 GB for 12 agents), so I expect the same mechanism there, but I did not count per file on Windows. SQLite's statement-journal spill threshold is the same on every OS.
- The counter slows srv a little (a syscall per write for the path); bytes are not affected, CPU and round times are.
- My path table overflowed at 2,047 distinct files in runs 2, 3 and 5 (thousands of temp files), so the temp-file total is exact (it is confirmed by the stacks and by the sum matching the lab's counter) but not split by file, and an early reading of a "single large temp file with 663 full-fsyncs" in run 2 was this artefact. The F_FULLFSYNC callers above come from stacks, not from the file names.
- The model is one file and a simplified append (no `tsidx`, no `db_wave_file` size beyond one UPDATE); the ratios between variants are the point, not the absolute megabytes.
- One run each for runs 3 and 4, two instrumented runs agree to 0.3%; no repeat of the single-copy run.

## Reproduce

Files in `/tmp/maop-perf/wr/`: `wrcount.c` (the counter), `srv-wrapper-sym.sh` (sets `DYLD_INSERT_LIBRARIES` and execs the real srv), `analyze.py` (per path), `stacks.py` (symbolized stacks), `sqltest.c` (the model), the snapshots `wr2/3/5.tsv` and `live/` (run 4). The lab command: `node scripts/perf/memory-lab/run.mjs --portable <AgentMux.app> --srv /tmp/maop-perf/wr/srv-wrapper-sym.sh --panes 10 --rounds 10 --label x`, with `MAOP_WR_DIR` read by the counter. The interposer works only for an srv signed without the hardened runtime (an ad-hoc build) and only on the machine it was built on; it is a diagnostic, not something to commit.

Agent: Maop@starpower
