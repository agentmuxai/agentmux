# Spec: opening an agent shows its first screen at once, and loads the rest afterwards

**Status:** active. The frontend is in #5156. Left: srv's `agent:open_tail` (step 1) and the index and counter upkeep off the read path (step 3).
**Date:** 2026-10-19
**Author:** Camper (coordinator)
**Builds on:** `SPEC_AGENT_OPEN_LATENCY_2026_09_27.md` (#3927–#3965), `REPORT_AGENT_OPEN_STALL_RCA_2026_10_05.md` (#4371–#4417), issue #5141 (the store lock).

## 1. Ask

The owner, 2026-10-19: "when loading an agent, we need a way to not load the entire transcript at once. instead, we have a slice of the transcript that actually shows to the agent in the pane, that's all we want to load right away, if you need to load the rest too, can that be done after the agent loads asynchronously? … often when opening an agent it feels odd it sits and takes a long time … we also want a more graceful scheme for users."

## 2. What happens today (read from the code on `main`, 2026-10-19)

The pane already loads a tail rather than the whole transcript. It is still slow, because of what it does to get that tail and what it waits on.

1. **Three or four RPCs in a row before anything is shown** (`useHistoryPagination.ts:274-607`):
   - `AgentSessionReadCommand` (the snapshot);
   - usually `BlockfileLineCountCommand`, since a global snapshot always has an empty `sourceBlockId`;
   - `BlockfileReadRangeCommand` for up to 10,000 lines (`RESTORE_WINDOW_LINES`);
   - then a synchronous parse of everything returned.
2. **The read is far larger than a screen.** srv reads the whole window of up to 10,000 lines (about 5 MB in the 10-05 RCA) and only then trims it to about 1 MB (`tail_bytes`, `blockfile.rs:198-214`, `trim_to_tail_bytes` :632). A screen shows perhaps 20–60 rows.
3. **Reading can write, under the store's single write lock.** On the open path:
   - `db_generation` runs `catch_up_line_counter`, which takes the writer mutex (`counter.rs:653`) and may write;
   - a stale `output.idx` (the steady state for a live agent) is extended with `patch_derived_if` → `write_txn`, or rebuilt whole after a `replace_file`;
   - `stat` on a cache miss and `read_at` also take the writer mutex.

   So an open queues behind every agent's appends, on the lock that can be held for seconds when the disk is slow (#5141).
4. **The pane waits for all of it.**
   - The cover stays up until history has painted, at most 1.5 s, and then "Loading conversation…" shows (`usePaneReveal.ts`, `PANE_REVEAL_BOUND_MS`).
   - **The user can't send** until history is ready (`InitPending` suppresses `TurnStart`, `reducer.ts:683-690`).
   - Live records are held until history settles, for up to 15 s (`HISTORY_HOLD_MAX_MS`).
5. **Not on the open path:**
   - `transcript_backfill.rs` runs once ever, as a startup migration;
   - `session_archive.rs`'s restore is a user action;
   - the provider's JSONL is not read for the pane.

   Neither rewrite is part of a normal open. But a `replace_file` leaves the index to be rebuilt by the next read (point 3).

## 3. Design

### 3.1 First screen: one small read, no writes, no lock

- **One RPC, `agent:open_tail`**, returns everything the first paint needs:
  - the snapshot overlay;
  - the last *N bytes* of the transcript, cut at line boundaries;
  - the total size;
  - the generation;
  - the byte offset where the slice starts.

  It replaces today's three or four calls.
- **N is a first screen, not a megabyte:** about 64 KB by default, enough for the rows a pane shows plus a margin. It is sized by bytes, from the end, so it never needs the line index.
- **It reads only through the WAL reader pool** (`read_snapshot` / `SnapshotReader::bytes`), never the writer mutex. It never extends the index, catches up the counter, or writes anything. srv's existing `read_transcript_tail` (`resume_retry.rs:823`) is the pattern, moved off `read_at` (which takes the writer mutex) onto the reader pool.
- **The pane parses only that slice and paints.** The cover lifts and **sending is allowed** as soon as the first screen has painted: `InitReady` no longer waits for the rest.
- **Live records** stream in from the first screen's end offset, so nothing is lost or duplicated between the slice and the live stream (compare the generation and offset).

### 3.2 The rest: in the background, never in the way

- After the first paint, the pane loads the earlier part of today's restore window, up to the existing 1 MB, in background pages. The visible rows don't move: the virtual list anchors on the first visible row, as `loadOlder` does today. The pages load on idle (`requestIdleCallback`), and stop if the user scrolls, types or switches away.
- Older history beyond that keeps today's behaviour: pages on scroll-up, and the "Open Agent History" tab.
- **Index and counter upkeep move off the read path.** Extending `output.idx` and catching up the line counter happen in the append path (incrementally, as appends do today) or in a background task, never inline in a reader's RPC. A read that finds the index stale reads by bytes from the end, which needs no index.

### 3.3 What the user sees

- **Opening an agent:** its latest messages within one frame of the read returning. Target: under 300 ms on a warm instance, and never blocked by the store lock. The composer works at once.
- **Scrolling up during the background load:** content is there, or a quiet row "Loading earlier messages…" at the top. Never a full-pane cover.
- **Failure:** if the background load fails, the first screen stays usable, and the row offers a retry.

### 3.4 Later, if still needed

- **A persisted first screen per agent** (the 10-05 RCA's §4.2): the pane writes its last rendered slice on close, and the next open paints it before any RPC. Only if 3.1 misses its target.

## 4. Measurement

- **The existing `[agent-open]` line:** `history_start`, `history_read`, `parsed`, `painted`, `revealed`, `quiet`, read with `muxlog opens`. Add a `first_screen` phase, and `rest_loaded` for the background load.
- **The lab (P0.3):** time from opening an agent with a long transcript (for example 50 MB) to its first content, with and without the background disk load (`--disk-load`, #5151) and with other agents streaming. Also: whether the open took the store's writer mutex at all (`result.storeLock` call sites).
- **Targets:** first screen under 300 ms (warm) and under 1 s with agents streaming; the writer mutex never taken by an open; no change to scroll position, live streaming or sending.

## 5. Plan

1. srv: `agent:open_tail` through the reader pool, with tests. A read never writes. Owner: Maricon (srv store).
2. Frontend: first-screen open, `InitReady` on first paint, and live records from the slice's end offset. Then the background fill with anchor-preserving prepends. Owner: a frontend agent, to be named.
3. Index and counter upkeep moved off the read path (srv). Owner: Maricon, with #5141's series.
4. Lab metric and the `first_screen` phase (P0.3). Owner: Loap-3.

Each step lands behind measurement: before and after, with the `[agent-open]` phases and the lab's open time.

## 6. Risks

- **A gap or duplicate between the slice and the live stream.** Mitigation: the slice carries `gen` and its end offset; the live stream resumes from it, with a test that appends during the open.
- **Turn boundaries.** The older part loads later as its own parse, and a streamed text or thinking run that straddled the seam would get two ids and show as two rows. So the slice must start at a turn start (a user message), where no run crosses it:
  - srv cuts at a line boundary, then drops leading lines up to the first turn start, as `tail_turns` does;
  - if the newest 64 KB holds no turn start (one long turn), srv extends the slice backwards to the nearest turn start, up to the feed's restore budget (1 MB, today's restore size); only a turn longer than that starts mid-turn;
  - the pane checks the first line. A slice that starts below line 0 on anything but a user message isn't used: the pane opens the usual way, which parses the window whole. It also uses the first screen only for `claude-stream-json`, the format whose turns it can place a seam by.
- **Byte budgets.** `tail_bytes` and the feed's restore size are UTF-8 bytes. The pane counts the first screen from `end_byte - start_byte` when srv gives them, else from the lines' UTF-8 length, and each fill page the same way, so CJK and emoji text don't undercount.
- **Older builds sharing the store.** This spec only reads, so it adds no new on-disk format.
