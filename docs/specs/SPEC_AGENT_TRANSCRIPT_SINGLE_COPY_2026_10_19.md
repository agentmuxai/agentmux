# Spec: an agent's transcript is written once, to the agent's own zone

**Date:** 2026-10-19
**Author:** Maricon@charlie
**Type:** Spec (a storage contract change; behaviour for the user is unchanged)
**Status:** active — step A (the readers) landed in #5173; step B (the setting `agent:transcriptsinglecopy`, off by default, and the skip in `append_transcript`) is in PR #PRNUM; remaining: the lab A/B with the setting on (`--heavy-turns`, `--disk-load`), then flipping the default in a one-line change, and C (remove the dead paths) after a soak.
**Issue:** #5141. Measured in `docs/reports/REPORT_FILESTORE_WRITE_VOLUME_2026_10_19.md`.
**Platform:** all.

## 1. The problem

Every line of an agent's output is written to two stores: the block's own store (zone = the block id, files `output`, `output.tsidx`) and the global transcript store (zone `agent:<definitionId>:current`, the same files). `append_transcript` (`backend/transcript/append.rs`) does both, one after the other.

In a local experiment that skipped the block-store write when the global one was written (12 agents, 10 rounds, Linux, same pinned build, interleaved), srv's bytes written fell **50.1%** (14489 → 7231 MB), its bytes to storage 50.1%, **srv CPU 42.6%** (104.7 → 60.1 s), and round time 9.9%. On the Windows machine that stalls (#5141) srv writes 5.3 GB for a 4-agent run; halving it halves the load that stalls the volume.

## 2. Who reads the block's copy (audit, 2026-10-19)

For a live agent block whose `agentId` meta is valid, whose global zone has content and which is not archived, the pane and everything behind `blockfile:read_range`, `blockfile:line_count` and `agent:open_tail` already read the **global** copy (`global_output_source`), and the live cursor pins `g:<zone>` (it prefers it before a pin, `TranscriptCursor.pick`). The block's byte `offset` in an event is used only by terminals. What still reads the block's copy of an agent's `output`:

| reader | needs | disposition |
|---|---|---|
| `ambient/digest.rs` (`read_recent_activity`) and its callers: title recovery, pull-path summaries and next-prompt suggestions, the supervisor's judgement, a worker's last words | last 96 KB | read the global tail (the zone is derivable at every caller from `agentId`) |
| `reactive/activity_watcher.rs:131` | the size, as a changed-since gate | the global size, or the block event it already sees |
| `reactive/progress_watcher.rs:604,638` | incremental bytes after a per-block cursor (todo list, current tool) | reads the agent's zone through `TranscriptSource` for a defined agent (one conversation, one live writer, so its offsets serve as the cursor); the cursor counts bytes of one copy, so a change of source starts over as a first sight (done in A) |
| `server/muxspect_handlers.rs:133,1006` | the last error frame; the last line and the file's modts | the global tail; the modts of a shared zone is not per block, so *last activity* keeps its meaning only if taken from the event time |
| `session_archive.rs` (`archive_session_output`, `restore_session_output`, `read_session_output`) and its callers: `session:archive`, `session:restore`, `session:export`, the sweep, the `/transcript` endpoint | the whole `output` and sidecars | `session:export` and `/transcript` stay on the block's copy until B, because the agent's zone holds the agent's whole record, not one block's session, so reading it changes what an export contains: B decides that scope explicitly; the block-level archive of an agent block becomes redundant with the agent-level archive (`agent_session/archive.rs`, which already works on the global zone) and must still set `session:archived_at`, which `global_output_source` reads |
| `resume_retry.rs` (`has_prior_transcript`, `pane_history_tail`), `persistent/spawn.rs`, `session:resume_preflight` | existence and the tail, with a global fallback already | none needed |
| `backend/viewer/feed.rs`, `http_viewer.rs` | events keyed by the block id; the cursor from `transcript_position` (global first) | none; one extra resync on a brand-new agent |
| `publish_transcript_changed` (replace/delete announcements) | the `b:` position from `line_state(block)` | take the position from the global store, since the pane pins `g:` |

