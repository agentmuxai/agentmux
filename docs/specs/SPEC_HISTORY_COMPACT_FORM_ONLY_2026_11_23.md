# Spec: AgentMux keeps conversation history in one lossless compact form

**Date:** 2026-11-23
**Author:** Agent5
**Type:** Spec (a storage and retention change; what a person reads of a conversation does not change)
**Status:** active — PR 1 (#8429) adds the pure compact module and checks L1 to L4; PR 2 (#8431) adds the live file; PR 3 is built in parts: the provider-file module (#8437, proof, import, removal by session id), then its wiring (#8438, the session-end sweep, off by default; review fixes #8445), then the container path, the continuation packet and the retention seed; PR 5 is built in parts, 5a first (new data is written once: the per-channel copy is no longer written, no setting, existing copies untouched); PRs 4 to 7 of section 15 otherwise remain.
**Area:** history store
**Supersedes:** `SPEC_HISTORY_CONSOLIDATION_NO_RAW_RETENTION_2026_11_14.md` (never built; its migration protocol, §6 and §8 there, is reused here). **Builds on:** `SPEC_HISTORY_STORE_COMPRESSION_AND_COMPACTION_2026_10_21.md` (frames, the framer, recovery rules), which stays in force; its rule 6, "history is kept forever", is restated in §10.
**Task:** #48525 (spec), #48526 (build).

## 1. Requirement, in one paragraph

AgentMux keeps everything that reconstructs a conversation, once, in its own store: the person's messages, the agent's replies, every tool call with its full input and its full output, compaction summaries, and their order and times. It keeps nothing that only repeats it. **Redundant forms are removed:** streaming fragments once their reply is complete; the provider CLIs' own session files once the store provably holds the same content; and copies in the per-channel store, old version folders, backups, snapshots, logs and dumps. Removal is real: freed database space is overwritten, and nothing readable remains in a backup or snapshot. The one remaining copy is protected at rest (§7). This holds on a new install from its first turn and on an existing install after upgrade. No setting turns redundant retention back on (§12).

The guarantee is stated about **what is on disk under the AgentMux home** after a reply is complete (§4), after a session has ended (§5) and after the migration (§9), and it is tested with marker strings (§13). Nothing the person can read of a finished conversation is lost.

## 2. The compact form

The compact form is **the store's existing line format, minus the streaming fragments**. It is lossless: every line the provider's CLI emitted that is not a fragment is kept byte for byte (after the secret-withhold marking the store already applies, which stays).

| Line | Kept as |
|---|---|
| The person's messages, jekt bodies as delivered | the `user` line as it is |
| The agent's replies | the complete `assistant` line as it is: text, thinking and signature blocks, `tool_use` blocks **with their full `input`** |
| Tool results | the `user` line carrying `tool_result`, **with its full content**, and the `tool_use_result` payload where the CLI sent one |
| Compaction | the boundary and the summary message as they are; the continuity summary and handoff files |
| Turn boundaries, times, usage | `result`, `system` (`init` and the rest), hook lines, rate-limit lines, as they are; the receive-time sidecar `output.tsidx` |
| Subagent lines | as they are |

**Dropped, and only these:**

1. **`stream_event` lines** (partial-message fragments), for a message whose complete `assistant` line has been written. The build proves it per message (check L2, §9): the fragments reassemble to the complete message's blocks.
2. **Lines marked as replays** (`isReplay`), as today: they repeat lines already stored.

**An interrupted reply** (the process was killed, or the stream ended before the complete line) has fragments and no complete line. The fragments are **reassembled into one `assistant` line marked `interrupted`**, with the text and tool-call input received so far. Nothing the person could see is lost.

The rule list is data (in `transcript/compact.rs`) and is **keep by default**: a line kind it does not name is kept. A new CLI line kind is therefore never lost by an omission; adding a drop needs a rule and a check.

**Alternative and its cost.** A deterministic *reduction* of tool input and output (a digest, a name only) would make the store much smaller, but it is not lossless: what a command printed, what an edit changed, what a search returned would be unrecoverable. Not chosen. The cost of the lossless form is storage: tool output is the bulk of a long conversation, and the frame codec (compression) is what keeps it affordable.

## 3. Every place conversation text is written, and what happens to it

"The store" below is the shared transcript store (`shared/agents/transcripts/filestore.db`), zone `agent:<id>:current` and its archives.

| # | Surface | Today | Under this spec |
|---|---|---|---|
| 1 | The store, `output` of a live agent | every line including fragments, forever | the compact form of §2, forever; fragments never reach it (§4) |
| 2 | Per-channel copy of the same output (written alongside while `agent:transcriptsinglecopy` is off, the default) | a second full copy | **removed.** Single copy becomes the only mode. Existing copies are deleted after the store is verified to hold the same conversation (§9 check L5) |
| 3 | Sidecars: `output.tsidx`, `output.state.json` | times; the pane's rendering snapshot | times kept; the snapshot is a rebuildable cache and holds no text the store lacks |
| 4 | Continuity summary and handoff | model-written rolling summary | kept (it is a summary, as a compaction summary is); forked and archived with the conversation |
| 5 | Fork copy | copies output and sidecars | copies the compact form; a fork of a live session may use the provider's fork while its file exists (§5) |
| 6 | Archives: zones `agent:<id>:archive:<ms>` and gzipped session files under `archives/` | the whole conversation, forever | the compact form only; a gzipped provider session file is a redundant form (§5) and goes through the same proof and removal |
| 7 | Activity summary, ambient digest | one derived line; in memory | unchanged |
| 8 | Attachments: originals and extracted text (`.text.txt`) | originals 30 days after last use; extracted text with them | originals unchanged (the person's files). The text the model received is in the user line. The extracted-text cache is deleted when its session ends; the build's audit confirms the user line carries what was sent |
| 9 | Jekt audit; audit activity index | a hash and length; a scrubbed one-line summary with a position | unchanged: neither is a second copy of a conversation (a hash; a pointer with a short scrubbed line) |
| 10 | Application log files | a debug-level line per stdout line in full; ids elsewhere | **no log at any level carries a prompt, reply, thinking, tool input or result, or stream line** (ids, counts, sizes); the debug stdout log is removed. Existing log files are deleted by the migration |
| 11 | Crash and hang dumps | whole process memory; hang dumps newest five; server crash dumps never pruned | pruned by count (newest 3) and age (7 days), owner-only, never uploaded. A dump is a moment of memory, not retained history; §11 says what it leaves open |
| 12 | Pre-upgrade snapshots | a copy of the per-channel store, five per channel | a snapshot is taken **without the transcript zones**; existing snapshots are deleted and overwritten once their channel's migration is verified |
| 13 | Viewer feed | the live transcript streamed over the LAN, token-gated | live reply from memory; finished replies from the store |
| 14 | Backfill from channel copies | copies the richest output into the store | removed with the copies (row 2); the one-time check L5 reads them before deletion |
| 15 | Bundle export | includes provider session files | renders the store's compact form (NDJSON or Markdown); a provider session file is never included |
| 16 | Provider CLIs' own session files | raw, no retention, kept for resume | §5 |
| 17 | Container agents' volumes | the provider's home is a named volume per container | §5: session files removed by session id at session end; the volume is removed with the agent, as today |
| 18 | Test and dev homes | the same code under their own home | the same guarantee; tested as a new install (§13) |
| 19 | Old version folders (`channels/<ch>/versions/<v>/data/db/`), shared pre-migration backups, `.pre-reclaim`, `.damaged-<ms>` repair files, slug-migration backups | full database copies, kept | **removed** once the running version's store is verified (rules in §6) |

## 4. A reply in progress

1. **While a reply streams**, its fragments go to **`output.live`**, a second file in the agent's zone, and to the pane's feed as they do today. **`output` never receives a fragment.** Complete lines (`user`, complete `assistant`, tool results, `result`, `system`) are appended to `output` as the CLI emits them.
2. **When the complete `assistant` line arrives**, it is appended to `output` and that message's fragments leave `output.live`, in one transaction. A reply whose reassembly check (L2) fails keeps its fragments in `output.live` and is retried at the next start; it is not appended as fragments.
3. **A reader sees one stream:** `output` followed by `output.live` as one logical file (line 0 at byte 0, contiguous); the live reply is the tail. At close a reader positioned inside the tail receives one `reply_closed` notice (the tail's first line and the complete line's number) and takes that line from `output`. **Positions before the tail do not move**: no generation change and no re-open.
4. **A crash or kill** leaves `output.live`; the next start reassembles it into an `interrupted` assistant line (§2) and empties it.

**Alternatives and their cost.** (a) *Write fragments to `output` and remove them at reply end, in place:* one file, but every reply end rewrites the tail and invalidates every open reader's position. (b) *Fragments in memory only:* no live file, but a reconnecting pane has nothing to replay and a crash loses the partial reply. (c) *Remove fragments an hour later (the 11-14 design):* fragments sit on disk after every reply. Recommendation: the live file.

## 5. The provider's session file

### 5.1 Can AgentMux rebuild it from the store? No.

Answered from the code, not by guessing:

- **AgentMux has no code that writes a provider session file from the store.** The only thing it does with one is copy an existing Claude file, trimmed to its last complete line, into a new session's folder for a fork or relocation. Codex's rollout files are only read. Importing an exported agent places no session.
- **What resume checks is that the file exists** (and, for relocation, that its last record parses). A missing file makes Claude report "no conversation found", the id is retired, and the agent starts fresh with a **continuation packet**: a text summary of the conversation built from the store. So the store already serves as a text fallback, not as a faithful rebuild.
- **The stored lines are not the session file's lines.** The store keeps the CLI's output stream, with an `amx` stamp added, secrets replaced by a mark, replays dropped and result cost totals rewritten. The session file carries fields the code reads (`uuid`, `parentUuid`, `cwd`, `gitBranch`, `slug`, `timestamp`) and, in Claude's own format, more (sidechain flags, request ids, version). Whether the real output stream carries the `uuid` chain is **not verified** by any fixture in the repository. Whether Claude accepts a hand-built file has never been tried with the real CLI.

**Consequence, stated plainly:** a provider session file **stays until its session ends** (§5.4). It cannot be removed at each turn or at each stop, because the agent resumes from it and AgentMux cannot recreate it. After a session ends, nothing resumes it (§5.4), so removal is then safe.

**Alternatives and their cost.** (a) *Build a rebuild:* if it worked (a real-CLI experiment must show Claude accepts the rebuilt file, including the `uuid` chain, which the stored lines may not carry), the provider file could go at every stop and resume would be exact; the cost is a converter per provider, a field gap that may need the stream to be stored differently, and a test against each real CLI. Recommended as a **separate, later piece of work** (task to file), not part of this build. (b) *Delete at the end of each turn:* resume is impossible. (c) *Keep for the life of the agent:* the redundant form lasts as long as the agent does, which the requirement forbids.

### 5.2 The per-session proof, before any provider file is removed

A provider file (and its session folder: tool-result spill files, subagent transcripts) is removed only after a proof **for that session**:

1. **Parse** the provider file's records into conversation records: role, text, thinking, tool calls (id, name, full input), tool results (id, full content, error flag), compaction boundaries and summaries, timestamps (to the second), in order.
2. **Compare record by record** with the store's lines for the same session id, by content after normalising what the store changes by design: the `amx` stamp, the withhold mark (where the provider file holds a secret value and the store holds the mark, they match), result cost totals.
3. **Pass** means every provider record has its counterpart, in order, with equal content, **and every file in the session folder is accounted for** (its content is in a stored tool result or is imported, step 4).
4. **Anything the store lacks is kept, not lost.** A provider record, spill file or subagent transcript with no counterpart is **imported verbatim** into the store (as `import/<session>/...`, compressed, in the provider's own format, which the history readers already parse). Provider-only fields on matched records (the `uuid` and `parentUuid` chain, sidechain and request ids, version, slug) are kept in a small per-session sidecar `output.provider` (record number and the field subset). After step 4 the second proof run must pass; **only then is the file removed.**
5. **A failed proof removes nothing.** The file stays, the failure is reported (counts only), and the proof is retried at the next session end or start.

The test the proof implements is the one the operator asked for: reconstruct from the store, compare record by record. It runs in the direction the code allows (provider file against store), because the store cannot rebuild the file (§5.1).

### 5.3 Sessions that exist only as provider files

A session whose provider file is older than the store, or is not in the store for any reason, is **imported first, never deleted**: the migration (§9) runs steps 1 to 4 over every session file it finds under every account's provider home and every agent's config directory that AgentMux started, and removal follows the proof like any other. The import preserves the provider's records verbatim, so nothing of those sessions is lost.

### 5.4 When a session ends

A stop, restart or quit **does not end a session**: agents resume with the provider's `--resume` many times a day. A session ends when:

- the person starts a **fresh session**, **archives** the conversation, or **deletes** the agent; or
- the agent has been **idle for 7 days** (`agent:rawsessionidledays`, default 7, lower allowed, higher capped at 30).

At the end AgentMux runs the proof (§5.2) and removes the session's files **by session id**: the `<session>.jsonl`, its `<session>/` folder, and the equivalent for any other harness that has a file AgentMux can name. It never sweeps a directory, and never touches a file whose session id it did not start. A person's own CLI home bound to an agent keeps its own sessions.

An agent that starts after its session ended has no provider file and continues from the **continuation packet** (built from the store: messages, summaries and, now, the tool calls and outputs it needs), which starts a new provider session. It knows the conversation; it loses the provider's own session state. That is the cost of the idle limit, and why it is 7 days, not 1.

**Alternatives and their cost.** (a) *No idle limit:* a conversation never ended keeps a redundant provider file forever. (b) *A short limit (one day):* resumes after a weekend start from a packet. Recommendation: end on fresh, archive, delete or 7 days idle.

**Container agents:** the session files are in the container's volume. At session end AgentMux removes them by session id through the container (or a short helper container if it is stopped); deleting the agent removes the volume, as today. **Harnesses with no file AgentMux can name** keep their sessions in the protocol or elsewhere: the build lists each with its removal rule or "none found".

### 5.5 The 36,500-day retention seed

AgentMux seeds `cleanupPeriodDays: 36500` into each isolated Claude config directory's `settings.json` so that Claude's own start-up sweep never deletes the session files AgentMux links to shared history. That made the provider file the one place raw history lived forever, which this spec ends.

- **New value.** AgentMux writes `cleanupPeriodDays` = the idle limit + 30 days (default 37, at most 60). Claude's sweep is then only a backstop that fires after AgentMux's own proven removal would have run; it never decides first.
- **Order matters.** The seed changes only **after** the migration has proved and imported every session under that account's `projects` folder (§5.3). Lowering it earlier lets the sweep delete files the store does not yet hold. No backup of the folder is taken to cover this (a backup would be a raw copy, §6).
- **Existing seeds.** Today the seed writes only when the key is absent or still the old default (180). The change also rewrites the value 36500 and leaves any other value a person set alone (their file, their choice; AgentMux's own removal still runs by session id).
- **Other harnesses:** the build reports any harness with a similar keep-forever setting and handles it the same way, or records "none".

## 6. Real removal

"Removed" means the bytes are not recoverable from any file AgentMux owns, not that a row was deleted:

1. **Freed database space is overwritten.** Every connection to the transcript store (and the per-channel stores while they exist) sets `PRAGMA secure_delete=ON`, so a deleted frame or part is zeroed in its page. Today nothing sets it, and freed pages keep their bytes.
2. **The write-ahead log is scrubbed.** After a deletion-class step (the migration swap, a session removal, a copy's deletion) AgentMux runs a checkpoint that **truncates** the log and then overwrites the freed region of the log file, so old page versions do not survive in it.
3. **Whole copies are overwritten, then unlinked.** The old store at a swap, `.pre-reclaim`, `.damaged-<ms>` (once the repaired store has been verified against it; if rows were unrecoverable it stays until the person decides), old version folders' databases, snapshots, the per-channel copy, provider session files and their folders: each file is overwritten with zeros in place (and synced) before its unlink. A reclaim (`VACUUM INTO` and swap) does this to the file it replaces.
4. **No backup or snapshot of the transcript zones is made.** Pre-upgrade snapshots exclude them (§3 row 12); the shared pre-migration backups are scanned by the build and made to exclude conversation content (the build's audit says which tables they hold). An upgrade's safety net for the transcript store is the migration's own verify-then-swap (§9), not a copy.
5. **What this does not reach:** a file system or drive that keeps old blocks (wear levelling, copy-on-write, snapshots by the operating system or a backup program). The guarantee is stated at the level of files AgentMux owns and their free space; §7 gives the answer for the device (disk encryption).

## 7. Protecting the single remaining copy at rest

Requirements, not designs. The store is now the only copy of a conversation, so it is more valuable and more exposed:

1. **Owner-only access.** The store, its log and shared-memory files, the live file, sidecars, the imported-session data, and the `agents/transcripts/` folder are readable and writable by the owning user only: mode `0600` and `0700` on Unix, an access-control list limited to the owner on Windows. Applied when a file is created and **re-applied at every start** to existing files and to the folders above.
2. **No world, group or network exposure.** The store file is never served as a file. Its content leaves the machine only through the authenticated, token-gated paths that exist today (the viewer feed, the agent API), and each such path is checked by the build's audit.
3. **Encryption at rest.** AgentMux relies on the operating system's disk encryption and says so: the Storage view shows whether the volume is encrypted where the operating system reports it, and the docs say that the history store is unencrypted by AgentMux. Encrypting the store itself (a key held in the operating system's secret store, never in a file beside it) is a **separate spec**, not part of this build. Cost of doing it now: a key lifecycle (loss, rotation, a second device, recovery), a performance cost on every read, and a migration of every existing store.
4. **Integrity and durability of the one copy.** The store runs with `synchronous=FULL` for transcript writes (a crash does not lose an acknowledged line), `PRAGMA quick_check` at start and on the existing 30-minute checkpoint, and **nothing is deleted before the thing it duplicates has been verified** (the proof of §5.2, the checks of §9). A repair never deletes the damaged file until the repaired store is verified against it (§6).
5. **Exports are the person's copies.** An export is written owner-only and named as containing the conversation; AgentMux keeps no copy of it.
6. **No secrets in the store.** The existing secret-withhold marking applies to every line before it is written, and to imports (§5.2 step 4): an imported provider record has its secret values replaced by the mark.

## 8. What changes for people and agents

Very little, by design: the compact form keeps everything the readers use.

| Feature | Under this spec |
|---|---|
| Pane, tool rows, diffs, Read views, result reload | unchanged: inputs and outputs are in the store |
| History search (by tool, text, target) | unchanged |
| History and Audit views | unchanged |
| `Agents transcript`, `Agents watch`, transcript requests between agents | unchanged. `Agents watch` keeps its byte cursor: positions before a live tail never move (§4); a cursor into a tail that closed gets the `reply_closed` result |
| Progress tracker, checklist, ambient digest, token and usage counting, loop guard | unchanged |
| Resume of a live session | unchanged: the provider file is still there (§5.1) |
| Resume of an ended or idle-expired session | the continuation packet (§5.4) |
| Export | the compact form (§3 row 15) |
| Recovery tool (`scripts/recover-shared-transcripts`) | unchanged; reads the compact form |
| Storage use | smaller than today (no fragments, no second copy, no provider file), and the reclaim spec returns freed pages to the disk |

## 9. New installs and upgrade

**New install:** all of the above from the first turn; no migration. The seed is written with the new value (§5.5).

**Upgrade.** One worker per zone, in the background, rate-limited, resumable, one file at a time, in every srv that reads the new format; no model tokens. It reuses the 11-14 spec's protocol: **never edit in place**; read through snapshots in turn-aligned windows; write **`output.next`**; check; swap in one transaction at a quiet point; a new generation and an old-to-new line map so open readers and the pane's snapshot translate; a record of what was done (`output.consol`).

**Checks before a swap (per file):**

- **L1.** The kept lines equal the old file's lines minus the dropped kinds, **byte for byte and in order** (hash per line, hash of the sequence).
- **L2.** Every dropped `stream_event` belongs to a message whose complete line is kept and whose blocks the fragments reassemble to; a message with no complete line is reassembled into an `interrupted` line, whose text equals the reassembled fragments.
- **L3.** Counts by line kind, old against new, differ only by the dropped kinds; every session id and tool-use id that appeared in any old line appears in a kept line.
- **L4.** A scan: no `stream_event` line remains in `output`.
- **L5.** For a per-channel copy about to be deleted: its conversation (kept lines by hash) is contained in the store's; anything it holds that the store lacks is appended to the store first (this is the backfill rule, run once).

**Failure leaves everything as it was:** the old file is untouched until the swap; a crash at any step leaves the old file. A file that fails a check is not declared compact and is retried; the failure is reported (counts only).

**Order of the upgrade's steps:** reduce the store (L1 to L4) → verify and delete per-channel copies (L5) → import and prove provider files (§5.3) → delete proven provider files → change the retention seed (§5.5) → delete old version folders' databases, snapshots, backups, logs and repair files (§6 and §3 rows 10, 12, 19), each only after its channel's checks passed.

**A build without this change** opens a compact store normally (the compact form uses only existing line kinds and adds no frame codec, so the version stamp does not change). It writes fragments again; the next start of a build that carries this change removes them (the pass is idempotent and checks every zone at start). The guarantee holds for builds that carry this change. **A downgrade to a build that predates the shared store is not supported**, because the per-channel copy it would read is gone.

**Space:** the new file is smaller; its pages return to the free list at the swap and are overwritten (§6).

## 10. "History is kept forever"

Rule 6 of the frame spec stays true and gets sharper: **every message, reply, tool call with its input and output, compaction summary, order and time is kept forever, in the compact form.** What is not kept is redundant: fragments, provider files, copies. Nothing in this change deletes anything a person could read of a conversation, and each removal of a copy is gated on a verified equal in the store.

## 11. What the guarantee does not cover

- A reply in progress, in memory and in `output.live`.
- A session that has not ended: its provider file exists until it ends or is idle for 7 days (§5.1, §5.4).
- A crash or hang dump holds what was in memory at that moment; it is owner-only and pruned (§3 row 11).
- What a person copied out themselves (exports, screenshots, pasted text), what the model providers retain of what was sent to them, and what the operating system keeps (swap, hibernation, drive snapshots, whole-disk backups, remnants on flash storage).
- A person's own CLI home outside AgentMux.

## 12. Settings

**No setting turns redundant retention on.** `agent:rawsessionidledays` (§5.4) can only shorten the life of a provider file, within its cap. **A person cannot opt in**: a "keep provider files" or "keep fragments" switch would recreate the second copy, its migration and its tests, and would be set by anyone who wants an older tool to keep working. A **debug capture** exists for developers only: the environment variable `AGENTMUX_DEBUG_KEEP_RAW=1`, honoured **only in a development build**, not persisted, active for the life of that process, shown as a banner in the pane and the Storage view, writing to a folder removed when the process ends. **The settings this change adds** are `history:sessionfilesweep` (the session-end sweep of §5.4; off until PR 6, which makes it on by default for a new install from its first start and turns it on for an existing install only after the migration has proved every session, §9) and `agent:rawsessionidledays` (§5.4). The per-channel copy has no setting (§3 row 2): new data is written once. **Deleting an agent removes its `agent-uid:<uid>:provider` zone (§5.2 step 4) with its other zones**; otherwise the proof finds nothing to match and the whole provider conversation would be imported again.

## 13. Tests (the guarantee, proved)

Markers are strings unique to a test, written by a fake provider that emits the stream a real one does into a temporary AgentMux home, and also writes its own provider session file (and a session folder with a spill file) under the agent's config directory.

1. **New install, one reply.** Marker A in the user message, B in a thinking block, C in a Bash `tool_use` input, D in its `tool_result` content, E in an Edit's new content, F in a field that only `stream_event` lines carry. After the reply completes: A to E are readable **exactly once each** through the store's readers (frames are compressed, so the test reads through the store, never the raw file, for this assertion); F is nowhere: not in the store's logical lines, not in `output.live`, not in any file under the home (a byte scan of every file, with the store's own file scanned with compression off for the test, free pages and log included).
2. **Provider file while the session lives:** present (a negative test of the removal's scope); a second session of the same agent is untouched.
3. **Session end** (fresh start, then separately archive, delete and the idle limit with a faked clock): the proof runs; on pass the provider file and its folder are gone, **overwritten before unlink** (checked through the removal function's hook), and the marker unique to the provider file (G) is in no file under the home. On a deliberately failing proof (a record removed from the store) **nothing is removed** and the failure is reported.
4. **A provider-only session** (a file with no store record) is imported first: its records and a spill file's content are readable from the store verbatim, the proof then passes, and only then is the file removed. Provider-only fields (marker H on a `uuid`) are in `output.provider`.
5. **A kill mid-reply:** `output.live` holds fragments; the next start reassembles an `interrupted` line containing the received text and empties the file.
6. **A foreign file** whose name carries a session id AgentMux did not start is not deleted.
7. **Migration:** a synthetic raw store with every line kind (fragments interleaved with complete lines, an interrupted reply, a reply crossing a window, subagent lines, replays, a per-channel copy with a line the store lacks, an archive, a snapshot, an old version folder) reduces; L1 to L5 hold and are also checked on a corrupted result (a line reordered, a complete line dropped, a surviving fragment), which fails and leaves the old file; a crash at each step leaves the old file; a concurrent append lands in the live tail; a second run changes nothing.
8. **Deletion semantics:** `secure_delete` is on for every connection to a transcript store (a pragma check at open); after a swap and after a copy's deletion, a scan of the database file, its log, `.pre-reclaim` and every removed path finds no marker of the removed content; the log file is truncated and overwritten.
9. **Logs:** a test greps every log written by a full fake session, at every level, for A to G.
10. **Readers:** the same conversation read from an old raw store and from its compact result gives identical messages, tool rows, checklist, search results and `Agents watch` output (apart from line numbers, which the map translates); a reader inside a closed tail gets `reply_closed`.
11. **At rest:** after start, the store, its sidecars, the live file, the imported data and the folder have owner-only permissions (re-applied to files created loosely before); no route serves the store file; `quick_check` runs at start.
12. **Retention seed:** a config directory seeded with 36500 is rewritten to the new value only after its sessions are proved; a user-set value is left alone.
13. **An older build's view:** a compact store opens in the previous release's reader without error and renders the conversation.

## 14. Decisions for the operator

| # | Decision | Recommendation | Cost of the alternative |
|---|---|---|---|
| 1 | Thinking blocks are kept as part of the reply (the compact form drops only fragments and replays) | keep | dropping them is lossy for the provider's signatures and for a person reading the reply; smaller store |
| 2 | The provider file stays until its session ends (§5.1), because AgentMux cannot rebuild it | as written | a rebuild is a separate, later piece of work with a real-CLI experiment |
| 3 | A session ends on fresh, archive, delete or 7 days idle (§5.4); 7 is a setting with a cap of 30 | as written | no limit keeps a redundant file for the life of an agent; one day makes weekend starts resume from a packet |
| 4 | Keep provider-only fields and unmatched records in the store (§5.2 step 4) rather than accept their loss | keep | accepting the loss makes the proof simpler but is not lossless |
| 5 | Delete per-channel copies once the store provably holds the same, and drop support for downgrade to a build before the shared store (§9) | delete | keeping them is a second copy and what the requirement removes |
| 6 | Retention seed becomes idle limit + 30 days, changed only after migration (§5.5) | as written | removing the key restores Claude's 30-day sweep, which can delete a file the store does not yet hold; keeping 36500 keeps the redundant file forever |
| 7 | Rely on the operating system's disk encryption now; store-level encryption is a later spec (§7.3) | as written | store encryption now adds a key lifecycle to this build |
| 8 | No person-level opt-in; a dev-build-only debug capture (§12) | as written | an opt-in recreates the second copy |
| 9 | Crash dump pruning: newest 3 and 7 days (§3 row 11) | as written | unpruned dumps keep whole memory indefinitely |
| 10 | Migrate existing stores on upgrade, with no grace copy of what is removed | yes; L1 to L5 and the proof run before every removal | a grace copy is a raw copy, which the requirement forbids |

## 15. Build plan (#48526)

| PR | What | Changes behaviour? |
|---|---|---|
| 1 | `transcript/compact.rs`: the rule list (keep by default), fragment reassembly, checks L1 to L4 and their tests; nothing calls it | no |
| 2 | The live file and the append path (§4): fragments to `output.live`, complete lines to `output`, `reply_closed`, the one-stream reader, crash reassembly | yes: new data has no fragments |
| 3 | Provider files (§5): the parser, the proof, import, `output.provider`, session-id removal, the idle limit, the container path, the continuation packet on an ended session, the seed (§5.5) | yes |
| 4 | Real removal and at rest (§6, §7): `secure_delete`, log scrub, overwrite-then-unlink, owner-only permissions at start, `quick_check`, `synchronous=FULL` | yes |
| 5 | The other surfaces (§3 rows 2, 8, 10 to 12, 15, 19): single copy only, log changes, snapshot exclusion, dump pruning, export | yes |
| 6 | The migration (§9) behind a setting that is off until PRs 1 to 5 have soaked, then on | yes |
| 7 | The marker tests of §13 as a CI gate | no |

Each PR names the consumers it audited (§8 and the 11-14 spec's consumer list).

## 16. Not covered

The model providers' own retention of what is sent to them; a person's own CLI homes; what the operating system keeps; building a provider-file rebuild (§5.1 alternative (a)); encrypting the store itself (§7.3); sharing a parent's compacted prefix with forks (the 11-14 spec's §6.5).
