# Spec: store calls stop pinning tokio workers while they wait for the connection

**Date:** 2026-10-19
**Author:** Maricon@charlie
**Type:** Spec (a scheduling change; no change to what is stored or read)
**Status:** active — the change and its tests are in PR #5148; remaining: Posa's A/B on the stalling Windows machine, and the write-volume work that goes with it (`SPEC_FILESTORE_WRITE_VOLUME_AND_GROUP_COMMIT_2026_10_19.md`).
**Issue:** #5141 (store stalls of 8–18 s when the disk stalls). Decided by Camper@narko as the first, independent fix.
**Platform:** all.

## 1. The problem

`FileStore`'s calls are synchronous and are made from async handlers on tokio workers (the refusal appends in the agent input handler, `cli_notice::append_to_pane`, `stat` and `read_at`, the turn-state writes). Every one waits for the single write connection (`lock_conn`). When the disk stalls, the holder is inside a write for seconds and everyone else waits for it, each on its own worker. srv runs eight workers; Posa's RCA on #5141 confirmed these paths run on workers and that up to eight callers queue, so a stall could take every worker and freeze unrelated work in srv (RPCs, timers, the UI's requests), not only the store.

## 2. The change

`filestore/off_worker.rs`: `off_worker(f)` runs `f` through `tokio::task::block_in_place` when the caller is on a multi-thread runtime, so the worker's other tasks move to another thread while `f` blocks; anywhere else (a plain thread, the blocking pool, a current-thread runtime, no runtime) it just calls `f`.

It is applied where the waiting happens:

- **`lock_conn`**: the wait for the connection's mutex, which is where the queue builds. Every direct user (stat, reads of a file's parts, list calls, the line counter's scan) is covered.
- **`write_txn`**: taking the connection, the write and the commit, whole, so the holder of a write does not sit on a worker either. Every append, `write_file`, `make_file`, copy, truncate and replace goes through it.

The slow-connection log still names the caller's site: `write_txn` captures its `#[track_caller]` location before entering the closure and passes it to `lock_conn_at`.

## 3. What it does not do

- It does not shorten a stall: callers still wait for the connection, and a user's `AgentInput` still waits for its own write. It stops the wait from taking the rest of srv with it. Shortening the stall is the write-volume spec.
- Reads that hold the connection (`stat` after a cache miss, a read of parts) do their query on the calling thread; they hold it briefly and one at a time, so at most one worker is occupied by such a read.
- The store's read connections (the reader pool) are untouched.

## 4. Tests

`tests_off_worker.rs`: with the connection held on a plain thread for 600 ms and six async tasks (on a two-worker runtime) calling a write and a read, an unrelated task that sleeps 10 ms at a time never waits more than 250 ms. Without the change the same test measures a 550 ms gap (the negative control, checked before committing). A current-thread runtime and a thread without a runtime call straight through.

## 5. Measurement

Posa on the machine that stalls (Area54): the lab's A/B on the same pinned commit, with `result.storeLock` (#5142) and the longest round, counting runs where unrelated handlers (the 1–2 s non-store handlers #5141 saw during a hold) are no longer delayed. The write-volume work is measured separately; this change is expected to leave bytes written, store lock holds and CPU unchanged.
