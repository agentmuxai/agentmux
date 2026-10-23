# Offline recovery of the shared transcript store

A development script, not shipped with the app. It works on a machine where
**every AgentMux instance is stopped**, on the shared transcript store
(`~/.agentmux/shared/agents/transcripts/filestore.db`, plus its `-wal` and `-shm`).
Background: `docs/analysis/RCA_UPGRADE_0_59_30_TO_0_59_34_ISSUES_2026_10_19.md`
section 6.

**The original store is never modified.** It is only read, to copy it. The script
compares the size and modification time of the three files before and after and
says so in the report. Everything else happens on copies in `--work`. Putting the
result in place is a manual step (see "Putting the result in place").

## What it does

1. Refuses to start if AgentMux processes are running or the process list can't be read (`--allow-running` overrides),
   if `--work` is inside the store's folder, if there is not about twice the
   store's size free under `--work`, or if the `sqlite3` shell is older than 3.51.3.
2. Copies the database, `-wal` and `-shm` into `--work`.
3. `PRAGMA integrity_check(1000)` on the copy.
4. `sqlite3 copy.db .recover | sqlite3 recovered.db`, then an integrity check of the
   result. The `user_version` of the original is kept.
5. Looks for damaged files in the recovered database. A file is damaged when its
   64 KB parts are not 0..N without a gap, do not add up to the recorded size, have
   no file row (or a file row has no parts), or, for transcripts, contain NUL bytes
   (a lost page read back as zeros; `--skip-content-scan` skips that read). Parts may be
   compressed frames (see "Compressed frames" below): a frame's parts count as parts, and
   a frame that doesn't decode or doesn't match its hash is damage (`bad-frame`, found by
   the content scan).
6. Plans what to do with each damaged file:
   - `agent:<id>:current` / `output`: **rebuild** from the richest whole per-channel
     copy, the same source `crates/srv/src/backend/transcript_backfill.rs` uses (every
     `channels/*/versions/*/data` and `dev/**/data` store; the blocks whose meta
     `agentId` or legacy `agent:id` is the agent). A copy with its own gap, the wrong size or NUL bytes is not used.
     No copy: **no-source**, left alone. A copy shorter than the zone: **source-shorter**,
     left alone unless `--allow-shorter` (the shared zone also holds conversation from
     other channels and builds, so a short copy can lose lines).
   - `output.tsidx`, `output.idx`, `output.state.json`: **drop-derived** (the app
     rebuilds them).
   - anything else (archives, other zones): **manual**, listed, not touched.
7. With `--apply` only: carries the plan out in `recovered.db` (one transaction per
   zone: `output` replaced by 64 KB parts, its sidecars dropped, a fresh view overlay
   written as `seed_global_zone` does, line counters left NULL so the app recounts),
   then runs an integrity check on the result.
8. Writes `report.md` and `report.json` in `--work`.

## Use

Needs Node 24 and a `sqlite3` shell 3.51.3 or newer (the "sqlite-tools" zip at
https://sqlite.org/download.html; unpack it anywhere and pass `--sqlite3`).

```
# 1. Dry run: copy, check, recover, find the damage, plan. Nothing is rebuilt.
node scripts/recover-shared-transcripts/recover.mjs --work D:/recovery --sqlite3 /path/to/sqlite3

# 2. Read D:/recovery/report.md, then rebuild in the recovered file (no second recovery):
node scripts/recover-shared-transcripts/recover.mjs --work D:/recovery --sqlite3 /path/to/sqlite3 \
    --recovered D:/recovery/recovered.db --apply
```

Options: `--source <filestore.db>`, `--home <~/.agentmux>` (where the per-channel stores
are; default derived from `--source`), `--skip-recover` (dry run on the copy itself, for
a store that still opens), `--allow-shorter`, `--skip-content-scan`, `--allow-running`.
Exit codes: 0 done, 1 error or the originals changed, 2 refused.

Time and space: for a 15 GB store expect about 31 GB of free space and a long
`.recover` (it streams the whole database through the shell).

## Putting the result in place

With every instance still stopped: move the old `filestore.db`, `filestore.db-wal` and
`filestore.db-shm` aside and keep them (the WAL may hold newer page images), copy
`recovered.db` in as `filestore.db` with no `-wal` or `-shm`, and start one instance
first. Zones the report lists as `no-source`, `source-shorter` or `manual` still need a
decision.

## Compressed frames

A store may hold compressed frames (`db_file_frame`, `docs/specs/SPEC_HISTORY_STORE_COMPRESSION_AND_COMPACTION_2026_10_21.md`):
up to 64 consecutive 64 KB parts of a file in one row, compressed with zstd (codec 1),
or with the streaming deltas stored once per block and then compressed (codec 2). A part
with a row has no frame. `lib/frames.mjs` is the tool's own decoder for both codecs,
written from `frames.rs` and `deltas.rs` and checked against every frame's SHA-256. Node's
built-in zstd does the decompression, so the tool still needs no packages. Rebuilding or
dropping a file deletes its frames with its rows, as the app does.

## Limits

- It cannot see content that is wrong but well formed (an older version of a page).
  Lengths, gaps and NUL bytes are what it checks.
- It does not touch the per-channel stores or objects databases (opened read-only).
- The recovered database is plain `.recover` output: SQLite re-creates the schema,
  triggers and indexes from the damaged file, and srv's next open re-applies its own
  migrations.

## Tests

`npx vitest run scripts/recover-shared-transcripts/recover.test.mjs` builds a small
store with injected damage (a missing part, a lost page, a zero-filled hole, zones with
no copy or a short one) and runs the dry run and `--apply` on it. The suite needs a
`sqlite3` shell 3.51.3 or newer (`$SQLITE3` or on `PATH`) and is skipped without one.

`frames.test.mjs` decodes `testdata/frames.json`, frames of both codecs written by the
Rust encoder, and runs the damage scan and a rebuild on a store holding frames. The Rust
side decodes the same file (`tests_frame_vector.rs` in the filestore), so the two decoders
can't drift apart. After a change to the frame format, write the file again with
`AGENTMUX_WRITE_FRAME_VECTOR=1 cargo test -p agentmux-srv --bins -- tests_vector`.
