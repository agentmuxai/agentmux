# SPEC: give disk space back after history compression

**Date:** 2026-10-23
**Status:** active — approved by the operator on 2026-10-23; built in #PR (all of §4–§6; §9's two PRs went in as one). See §11 for where the build differs. Follows `SPEC_HISTORY_STORE_COMPRESSION_AND_COMPACTION_2026_10_21.md` (the framer, #5324/#5328/#5330/#5331), whose §3 rules hold here too.
**Author:** Loap-3 (loap-loap-2-loap-3), for Agent5 (history store).
**Task:** Kanban #1797 (from #1104). Build: #1107.

---

## 1. Problem, with this machine's numbers

The operator turned on `agent:transcriptframes`. The framer has compressed the shared transcript store, `shared/agents/transcripts/filestore.db`, and is now in steady state: one or two frames per pass. But every part row it deletes leaves its pages free inside the file. Nothing in `backend/storage/filestore/` runs `VACUUM`, and the store has `auto_vacuum=NONE`. So the file is as big as before, and the status bar's history total (`backend/history_usage.rs`, sizes on disk) hasn't moved.

Snapshot of the live store, 2026-10-23 18:17 (SQLite 3.53.3, read-only online backup):

| | |
|---|---|
| File | 15.49 GB (3,782,638 pages of 4 KiB) |
| Free pages inside it | 2,912,313 = **11.93 GB (77 %)** |
| Live data | 869,159 pages = **3.56 GB** |
| `auto_vacuum` | 0 (none) |
| Per-channel copies (`channels/*/versions/*/data/db/filestore.db`) | 37 files, 8.19 GB, **0 free pages** |

So about 12 GB can be given back, all of it from the shared store.

## 2. Measurements

All on copies of the store in my workspace, on this machine (narko, NVMe), with SQLite 3.53.3 through `node:sqlite`. The scripts are in my scratch folder.

While each operation ran, a second process (`probe.mjs`) acted like another srv: every 500 ms it appended a row (a writer) and read a part (a reader), with `busy_timeout=5000`, the same as `FileStore`. Each figure is one run, good for planning, not a benchmark.

| Operation | Time | Disk needed | Other srvs during it |
|---|---|---|---|
| Online backup (snapshot, one read transaction) | 50 s | file size | none |
| `VACUUM INTO` a new file | **16.3 s** → 3.56 GB | live size (3.6 GB) | readers fine; nobody writes to the new file |
| `VACUUM INTO` with `auto_vacuum=INCREMENTAL` set first (source: a 3.56 GB copy) | 31.0 s; output `auto_vacuum=2`, `quick_check` ok, `user_version` and triggers kept | live size | — |
| In-place `VACUUM` | 42.6 s | WAL grew to 3.58 GB | the shrink waits for a checkpoint with no reader (see the note below) |
| In-place conversion (`PRAGMA auto_vacuum=INCREMENTAL; VACUUM`) | 66.9 s | WAL 3.59 GB | **writer: 13 of 18 writes failed** after the 5 s busy timeout; reader fine (worst 206 ms) |
| `incremental_vacuum` after conversion, 1.01 GB free (246,711 pages) | 121 steps of 2,048 pages, **21.1 s**; worst step 1.9 s | none | **writer: 0 failures**, worst 953 ms; reader worst 300 ms |

**The in-place shrink.** After the in-place `VACUUM`, a `wal_checkpoint(TRUNCATE)` returned at once without checkpointing, because the probe's reader was in the way. The file stayed 15.49 GB, with the new content in a 3.6 GB WAL. Alone, the same checkpoint took 5 ms and the file became 3.56 GB. On the real machine every running srv is a reader, so an online `VACUUM` gives nothing back until they all pause at once.

## 3. Options

1. **In-place `VACUUM` online, through the truncate-owner srv.** Rejected:
   - It holds the write lock for 40–70 s. Every other srv's transcript append fails after 5 s, so those lines miss the shared store and reach only the per-channel copy.
   - It needs WAL space equal to the live size.
   - The file shrinks only when no srv is reading.
