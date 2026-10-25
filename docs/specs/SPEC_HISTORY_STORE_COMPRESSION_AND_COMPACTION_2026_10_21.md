# Spec: the shared transcript store gets smaller: compressed frames, and the streaming deltas cleaned out

**Date:** 2026-10-21
**Author:** Agent5@narko
**Type:** Spec (a storage change; what a reader sees does not change)
**Status:** active. The operator approved both parts on 2026-10-22 ("lets get the compaction and also cleaning out the streaming deltas"). Merged: PRs 1 and 2 of §9 (the frame table, both codecs, every read and write path, the store accepting version 3) in #5324; PR 3 (the framer, `agent:transcriptframes`, and its checks) in #5328; PR 4 (the recovery tool's decoders) in #5330. PR 5 (the Storage numbers) is #5331. **On by default since the operator's call of 2026-10-25** (it had run by hand on the operator's machine since 2026-10-23); `agent:transcriptframes` stays as the opt-out (`false` stops new frames). Remaining: `history.compact` and `decompress` (§5.3, §5.4) wait until dogfooding shows they are needed. Task #49.
**Builds on:** `REPORT_AGENT_HISTORY_STORE_DEDUP_COMPRESSION_2026_10_17.md` (AgentY: §4.3 compression, §7.3 "shrink the pane store"), `SPEC_AGENT_TRANSCRIPT_SINGLE_COPY_2026_10_19.md` (Maricon: one copy of each transcript), `docs/analysis/RCA_UPGRADE_0_59_30_TO_0_59_34_ISSUES_2026_10_19.md` §6 (the damaged parts).
**Platform:** all.

## 1. What this decides, in one paragraph

The shared transcript store (`shared/agents/transcripts/filestore.db`) is 15.4 GB on this machine and is kept forever. Replayed over a copy of the whole store, **compressing it in 1 MiB frames takes the 14.0 GB of transcript it can frame to 2.39 GB (5.8x)**, and **cleaning out the streaming deltas on top of that saves another 11.5%**, to 2.12 GB (6.6x) (§2.1). Both are built, in that order, as one storage format: a **frame** holds 1 MiB of a transcript compressed, and a frame may also store its streaming delta lines as just each line's id and length, with each reply block's text once, and rebuild them byte for byte. Every byte offset and line number stays what it is, so no reader changes. Nothing runs until the operator turns it on, and it never runs while an AgentMux build that can't read frames is running on the machine.

## 2. Measurements (2026-10-21, narko)

