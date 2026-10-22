# Spec: the shared transcript store gets smaller: compressed frames, and the streaming deltas cleaned out

**Date:** 2026-10-21
**Author:** Agent5@narko
**Type:** Spec (a storage change; what a reader sees does not change)
**Status:** active. The operator approved both parts on 2026-10-22 ("lets get the compaction and also cleaning out the streaming deltas"). PR 1 of §9 (the frame table, codec 1, every read and write path; nothing creates a frame yet) is the first pull request; remaining: codec 2, the framer with its settings and checks, the offline run on a copy of the store, and turning it on. Task #49.
**Builds on:** `REPORT_AGENT_HISTORY_STORE_DEDUP_COMPRESSION_2026_10_17.md` (AgentY: §4.3 compression, §7.3 "shrink the pane store"), `SPEC_AGENT_TRANSCRIPT_SINGLE_COPY_2026_10_19.md` (Maricon: one copy of each transcript), `docs/analysis/RCA_UPGRADE_0_59_30_TO_0_59_34_ISSUES_2026_10_19.md` §6 (the damaged parts).
**Platform:** all.

## 1. What this decides, in one paragraph

The shared transcript store (`shared/agents/transcripts/filestore.db`) is 15.4 GB on this machine and is kept forever. Measured on real transcripts, **compressing it in 1 MiB frames takes it to about 2.7 GB (5.6x)**, and **cleaning out the streaming deltas on top of that saves another 15 to 18%** (about 0.4 GB, to about 2.3 GB). Both are built, in that order, as one storage format: a **frame** holds 1 MiB of a transcript compressed, and a frame may also leave out the delta lines of the finished replies inside it, keeping just enough (each line's id and length) to rebuild them byte for byte. Every byte offset and line number stays what it is, so no reader changes. Nothing runs until the operator turns it on, and it never runs while an AgentMux build that can't read frames is running on the machine.

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

**Estimate for the 15.4 GB pane store** (assuming this sample is typical, which §9 checks): 1 MiB frames give about 2.7 GB; adding delta compaction about 2.3 GB. AgentY's estimate for the pane store was 0.9 GB; that assumed the deltas were the bulk of what was left after compression.

## 3. Rules that hold for everything below

1. **Logical addressing does not change.** A file stays a stream of bytes; byte offsets, line numbers, `output.idx`, `output.tsidx`, the agent record's `ranges` and every reader keep working unchanged. A frame is a different storage of the same bytes.
2. **Never edit in place.** New rows are written, decoded, compared with the original, and only then do the old rows go, all in one SQLite transaction. (The 2026-10-20 finding: two agents' newest parts were damaged; see the damage report.)
3. **Only finished data.** Never the part being written, and never anything within the safety margin of the end of the file (§5.2).
4. **No older build may read a store it cannot decode.** The store is stamped with a higher `user_version` (3; the pattern of #5203), so a build without the decoder refuses it and runs from its own per-channel copy. The stamp is written **when the first frame is about to be written**, not when a new build opens the store: stamping on open would push every older instance on the machine onto its per-channel copy the moment one new build starts. And because a build that opened the store before the stamp keeps its connection, the framer also waits until **every running srv on the machine has said it reads frames** (§5.1).
5. **Not before the store is repaired and has been clean for a while.** A frame is a bigger loss when damaged (§6). The repair was done on 2026-10-22 (the recovery tool, #5206; see the RCA §6). The framer stays off by default, so the soak is the time between the code landing and the operator turning it on.
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

A background worker in the srv that owns the store's truncate lock (`filestore.db.owner`, #5201: one srv per machine), so two processes never frame the same file, and every process still reads frames. It runs at low priority, a bounded amount per pass (one file, a few frames), and not while the machine is busy (the load the sampler already reads).

**Who else is running.** Every frames-aware srv writes a small file, `transcripts/readers/<pid>.json` (its pid, version and the frame format it reads), when it opens the store, and removes it on exit. Before each pass the framer lists the running processes named `agentmux-srv*` (sysinfo, already a dependency; crash monitors excluded by their `--crash-monitor` argument) and frames nothing while one of them has no such file. An older build, or another agent's dev build from an older branch, therefore holds framing off until it exits. The Storage view says so ("compression waits: AgentMux 0.59.36 is still running").

For one file it repeats:

1. Choose up to 16 consecutive part rows below the safety margin, none of them the file's last part.
2. Read their bytes. All 16 must exist and be full (65,536 bytes). If any row is missing, short or fails to read (a damaged part), stop: **a damaged or lost part is never framed**, and the file is skipped and reported.
3. Encode (codec 2 when the frame has finished deltas to leave out, otherwise codec 1), write the `db_file_frame` row. If this is the store's first frame, stamp `user_version` 3 first, in the same transaction.
4. In the same transaction: read the frame back, decode it, compare with the bytes read in step 2, **every byte, every frame**, and only then delete the part rows and put `rev` back. Any mismatch rolls back and records an incident.
5. `COMMIT`. One frame per transaction, so a crash leaves either the old parts or the frame, never both missing.

### 5.2 The safety margin

Frames cover parts that are at least 8 MiB behind the end of the file **and** whose last line was received at least 24 hours ago (from the file's receive-time sidecar; a file without one uses the byte margin alone). A transcript being written is untouched. Both numbers are settings.

### 5.3 Setting and rollout

`agent:transcriptframes` (off by default). Off: no new frames; existing frames still decode. `agent:transcriptdeltas` (on by default once frames are on) chooses codec 2 where it applies; off, every frame is codec 1. A `history.compact` command runs one pass on demand (used by tests and by the operator's "Compress now" in Warden > History > Storage).

### 5.4 Rollback

`decompress` (a CLI and a command) unframes every frame of a store, a zone or a file back to part rows, in the same one-transaction-per-frame way, and a setting reverts the store version stamp. The per-channel copies and the provider's session files stay as they are, so even a lost frame has two independent sources (§6).

## 6. Damage and recovery

- A damaged 64 KiB part today loses at most 64 KiB of a conversation. A damaged frame loses up to 1 MiB (16 parts). That is the price of the larger window, and it is why: frames are verified when written and by a periodic scrub (one frame in 1,000 per day, decoded and checked against `check`); a failing frame is an incident like a corrupt zone (#5204); `recover.mjs` rebuilds the zone from the richest per-channel copy, as it does for a damaged part.
- Frame size is capped at 1 MiB for the default so the worst case stays small; 4 MiB is a setting for cold data.
- The recovery tool and the History damage handling treat a damaged frame as a gap of its whole range. `parts.rs` (the blank-part fallback) gets the same rule: a frame whose decode or check fails reads as blanks of its logical length.
- **Do not turn this on before the store is repaired and soaked (Rule 5).** The repair rebuilt the database with `.recover`, which also rebuilt the index the damage report found two unexplained entries in; the rebuilt store passed its integrity check.

## 7. What each reader sees

| Reader | Effect |
|---|---|
| History reader, `blockfile:read_range`, `output.idx` | none: they read logical lines. `output.idx` is rebuilt only if its own rows are rewritten |
| Receive-time sidecar (`output.tsidx`) | none; it is just another file and is framed the same way |
| Line counter, `rev` | unchanged by framing (§4); a recount only after an unframe that changes bytes |
| Agent record (`history/record.rs`), session ranges | none: they are byte ranges of the logical file |
| Fork copy (`copy_older_session_history`, #5219) | a prefix ending inside a frame decodes that frame; whole-file copies copy frame rows |
| Archive (`move_files_to_zone`) | frame rows move with the part rows |
| Offline recovery tool | learns the frame table and both codecs (`recover.mjs`, in the framer's PR, before any frame can exist); a test vector written by the Rust encoder is decoded by the tool's own decoder |
| An older build | refuses the store (version gate) and runs from its own copy |

**Measuring it:** Warden > History > Storage gets, per store, the **stored size** and the **logical size** (what the files hold decoded), the ratio, bytes framed and not yet framed, and a count of frames checked by the scrub with any failures. `history.usage` already reads file sizes; it adds `SUM(length(data))` per table.

## 8. Cleaning out the streaming deltas (codec 2)

Decided 2026-10-22: built. Expected extra saving on the sample: 15 to 18% (§2), about 0.4 GB of 2.7 GB.

**It lives inside a frame.** A frame's bytes are split into lines (the partial line it starts and ends with stays as it is). Everything below happens within one frame, so decoding a frame never needs another frame, and every offset and line number stays valid (rule 1). A reply whose deltas and final line fall in different frames keeps its deltas.

**What is left out:** the `stream_event` lines of type `content_block_delta` carrying a `text_delta` or an `input_json_delta`, for one content block of one reply, when all of these hold:
- the reply's final `assistant` line (the same `message.id` as its `message_start`) is in the frame, and holds that block;
- **text:** the block's text, JSON-escaped the way the CLI escapes strings, equals the deltas' escaped text strings put end to end;
- **tool input:** the block's `input`, as its raw JSON text in the assistant line, with a space put after each `,` and `:` outside strings (and, where the stream used them, non-ASCII characters written as `\uXXXX`), equals the deltas' `partial_json` put end to end. (The model streams spaced JSON; the CLI stores it compact. On the sample this held for 96% of tool-input bytes.) Nothing is parsed into a map and written again, so key order, number formats and escapes are what the line had;
- every one of the block's delta lines is exactly the template below with its own values.

If any of these fails, **the whole block keeps its deltas**. Thinking and signature deltas, block start and stop, `message_start`, `message_delta` and `message_stop` always stay (about 9% of the bytes). A reply that was interrupted or crashed has no final line, so it keeps everything.

**What is kept per left-out line:** the `index`, the length of its escaped text, its uuid (16 bytes), the line number of the assistant line in the frame and which block, and per block one flag for the `\uXXXX` form. `session_id` and `parent_tool_use_id` are kept once per block.

**The template:**
`{"type":"stream_event","event":{"type":"content_block_delta","index":I,"delta":{"type":"text_delta","text":"T"}},"session_id":"S","parent_tool_use_id":P,"uuid":"U"}` (or `input_json_delta` / `partial_json`), then a newline; `P` is `null` or a quoted id.

**The payload:** a list of line records (a raw line, or a left-out line's kept values), then zstd over the whole list. The decoder writes the raw lines, rebuilds each left-out line from its assistant line, and checks the SHA-256 of the result against the frame's `check_hash`.

**Verification, the operator's rule:** the encoder decodes what it produced and compares it with the original bytes, all of them, before the frame is written (§5.1 step 4 then checks it again from the database). A frame whose rebuild differs anywhere is written as codec 1 instead, and the encoder logs which block failed, so a format change on Anthropic's side shows up as a falling rate of codec 2, never as wrong bytes.

## 9. Plan

| PR | What | Changes behaviour? |
|---|---|---|
| 1 | `db_file_frame`, codec 1, every read and write path of §4, the store accepting `user_version` 3 without writing it, tests | no: nothing creates a frame yet |
| 2 | Codec 2 (§8): the encoder, the decoder, the verification, tests on synthetic transcripts (real transcripts are private and never go in the repository) | no |
| 3 | The framer (§5), its settings (off), the readers file and the running-instance check, the version stamp, `history.compact`, the Storage numbers, the recovery tool's decoders | only once the operator turns it on |
| — | Offline run on the 2026-10-21 copy of the real store: frame everything, then read every line of every transcript and compare with the copy, byte for byte; measure the real ratio and the codec 2 rate | no (a copy) |
| — | The operator turns `agent:transcriptframes` on | yes |

**Tests:** (1) for random files and random frame boundaries, `read_at` over every offset and length returns the original bytes, framed or not, with appends, truncates, replaces, copies and moves interleaved; (2) the framing transaction rolled back at each step leaves the old parts; (3) a decoder fuzz (truncated, flipped, oversized frames: an error, never a panic, never wrong bytes); (4) the offline replay above; (5) a guard that fails the build if a new `db_file_data` query appears outside the known sites; (6) for codec 2, the byte-exact rebuild over synthetic transcripts with the awkward cases (escapes, non-ASCII, key order, numbers, a reply split across frames, an interrupted reply).

## 10. Decisions

Taken on 2026-10-22: build both (compression and the delta clean-out). Defaults, changeable as settings: **1 MiB frames**; a margin of **8 MiB and 24 hours**; **no dictionary**; the framer **off** until the operator turns it on, which is also the soak after the repair.

## 11. Not covered

The provider's own session files (5.8 GB; AgentY §7.3: store repeated payloads once), the per-channel copies (Maricon's single-copy work), and the content-addressed store with its tamper-evident log (task #51), which would replace this layout rather than sit next to it.