2. **`VACUUM INTO` a new file, then swap, with no other srv running.** **Chosen, for the one-time reclaim.** It takes 16–31 s, needs only the live size in free disk, never touches the original until the swap, and converts the store to `auto_vacuum=INCREMENTAL` in the same step.
3. **`incremental_vacuum` in small steps.** **Chosen, for afterwards.** It needs `auto_vacuum=INCREMENTAL`, and an existing store only gets that through one full `VACUUM` (option 2 does it). Short write transactions stay well under the busy timeout, so other srvs keep writing.

So: reclaim once, offline, with option 2. After that the store gives space back by itself (§5), and the button is needed only once.

## 4. The one-time reclaim

### 4.1 When it runs

- **At the start of an srv, before it opens the shared store** (`bootstrap/stores.rs:327`), and only when a reclaim was asked for: a marker file `filestore.db.reclaim` beside the store, holding `{requested_at_ms, requested_by}`.
- **And only when no other srv is running.** That means no process named `agentmux-srv*` other than itself, crash monitors excluded. The check uses the same scan `framer::who_blocks` does, but counts **every** srv, not just frames-aware ones, because any open connection makes a swap unsafe. If one is running, the reclaim is skipped and the marker kept, and the Storage view says "Reclaim waits: AgentMux 0.59.36 (dev) is running".
- **Two srvs starting at once.** The reclaimer holds an exclusive lock on `filestore.db.gate` for the whole reclaim. A frames-aware srv takes the same lock shared (blocking, up to 120 s) before it opens the store, so it waits instead of opening mid-swap.
- **Older builds** don't take the lock. Before #5203 they read the store's version stamp and refuse version 3, but they may still open the file. On Windows an open file can't be renamed, so the swap then fails safely and is retried at the next start. macOS and Linux would let the rename succeed under the other process, which keeps writing to the old file. So on those platforms the process scan runs again just before the swap, and the swap is refused if anything appeared.
- **The operator chooses when.** The confirm dialog offers **At next start**, the default: the next upgrade restart makes it free, since an upgrade restarts every agent anyway. It also offers **Restart AgentMux now**, which says that it restarts every agent.

### 4.2 Steps

1. Check free disk is at least the live size × 1.2 + 1 GB, where live size = (`page_count` − `freelist_count`) × `page_size`. If not, refuse and say how much is needed.
2. Open the store alone. Run `PRAGMA wal_checkpoint(TRUNCATE)`; it must report `busy=0`.
3. `PRAGMA auto_vacuum=INCREMENTAL; VACUUM INTO 'filestore.db.reclaim-new'`.
4. Verify the new file:
   - `quick_check` = ok;
   - `user_version` the same;
   - `auto_vacuum` = 2;
   - for `db_wave_file`, `db_file_data` and `db_file_frame`, the same row count and the same `sum(length(data))` (and `sum(size)` for `db_wave_file`);
   - the same `sqlite_master` (tables, indexes, triggers).
5. Close every connection. Rename `filestore.db` to `filestore.db.pre-reclaim`, and its `-wal` and `-shm`, which are empty after step 2. Then rename `filestore.db.reclaim-new` to `filestore.db`.
6. Open the store normally and read one file. On success:
   - delete `filestore.db.pre-reclaim*` and the marker;
   - log `history store: reclaimed N GB in T s`;
   - record `{done_at_ms, before_bytes, after_bytes}` in `filestore.db.reclaim-last`.
7. **Any failure before step 5:** delete the new file, and write `last_error` into the marker. It is retried at the next start, at most once a day. After two failures the marker is cleared and the Storage view shows the error.

### 4.3 Crash safety and rollback

The original is never written. Every crash leaves one of three states, which the next srv start resolves before opening the store:

| Found at start | Meaning | Action |
|---|---|---|
| `filestore.db` + `filestore.db.reclaim-new` | crashed in steps 2–4 | delete the new file; the marker retries |
| no `filestore.db`, `filestore.db.pre-reclaim` present | crashed between the two renames | rename `.pre-reclaim` back (roll back), delete the new file |
| `filestore.db` + `filestore.db.pre-reclaim` | crashed after the swap, before step 6 | open the new store; if it opens and reads, delete `.pre-reclaim*`, otherwise swap back |

Rollback after success isn't needed. The new file is the same store, with the same content and version stamp, in a smaller file. `auto_vacuum=INCREMENTAL` is a file-format feature every SQLite version since 3.4 reads, and builds that can't read version 3 already refuse the store.

Nothing in the filestore relies on SQLite rowids (checked: no `rowid` in `backend/storage/filestore/` or `backend/transcript/`), so the renumbering a `VACUUM` may do is harmless. Triggers don't fire during a `VACUUM`, so `rev` and the line counters don't change.

### 4.4 Time at start

The build's reclaim, run on a fresh snapshot of this machine's store (2026-10-23), took 41.6 s from request to swapped-in store, checks included: 15.49 GB → 3.59 GB, `auto_vacuum=2`, 0 free pages.

That is longer than the launcher's 30 s wait for `AGENTMUXSRV-ESTART` (`crates/launcher/src/srv_spawner.rs`). So, only when a reclaim is requested, srv first prints `AGENTMUXSRV-MIGRATING reclaiming-history:1`, which the launcher and the CEF host's sidecar already read as "extend the deadline to 30 minutes", the same as a migration. It then runs the reclaim as a splash step, "Giving back history disk space" (`boot_timing::step`). A start with no request prints nothing and checks only that the marker file is there.

## 5. Afterwards: incremental reclaim