Must **keep** the block's copy: terminals (`term`), the `/btw` side-question throwaway block (no `agentId` by design), any block with no valid `agentId`, and any block whose global write is not made (global store absent, or the global zone cannot be resolved). Also: a writer that has lost the agent lease and is fenced out of the shared record (`persistent/stdout_reader.rs`) writes only the block's log today; without that write its output would leave no trace on disk, so a fenced writer keeps writing the block's copy.

## 3. The change

Write an agent's transcript line to the **global store only** when a global write is made (`resolve_global_output_zone` returns a zone and the global store is installed), and to the block's own store otherwise, and for a fenced writer.

- **No change to what is stored in the global zone.** The line, its positions, its counter and its sidecar are exactly what they are now.
- **Reads:** every row of §2 reads the global zone for such a block, falling back to the block's copy when the block has history from before this change (an old block's per-channel output stays where it is and is never deleted by this change).
- **Events:** the `b:<block>` position and the block's `offset` are no longer present for such a block; the `g:` position is. The cursor treats a `b:`-less event as it does today for a pane pinned to `g:`. Replace/delete announcements carry the `g:` position.
- **Setting:** `agent:transcriptsinglecopy` (boolean, **off by default**). Off, every build writes both copies exactly as before; on, a defined agent's line goes to the agent's zone first and to the block's own store only if that write failed (the line is never lost). A writer with no global zone (a fenced one) and a quick-launch pane keep writing the block's copy. The default is flipped in a separate one-line change once the lab and the nightly check have run with it on.
- **Announcements:** `publish_transcript_changed` (the replace and delete announcements of a session archive or restore) names the agent's zone when the block has no counted copy of its own.
- **Older builds** in other channels read the global zone before the block's own (they already do), so they see the same agent history. An older build in the *same* channel that opens a block written only to the global zone reads the global copy too; one that cannot (no global store) is not a supported combination today either.

## 4. Behaviours that change, and how they are checked

- Digest, title, summary, supervisor and worker-notes features keep working from the global tail: tests that run each against an agent block written only to the global zone.
- The progress watcher keeps its todo/progress feed from the block's file events: a test that feeds events with no store.
- `session:export`, `/transcript`, muxspect error/last-line: tests against a global-only block.
- The block archive and restore of an agent block: `archived_at` still set; the pane stops reading the block's (now absent) output; the agent-level archive carries the transcript.
- Shared zones: a quick-launch pane has a provider key as its `agentId` and so shares `agent:<provider>:current` with the other panes of that provider. `TranscriptSource` therefore uses the agent's zone only for a defined agent (an `agentId` that is not a provider key); a quick-launch pane keeps reading its own copy, and B must keep writing it for those panes. For a defined agent the zone is one conversation with one live writer (the agent lease), so a tail, a size or a modts taken from it is the agent's, which is what those readers mean.

## 5. Order of work

A. Make every reader in §2 able to read the global zone, with the block's copy as the fallback, and no change to what is written. Tests per reader. Safe to ship alone.
B. Stop the block-store write for agent blocks with a global write (this change's bytes), behind the setting, with the lab A/B: bytes written, storage, srv CPU, round time, and the watchers' and digests' own checks.
C. Remove the setting after a soak, then the dead block-store paths.

## 6. Measurement

The lab (Loap-3's `ab.mjs`, 12 panes, 10 rounds, with and without `--disk-load 50`): `result.io.writtenMB.srv`, `result.io.storageWrittenMB.srv`, `result.storeLock`, srv CPU and round time, before and after B, at least 4 runs per side. The earlier experiment is the expected size of the effect: −50% bytes, −43% srv CPU.

## 7. Open questions

1. Does anything outside this repository read a channel's block-store `output` (a paired device, a plugin, an export tool)? The in-repo audit found no such reader; I did not check other repositories.
2. The receive-time sidecar of the block's copy is read by the block-level archive and by pane history when it falls back to the block; both are in §2. Step A of `SPEC_FILESTORE_RECEIVE_TIME_SIDECAR_THROTTLE_2026_10_19` halves the sidecar's cost first and is independent of this.
3. Is the block's `output.state.json` (the pane's overlay snapshot, a different file in the block's zone) in scope? It is written twice as well (block and global) as a whole file each time; it is a separate item in the report's plan (step 3), not part of this change.
