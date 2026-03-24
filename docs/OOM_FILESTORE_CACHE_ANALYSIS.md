# OOM Crash Analysis: FileStore Cache Memory Leak

**Date:** 2026-03-24
**Severity:** Critical — process crash (OOM)
**Component:** `agentmuxsrv-rs` → `backend/storage/filestore/core.rs`

---

## Incident Timeline

| Time (UTC) | Event |
|---|---|
| 2026-03-22 ~21:30 | sidecar pid=2700 starts, uptime clock begins |
| 2026-03-23 21:24–21:40 | Agent 1 runs in block `25d5f273`, exits cleanly (exit_code=0) |
| 2026-03-24 01:23 | Last terminal activity logged (`[raf-write]`) |
| **2026-03-24 01:32:52** | **OOM crash: `memory allocation of 1733122 bytes failed`** |
| 2026-03-24 01:32:52 | `TERMINATED exit_code=-1073740791 (0xC0000409)` — Windows `__fastfail` from Rust allocator |
| 2026-03-24 03:42:12 | App reopened, new sidecar pid=5552 starts |

Sidecar uptime at crash: **15,002 seconds (~4.2 hours)**

---

## Investigation Chain (Agents 1–3)

- **Agent 1** was running in block `25d5f273`. Had multiple clean subprocess exits (exit_code=0) between 21:24–21:40 — these were normal completions.
- **Agent 2** investigated Agent 1's death. Searched the March 23 sidecar log, found only clean exits, concluded "no crash." **This was wrong.** The crash happened at 01:32:52 on March 24 and was logged in the March 24 **host log** (`agentmux-host-v0.32.77.log.2026-03-24`), not the sidecar's own log file (which is date-stamped by start date). Agent 2 looked at the wrong log.
- **Agent 2's own death:** When the sidecar OOM-crashed, Windows job objects killed all sidecar child processes. Agent 2's Claude Code subprocess was a child of the sidecar — it was killed collaterally mid-operation, producing the "file not found" error.
- **Agent 3** (this agent) found the crash in `agentmux-host-v0.32.77.log.2026-03-24` at line with timestamp `2026-03-24T01:32:52`.

---

## Root Cause: Two Bugs in FileStore Cache

### Bug 1: Cache entries are never evicted (primary leak)

**File:** `agentmuxsrv-rs/src/backend/storage/filestore/core.rs`

`flush_cache()` only removes entries where `dirty == true`:

```rust
// flush_cache — lines 495–502
cache
    .iter()
    .filter(|(_, e)| e.dirty)   // ← only dirty entries
    .map(|(k, _)| k.clone())
    .collect()
```

However, **nothing in the codebase ever sets `dirty = true`**. Every write path (`make_file`, `write_file`, `append_data`, `write_meta`) writes directly to SQLite and sets `dirty = false`. Result: `flush_cache()` is a no-op. The cache HashMap grows monotonically for the lifetime of the process.

With multiple concurrent agents, each creating output files, object files, and terminal state — the cache accumulates one `CacheEntry` per file, per block, per zone, and none of them are ever removed.

### Bug 2: `data_entries` populated but never read (memory waste)

**File:** `agentmuxsrv-rs/src/backend/storage/filestore/core.rs`, `write_file` (lines 260–270)

`write_file` populates `data_entries` in the cache entry with full data part copies after writing them to DB:

```rust
entry.data_entries.clear();
for (idx, part_data) in parts.into_iter().enumerate() {
    entry.data_entries.insert(idx as i32, DataCacheEntry { ... });
}
entry.dirty = false;  // ← already in DB, dirty=false
```

But `read_file` never reads from `cache.data_entries` — it always loads parts directly from SQLite. So every `write_file` call stores a full duplicate copy of the file data in memory, which is then never used and never freed (due to Bug 1).

For a terminal scroll buffer file that grows to 1.7MB (the failing allocation size), this means ~1.7MB of data is stored in the cache entry's `data_entries` and stays there for the process lifetime.

---

## Memory Growth Model

Each active block has files for:
- Terminal output (circular, up to `maxsize`)
- Agent stream-json output (append-only, grows with every token)
- Object state (`ijson` files, metadata)

With 4+ hours of agent activity across multiple blocks:
- Object meta updates via `UpdateObjectMeta` → each creates/updates a CacheEntry, stored forever
- Agent output appends → `append_data` doesn't touch `data_entries`, but `make_file` creates entries that stay
- `write_file` calls (e.g., for ijson compaction, terminal state saves) → copies data into `data_entries`, never freed

The sidecar's last activity before the crash was `[raf-write]` (terminal data) followed 9 minutes of silence, then OOM. The terminal data write likely triggered a `write_file` for a ~1.7MB terminal state blob that couldn't be allocated because the heap was already exhausted by accumulated cache entries.

---

## Fix

Two changes to `agentmuxsrv-rs/src/backend/storage/filestore/`:

### 1. Add TTL-based eviction to `CacheEntry` (`cache.rs`)

Add `last_access_ms: i64` field. Update on every cache read/write. In `flush_cache`, evict clean entries older than `CACHE_TTL_SECS` (60s).

### 2. Remove dead `data_entries` population in `write_file` (`core.rs`)

Since `read_file` always loads from DB, there's no benefit to caching data parts for non-dirty entries. Remove the `data_entries` population block in `write_file` — this eliminates the duplicate data copies entirely.

---

## Testing

New test `test_cache_eviction_after_ttl` in `tests.rs` verifies:
1. A file created and accessed is in cache
2. After simulated TTL expiry, `flush_cache` evicts it
3. The file is still readable from DB after eviction (correctness preserved)

---

## Lessons

1. **`flush_cache` was always a no-op** — the dirty flag was never set. The periodic flusher was running every 5s and doing nothing.
2. **`data_entries` is vestigial** — likely carried over from a write-buffering design that was then changed to write-through. The field exists in the struct and is populated, but no read path ever consults it.
3. **Log file date-stamping** — sidecar log files are named by start date. A sidecar that starts on March 23 and crashes on March 24 will have its crash appear in the host log under March 24 but in its own log under March 23. Always check the host log for termination events.