Sample: the `output` of two agents (Naki #2 and QTimer), parts 20 to 400 of each, taken from an online-backup copy of the store: 49.76 MB, 126,866 lines. Mid-conversation, one machine, two agents; a wider sample should be taken before building (§9). Scripts: `tmp/sample*.py`, `tmp/mk.py`, `tmp/z*.mjs` in the Agent5 workspace (not committed).

**What the bytes are** (as in AgentY's sample, a little more skewed to tool JSON):

| Line kind | Lines | Bytes | Share |
|---|---|---|---|
| `stream_event` `input_json_delta` | 99,437 | 25.68 MB | 51.6% |
| `stream_event` `text_delta` | 9,039 | 2.26 MB | 4.5% |
| other `stream_event` (`message_start`, block start/stop, `message_delta`/`stop`, `thinking_delta`, `signature_delta`) | 9,742 | 4.48 MB | 9.0% |
| `user`, `assistant`, `system`, `result`, `rate_limit_event` | 8,648 | 17.34 MB | 34.8% |

**Which deltas rebuild from the final message** (the assistant line that follows the turn's `message_stop`):

| Delta kind | Rebuilds exactly | Not | Notes |
|---|---|---|---|
| `text_delta` | 292 of 295 blocks (99%); 2.25 of 2.26 MB | 3 blocks | the concatenated text equals the final text |
| `input_json_delta` | 825 of 969 tool blocks, 24.75 of 25.68 MB (96%) | 4 blocks differ in key order (42 KB); about 140 have no matching deltas in the turn (not streamed, or the stream was cut) | the concatenated `partial_json` equals `json.dumps(input)` with Python's default separators, with ASCII escapes for 749 blocks and raw UTF-8 for 76 (10.5 MB, the large Write/Edit inputs): a one-bit serializer flag per block, then verify |

**What compression does** (zstd level 3, each frame compressed on its own, no dictionary):

| Frame size | Raw (49.76 MB) | After deltas dropped (24.64 MB, residue counted as 18 random bytes per dropped line) |
|---|---|---|
| 64 KiB (today's part) | 11.34 MB (4.4x) | 9.98 MB (2.5x) |
| 256 KiB | 10.47 MB (4.8x) | 8.95 MB |
| **1 MiB** | **8.89 MB (5.6x)** | **7.59 MB** |
| 4 MiB | 8.48 MB (5.9x) | 6.96 MB |

- Dropping the deltas halves the bytes (49.76 to 24.64 MB) but, after compression, saves only 12% at 64 KiB and 15% at 1 MiB. The delta lines are 83% of the lines and compress to almost nothing except their uuids: 104,875 dropped lines leave 1.68 MB of random uuids that nothing can shrink.
- The residue is what stays: a uuid (16 bytes) and a length per dropped line. Everything else on a delta line (`type`, `index`, `session_id`, `parent_tool_use_id`) is constant within a turn or a fixed template.
- Larger frames win clearly (4.4x to 5.6x from 64 KiB to 1 MiB) because the same prompts, paths and schemas recur across records.

**Estimate for the 15.4 GB pane store** (from the sample; §2.1 has the real numbers): 1 MiB frames give about 2.7 GB; adding delta compaction about 2.3 GB. AgentY's estimate for the pane store was 0.9 GB; that assumed the deltas were the bulk of what was left after compression.

### 2.1 Offline replay over the whole store (2026-10-22)

`replay_a_real_store` (an ignored test in `filestore/tests_deltas.rs`, run in release against `AGENTMUX_FRAMES_REPLAY_DB`) read every 16-part frame of every transcript in a copy of the repaired store, encoded it with both codecs, decoded it and compared every byte.

| | Frames | Bytes | Ratio |
|---|---|---|---|
| Raw | 13,321 (59 transcripts) | 13,968 MB | |
| Codec 1 | 13,321 | 2,394 MB | 5.84x |
| Codec 2 | 13,038 used codec 2, the rest fell back to codec 1 | 2,119 MB | 6.59x, 11.5% smaller than codec 1 |

Every frame decoded to its exact bytes under both codecs. Codec 2 left out 28.2 million delta lines. One frame was skipped because a part in it is missing (a lost part is never framed, §5.1). It ran in about 6 minutes on one core.

## 3. Rules that hold for everything below

1. **Logical addressing does not change.** A file stays a stream of bytes; byte offsets, line numbers, `output.idx`, `output.tsidx`, the agent record's `ranges` and every reader keep working unchanged. A frame is a different storage of the same bytes.
2. **Never edit in place.** New rows are written, decoded, compared with the original, and only then do the old rows go, all in one SQLite transaction. (The 2026-10-20 finding: two agents' newest parts were damaged; see the damage report.)
3. **Only finished data.** Never the part being written, and never anything within the safety margin of the end of the file (§5.2).
4. **No older build may read a store it cannot decode.** The store is stamped with a higher `user_version` (3; the pattern of #5203), so a build without the decoder refuses it and runs from its own per-channel copy. The stamp is written **when the first frame is about to be written**, not when a new build opens the store: stamping on open would push every older instance on the machine onto its per-channel copy the moment one new build starts. So a build that reads every frame codec **accepts** 3 but an ordinary open **stamps** only 2, and never lowers a 3 already there (`GLOBAL_TRANSCRIPT_STORE_FRAMES_VERSION`). That is why codec 2 ships in the same PR as the frame table: a build that read codec 1 only and accepted 3 would read codec 2 frames as damaged. And because a build that opened the store before the stamp keeps its connection, the framer also waits until **every running srv on the machine has said it reads frames** (§5.1).
5. **Not before the store is repaired and has been clean for a while.** A frame is a bigger loss when damaged (§6). The repair was done on 2026-10-22 (the recovery tool, #5206; see the RCA §6). The framer was off by default for the soak, the time between the code landing and the operator turning it on (2026-10-23); it has been on by default since 2026-10-25 (§10).
6. **History is kept forever.** Nothing here deletes history.

## 4. The design: frames

A **frame** holds a run of whole 64 KiB parts of one file, compressed together.

```
db_file_frame (
  zoneid      TEXT    NOT NULL,
  name        TEXT    NOT NULL,
  first_part  INTEGER NOT NULL,   -- logical part index of the first part it covers
  part_count  INTEGER NOT NULL,   -- 1 to 16
  codec       INTEGER NOT NULL,   -- 1 = zstd; 2 = zstd of the frame with its finished deltas left out (§8)
  raw_len     INTEGER NOT NULL,   -- logical bytes (part_count * 65536: frames hold whole parts only)
  check_hash  BLOB    NOT NULL,   -- SHA-256 of the logical bytes (sha2 is already a dependency)
  data        BLOB    NOT NULL,
  PRIMARY KEY (zoneid, name, first_part)
)
```

- A part covered by a frame has **no row** in `db_file_data`. A reader looks for the part row first, then for the frame whose range holds it. **A part row always wins over a frame**: a writer that knows nothing of frames (and every write path below) leaves correct data behind, and the frame only fills the parts that have no row.
- **Scope:** only the transcript text (`output`) of agent zones (`agent:<id>:current` and its archives) in the shared store. The sidecars (`output.idx`, `output.tsidx`, a few per cent of the bytes) and the per-channel stores are never framed.
- **Frame size 1 MiB** (16 parts): the measured knee (§2). Smaller at the end of a file, never more than 4 MiB.
- **Reads:** `FileStore::read_at`, `read_file`, the snapshot reader and the counter's `read_bytes*` ask for a part range; they decode a covering frame, verify `check`, and slice. A small cache keeps the last few decoded frames (a History page read touches one or two). Decoding 1 MiB of zstd is about 1 ms.
- **Writes:** appends touch only the tail part, which is never framed. A write into a framed range (a `replace`, a truncate, a patch) first **unframes**: the frame is decoded into part rows in the same transaction. This is rare (truncates and replaced files) and is the simple, safe path.
- **Raw SQL sites that must learn frames** (every one found by `grep db_file_data`): `filestore/core.rs`, `offset_ops.rs`, `counter.rs` (`read_bytes_covered_rows`, tail updates), `parts.rs` (the damaged-part fallback), `replace.rs` (`patch_range_in`, `truncate_parts_in`, the snapshot reader), `copy.rs` (`copy_files_to_zone`, `copy_prefixes_to_zone`, `move_files_to_zone` copy or rename frame rows too; a prefix copy that ends inside a frame decodes that one frame), `transcript/damage.rs`, `transcript_backfill.rs`, and the offline tool `scripts/recover-shared-transcripts/` (`analyze.mjs`, `rebuild.mjs`, `sources.mjs`, which must decode frames and treat a damaged frame as a gap of its whole range). A test fails the build if a new `db_file_data` query appears outside that list.
- **Counters and `rev`:** the triggers `db_file_data_rev_*` bump a file's `rev` when a part row is deleted or inserted inside the file's size, which would invalidate the line counter (`lines_rev`) and force a recount of a gigabyte. Framing and unframing preserve the bytes, so each does its swap and then **puts `rev` back to the value it read at the start**, in the same transaction (one writer at a time, so nothing else can have moved it). Only the line counter reads `rev`.
- **No dictionary for now.** 1 MiB frames already see the repeats a dictionary would supply; a dictionary can come later as another codec number if a measurement shows it pays.
- **Every write path that removes a file's parts removes its frames too** (`delete_file`, `delete_zone`, `delete_files`, `replace_inner`, `replace_content_in`, `truncate_parts_in`), copies copy frames (`copy_files_to_zone`; `copy_prefixes_to_zone` decodes the one frame the prefix ends inside), and moves move them (`move_files_to_zone`). A write into a framed part (`write_at`, `patch_range_in`) unframes that frame first, in the same transaction.

## 5. How a file gets framed

### 5.1 The framer

A background worker in every srv whose own `agent:transcriptframes` is on. Settings are per channel (a dev build keeps its own), so the setting means "this instance compresses", and no single srv's setting can speak for the machine. Each frame is written in one transaction that checks its parts first (§5.1 steps 2 to 5), so two srvs framing at once never conflict: the later one finds the parts framed and skips them. Every process reads frames. It runs on a blocking thread, at most 64 frames (about 64 MiB) per pass: a pass each minute, or every five seconds while work remains (`transcript/framer.rs`, driven by the truncate loop in `bootstrap/shutdown.rs`).

**Who else is running.** Every frames-aware srv writes a small file, `transcripts/readers/<pid>.json` (its pid, version and the frame format it reads), when it opens the store. The framer prunes the files of processes that have exited; the process start time in each file keeps a reused pid from passing for the srv that wrote it. Before each pass the framer lists the running processes named `agentmux-srv*` (sysinfo, already a dependency; crash monitors excluded by their `--crash-monitor` argument) and frames nothing while one of them has no such file. An older build, or another agent's dev build from an older branch, therefore holds framing off until it exits. The Storage view says so ("compression waits: AgentMux 0.59.36 is still running").

For one file it repeats:

1. Choose up to 16 consecutive part rows below the safety margin, none of them the file's last part.
2. Read their bytes. All 16 must exist and be full (65,536 bytes). If any row is missing, short or fails to read (a damaged part), stop: **a damaged or lost part is never framed**, and the file is skipped and reported.
3. Encode (codec 2 when the frame has deltas to leave out, otherwise codec 1) and write the `db_file_frame` row. Before its first pass the framer stamps `user_version` 3, then checks the running processes again: a build that opened the store before the stamp is a running process the check sees, and one that starts after it refuses the store.
4. In the same transaction: read the frame back, decode it, compare with the bytes read in step 2, **every byte, every frame**, and only then delete the part rows and put `rev` back. Any mismatch rolls back and records an incident.
5. `COMMIT`. One frame per transaction, so a crash leaves either the old parts or the frame, never both missing.

### 5.2 The safety margin

Frames cover parts that are at least 8 MiB behind the end of the file **and** whose last line was received at least 24 hours ago (from the file's receive-time sidecar; a file without one uses the byte margin alone). A transcript being written is untouched. Both are constants for now (`MARGIN_BYTES`, `MARGIN_MS`).

### 5.3 Setting and rollout

`agent:transcriptframes` (on by default since 2026-10-25; set `false` to opt out). Off: no new frames; existing frames still decode. `agent:transcriptdeltas` (on by default once frames are on) chooses codec 2 where it applies; off, every frame is codec 1. A `history.compact` command, one pass on demand (a "Compress now" in Warden > History > Storage), is not built: the framer already runs a pass each minute.

### 5.4 Rollback

Turning `agent:transcriptframes` off stops new frames; the frames already written keep reading, so no rollback is needed to stop. Going back to a build older than the frames needs every frame undone: `decompress` would unframe every frame of a store, a zone or a file back to part rows, in the same one-transaction-per-frame way, and lower the version stamp to 2 once no frame is left (`FileStore::unframe_file` is the building block). Not built yet; until it is, such a build runs from its own per-channel copy (rule 4). The per-channel copies and the provider's session files stay as they are, so even a lost frame has two independent sources (§6).

## 6. Damage and recovery

- A damaged 64 KiB part today loses at most 64 KiB of a conversation. A damaged frame loses up to 1 MiB (16 parts). That is the price of the larger window, and it is why: frames are verified when written and by a periodic scrub (one frame in 1,000 per day, decoded and checked against `check`); a failing frame is an incident like a corrupt zone (#5204); `recover.mjs` rebuilds the zone from the richest per-channel copy, as it does for a damaged part.
- Frame size is capped at 1 MiB for the default so the worst case stays small; 4 MiB is a setting for cold data.
- The recovery tool and the History damage handling treat a damaged frame as a gap of its whole range. `parts.rs` (the blank-part fallback) gets the same rule: a frame whose decode or check fails reads as blanks of its logical length.
- **Do not turn this on before the store is repaired and soaked (Rule 5).** (Met: repaired 2026-10-22, on by hand from 2026-10-23, on by default from 2026-10-25.) The repair rebuilt the database with `.recover`, which also rebuilt the index the damage report found two unexplained entries in; the rebuilt store passed its integrity check.

## 7. What each reader sees

| Reader | Effect |
|---|---|
| History reader, `blockfile:read_range`, `output.idx` | none: they read logical lines. `output.idx` is rebuilt only if its own rows are rewritten |
| Receive-time sidecar (`output.tsidx`) | none; it is just another file and is framed the same way |
| Line counter, `rev` | unchanged by framing (§4); a recount only after an unframe that changes bytes |
| Agent record (`history/record.rs`), session ranges | none: they are byte ranges of the logical file |
| Fork copy (`copy_older_session_history`, #5219) | a prefix ending inside a frame decodes that frame; whole-file copies copy frame rows |
| Archive (`move_files_to_zone`) | frame rows move with the part rows |
| Offline recovery tool | learns the frame table and both codecs (`recover.mjs`, PR 4, #5330, which lands before the setting is turned on); a test vector written by the Rust encoder is decoded by the tool's own decoder |
| An older build | refuses the store (version gate) and runs from its own copy |

**Measuring it:** `history.usage` answers a `compression` block for the shared store: frames, the bytes they hold and the bytes they take (one sum over `db_file_frame`), whether the setting is on, and what the framer waits for. Warden > History > Storage shows it as one line under the stores ("Pane history compressed: 14.0 GB of transcript in 2.1 GB (6.6×), 13,321 frames."). A scrub count comes with the scrub.

## 8. Cleaning out the streaming deltas (codec 2)

Decided 2026-10-22: built. On the sample it takes a 1 MiB frame from 8.89 MB to **7.47 MB** (16% smaller than compression alone); over the whole store, 11.5% (§2.1).

**What a delta line is.** A reply streams as one line per few characters, each line mostly the same envelope around a little text:
`{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}},"session_id":"…","parent_tool_use_id":null,"uuid":"…"}`
Tool input streams the same way, as `input_json_delta` / `partial_json`.

**What codec 2 stores instead.** For each such line, its uuid (16 bytes) and the length of its text. For each content block (one session, one `parent_tool_use_id`, one `index`, one kind), the text of all its lines **once**, end to end. Everything else stays as it was: every other line, and any delta line that isn't exactly the template (an extra field such as thinking deltas carry, an uppercase uuid, a space, another key order).

**Why not rebuild the text from the reply's final `assistant` line**, as an earlier draft proposed: the final line isn't a faithful copy of the stream. The CLI reorders tool-input keys (`"replace_all":false` first) and adds defaults the model never sent, so a rebuild from it would miss many blocks or need fragile transforms. Storing the block's text once is simpler, needs no parsing into maps and no derived text, works for interrupted replies too (nothing depends on the final line), and compresses as well or better: zstd finds the copy in the final line on its own. Measured on the same sample: 7.47 MB per 1 MiB frame against 7.59 MB for the rebuild.

**How it is checked.** A line is taken only if rendering it back from its stored values gives exactly its bytes. The encoder then decodes the whole payload and compares it with the frame's bytes; any difference and the frame is stored with codec 1. The frame's SHA-256 is checked again on every decode. So a change in Anthropic's format shows up as fewer lines taken, never as wrong bytes.

**The payload** (then zstd): `AMD2`, a string table (session ids, `parent_tool_use_id` values as they appear), the blocks (session, parent, index, kind, text), and the records in order: raw bytes, or a delta line (block, text length, uuid). A frame's first and last chunks may be partial lines; they are raw.

## 9. Plan

| PR | What | Changes behaviour? |
|---|---|---|
| 1 + 2 | `db_file_frame`, codec 1, every read and write path of §4; codec 2 (§8): the encoder, the decoder, the verification, tests on synthetic transcripts (real transcripts are private and never go in the repository); the store accepting `user_version` 3 without writing it. One PR (#5324), so the first build that accepts 3 reads every codec | no: nothing creates a frame yet |
| 3 | The framer (§5), its settings (on by default since 2026-10-25), the readers file and the running-instance check, the version stamp | from 2026-10-25 for everyone, unless `agent:transcriptframes` is `false` |
| 4 | The recovery tool's decoders for both codecs, with a test vector the Rust encoder writes (#5330). Lands before the operator turns frames on | no |
| 5 | The Storage numbers (§7) | no |
| — | Offline run on a copy of the real store: every frame encoded and decoded under both codecs, byte for byte; the real ratio and codec 2 rate. Done 2026-10-22, §2.1 | no (a copy) |
| — | The operator turns `agent:transcriptframes` on (2026-10-23, by hand); on by default from 2026-10-25 | yes |

**Tests:** (1) for random files and random frame boundaries, `read_at` over every offset and length returns the original bytes, framed or not, with appends, truncates, replaces, copies and moves interleaved; (2) the framing transaction rolled back at each step leaves the old parts; (3) a decoder fuzz (truncated, flipped, oversized frames: an error, never a panic, never wrong bytes); (4) the offline replay above; (5) a guard that fails the build if a new `db_file_data` query appears outside the known sites; (6) for codec 2, the byte-exact rebuild over synthetic transcripts with the awkward cases (escapes, non-ASCII, key order, numbers, a reply split across frames, an interrupted reply).

## 10. Decisions

Taken on 2026-10-22: build both (compression and the delta clean-out). Defaults, changeable as settings: **1 MiB frames**; a margin of **8 MiB and 24 hours**; **no dictionary**; the framer **off** until the operator turns it on, which is also the soak after the repair. **Changed on 2026-10-25 (operator: "we want history compaction on by default"): the framer is on unless `agent:transcriptframes` is `false`.** The safety gate is unchanged: nothing is framed while a build that can't read frames runs (§5.1).

## 11. Not covered

The provider's own session files (5.8 GB; AgentY §7.3: store repeated payloads once), the per-channel copies (Maricon's single-copy work), and the content-addressed store with its tamper-evident log (task #51), which would replace this layout rather than sit next to it.