Once the store is `auto_vacuum=INCREMENTAL`, the srv that holds the truncate lock (`filestore/truncate_owner.rs`) gives space back as it is freed:
- **When:** after each framer pass, and after a zone or file delete.
- **Condition:** `freelist_count` is at least 2,048 pages (8 MiB).
- **How:** `PRAGMA incremental_vacuum(1024)` (4 MiB), each step its own write transaction, with a 20 ms pause between steps, up to 64 MiB per pass (the framer's budget).
- **Effect:** the file shrinks at the owner's next `TRUNCATE` checkpoint (every 30 min; `journal_size_limit` 64 MiB).

At 2,048 pages a step the worst was 1.9 s with no writer failure (§2). 1,024 pages halves that, leaving the 5 s busy timeout a wide margin.

The framer frees about 50–56 MiB of pages per full pass (64 MiB in, 8–17 MiB out), so the store stays within about one pass of its live size. A store still at `auto_vacuum=NONE` skips this.

## 6. What the operator sees

**`history.usage`** (`server/app_api/history_usage.rs`) gains, in its `compression` block:
- `free_bytes`: `freelist_count × page_size`, two cheap pragmas;
- `auto_vacuum`;
- `reclaim`: `{pending, waiting_for, last_error, last_done_ms, last_before_bytes, last_after_bytes}`.

**Warden ▸ History ▸ Storage** (`frontend/app/view/warden-history/warden-history-storage.tsx`):

| | Before (today) | Before the reclaim (step 1) | After |
|---|---|---|---|
| Pane history (shared) | 15.5 GB | 15.5 GB · "11.9 GB of this is free space AgentMux can give back" + **Reclaim 11.9 GB…** | 3.6 GB · "Gave back 11.9 GB on Oct 24" |
| Compression line | "… compressed …" (#5331) | unchanged | unchanged |
| Status bar history total | includes the 11.9 GB | the same (the number is true: it's on disk) | about 11.9 GB lower |

- The button appears when `free_bytes` is at least 1 GB **and** at least 20 % of the file, `auto_vacuum` is 0, and the framer is in steady state. Steady state means its last pass framed at most 4 frames, so there's no point reclaiming while it is still compressing.
- After the conversion the button doesn't come back unless `free_bytes` stays over those limits for a day. That would mean the incremental step is failing, which it reports.
- While waiting, the row says why ("Reclaim waits: …"), with a Cancel.

## 7. The per-channel copies

**Nothing to reclaim by vacuum.** All 37 files have 0 free pages: they aren't framed, and history isn't deleted from them.

Their 8.19 GB is mostly copies of older versions. Each upgrade carries the previous version's copy forward (`versions/<v>/data/db/`), so a channel keeps one per version it has run; the newest one here is 0.62 GB. Removing older versions' copies is **deletion**, not reclaim, and the per-channel copy is the recovery source (damage report). So it is not part of this spec. If the operator wants it, a separate spec can list them with sizes and keep the current and previous version of each channel. The Storage row "Pane history copies (per version)" is unchanged.

**Also on disk, outside these numbers, and needing no code:** the repair's leftovers from 2026-10-22, beside the store (`filestore.db.damaged-2026-10-22T03-33-12` and its WAL/SHM, 15.5 GB), and the repair's work copy (about 15 GB). Task #104 waits on the operator's go-ahead to delete them. Together that's about 30 GB, which is more than this whole spec gives back.

## 8. Tests

**Unit** (`filestore`, on temp stores with part rows, frames and triggers):
- the marker's lifecycle and the once-a-day retry;
- the refusal when another srv runs, when disk is short, and when step 2 reports busy;
- verification mismatch → the new file deleted, the original untouched;
- each crash state in §4.3 → the expected file set and an openable store;
- after a reclaim, every file reads byte-identical to before (read all, compare hashes), `auto_vacuum=2`, `rev` and the line counters unchanged;
- incremental: deleting a zone, then one owner pass, shrinks the file after the checkpoint, and a concurrent writer with a 5 s busy timeout never fails.

**Manual, in a task dev build** with `AGENTMUX_HOME_OVERRIDE` pointing at a scratch home holding a snapshot of the real store (as in §2):
- click the button, restart, and see the Storage numbers before and after;
- kill the srv during step 3 and during step 5, restart, and check the store.

## 9. Rollout

1. **PR 1:** `free_bytes`, `auto_vacuum` and `reclaim` in `history.usage`, and the Storage view's line (no action yet). Small; it shows the operator the 11.9 GB at once.
2. **PR 2:** the reclaim at start (§4), the button and its dialog, crash recovery, and the incremental step (§5). It ships this spec.
3. The operator clicks it once, at a convenient restart or the next upgrade.

Built by Loap-3 for Agent5 (history store), task #1852 (filed earlier as #1107).

## 10. Decisions (operator, 2026-10-23)

1. **The reclaim runs at the next start by default**, with "Restart AgentMux now" as the option in the dialog.
2. **The repair's work copy is deleted** (about 15 GB, by AgentY). The damaged original beside the store (`filestore.db.damaged-2026-10-22T03-33-12*`, 15.5 GB) is kept until the first reclaim has succeeded, then deleted. The step-2 PR's §4.2 step 6 doesn't delete it: the operator or the build's owner does, by hand, once the reclaimed store is confirmed.
3. **Removing older versions' per-channel copies** gets a separate spec, later.

## 11. Where the build differs

- **No "Restart AgentMux now" button.** AgentMux has no action that restarts the app from the window, so the confirmation only says "Reclaim at next start". The row explains that it runs after an update, or when AgentMux is quit and reopened. A restart button can come once an app restart action exists.
- **The incremental step runs at the top of the store's minute loop** (`bootstrap/shutdown.rs`), before that minute's framer pass, so it gives back what the previous pass freed. It doesn't run in the same breath as the pass.
- **`steady` is this srv's own view** (`framer::steady`): its framer is off, or its last pass was done or framed at most 4 frames.
- **Disk short** keeps the request waiting, retried at each start, rather than dropping it. The row says how much disk is needed.
