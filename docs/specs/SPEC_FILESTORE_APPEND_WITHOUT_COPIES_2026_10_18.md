# Spec: A transcript append without whole-part copies

**Date:** 2026-10-18
**Author:** Maricon@charlie
**Type:** Spec (less allocation in the store's append path; no behaviour change)
**Status:** proposed
**Part of:** track E (srv's memory) of the 500 MB budget (`docs/reports/REPORT_MEMORY_FOOTPRINT_500MB_BUDGET_2026_10_12.md`). Asked by Camper@narko, after the breakdown below.

## 1. The problem

srv's live Rust heap is small, about 31 MB at the end of a 12-agent tool-heavy run and 38 MB at its peak, but its private memory under that load is 114–126 MB. The gap is the allocator holding on to what the process allocated and freed: the same run with glibc limited to two arenas settles at 85 MB instead of 126 MB.

What feeds that is churn, not retention. Over the run srv allocated **19.9 GB** (about 22 MB/s). The biggest source is the transcript store's append (`FileStore::append_in_tx`): **9.1 GB**, and `read_bytes_covered`, which the same append calls, **1.5 GB**: together over half of all srv allocation.

Both copy a whole part into a fresh `Vec<u8>`:

- An append that doesn't start on a part boundary reads the last (partly filled) part, adds the new bytes and writes it back. The part is up to 64 KB (`PART_DATA_SIZE`). `r.get::<Vec<u8>>` copies it into a new allocation every time.
- The append also reads the file's last byte (the torn-line check) and its open last line (the line counter), each through `read_bytes_covered`, which copies every part it touches whole, 64 KB, to hand back one byte or a few.

So one small append makes three or four 64 KB allocations, all freed at once.

## 2. The change

No change to what is stored, when it is stored, or what any reader sees.

- **`read_bytes_covered`** borrows each part's bytes from SQLite's row (`row.get_ref(..).as_blob()`) and copies only the requested range into its output. Nothing else changes.
- **`append_in_tx`** reads the tail part into one reused buffer (`TAIL_SCRATCH`) instead of a fresh `Vec`, extends it, and binds it for the `REPLACE` as a slice. Appends run one at a time under the connection lock, so one buffer is enough; a buffer that grew past two parts (a torn, over-long part) is shrunk back after use.
- **Unchanged:** the SQL, the part boundaries (`PART_DATA_SIZE`), the transaction (the parts, the size and the counter are still written in one), the counter's rules, and durability: nothing is batched or deferred.
- **Not changed, on purpose:** `write_at` and the readers in `core.rs` and `offset_ops.rs` also copy whole parts; they are not on the hot path in the profile and are left alone.
- **What is left:** the bound parameter for the `REPLACE` is copied once by SQLite (it binds a blob as transient). That one 64 KB copy per append is in SQLite's own allocator, outside this change.

## 3. Tests

- **`tests_append_reference.rs`** (new): a plain `Vec<u8>` takes every append too, and the store must read back exactly what it holds. 1,500 raw appends and 1,200 line appends of a deterministic mix: one to 200 bytes (the common record), exactly to a part boundary, one short of it, one over it, spanning several parts, and empty. Reads at every boundary (three bytes each side) and the last 100 bytes after many of them; the offset each append reports and the size after each; the whole file at the end; and the line counter against the number of newline-terminated records. A third test closes a torn raw tail that ends exactly at a part boundary with the next line append.
- **Written first, against the unchanged code, and passing**, so it is a true baseline. Then the change; the whole store suite (102 tests) passes. As a check on the test, clearing the reused buffer was skipped on purpose: all three reference tests failed.

## 4. Measurement

Linux (charlie, Ubuntu 26.04, glibc), the repo's scripted fake-claude with a generated tool-heavy script (about 218 KB per turn), 12 agents in 12 tabs, the same lab build and the same scenario before and after; srv's private memory from `/proc/<pid>/smaps`, the allocation profile from a local `dhat` build of the same srv (8 rounds). Transcript growth was checked against the script's size, so the agents did play their turns.

**What it does: half the allocation, and the store's churn is gone.**

| bytes allocated by srv in the run | before | after |
|---|---|---|
| `FileStore::append_in_tx` | 9,202 MB | 93 MB |
| `read_bytes_covered` | 1,549 MB | 6 MB |
| **all of srv** | **19,876 MB** | **10,015 MB** |

That is less `malloc`/`free`/`memcpy` for every transcript line srv stores; the `SELECT` and `REPLACE` of the tail part stay, as before.

**What it does not do: lower srv's resident memory on Linux.** srv's private memory with 12 agents, glibc default arenas:

| | before | after |
|---|---|---|
| clean idle | 48 MB | 52 MB |
| 12 agents idle | 85 MB | 80 MB |
| loaded, rounds 5–12 | 114–126 MB | 105–126 MB |
| settled after the last round | 125.6 MB | 125.9 MB |

Single runs with a sawtooth, so the difference is within noise. Halving the allocation volume did not change what glibc retains. That is set by how many arenas the process's threads use: the same run with `MALLOC_ARENA_MAX=2` settles at 85 MB (a separate change). Whether the cut helps on Windows' heap is not known; it is measured there separately, and any such number belongs in `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`, not here.

**What is left of the churn** (after): the Linux cgroup member listing (`CgroupTracker::list_members`, about 3.3 GB, a per-sample re-read), about 3.5 GB of std and tokio allocations with no srv frame, the process-stats snapshot (about 0.7 GB), and the SQLite bind copy of the tail part, outside this profile.
