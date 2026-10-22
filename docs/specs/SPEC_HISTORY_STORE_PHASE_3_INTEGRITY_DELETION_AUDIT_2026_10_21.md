# Spec: Warden History phase 3: integrity, deliberate deletion and a durable audit, in layers

**Date:** 2026-10-21
**Author:** Agent5@narko
**Type:** Spec (new Warden views, a deletion command, an integrity log)
**Status:** draft, for the operator's approval. Nothing is built. Task #51.
**Builds on:** `SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md` §5.2 (phase 3, not started), `REPORT_AGENT_HISTORY_STORE_DEDUP_COMPRESSION_2026_10_17.md` §6 (AgentY: a content-addressed store with a Merkle log), `SPEC_HISTORY_STORE_COMPRESSION_AND_COMPACTION_2026_10_21.md` (compressed frames, task #49), the damage report `REPORT_SHARED_TRANSCRIPT_STORE_DAMAGE_2026_10_21.md` (task #34).
**Platform:** all.

## 1. What this decides, in one paragraph

Phase 3 of the History section was designed as one big thing: a new content-addressed store with a Merkle log, from which a new Integrity view, a durable Audit, deletion and the Storage numbers all follow. Looking at what each piece needs, **almost none of it needs a new store.** The audit trail is already durable on disk; deletion needs a safe delete of a zone; integrity needs a hash per sealed part and a signed log over those hashes, which a compressed-frames store (task #49) computes anyway; only deduplication needs content addressing, and on this machine the measured benefit is small once frames exist. So I propose four layers, each shippable and useful alone, in this order: **A** deliberate deletion, a durable Audit view and truthful Storage numbers (on today's store); **B** an integrity log and the Integrity view; **C** a content-addressed store, deferred and re-measured after A and B; **D** an optional cloud mirror, a separate spec. The operator's rules stand: history is kept forever by default, and deleting any of it is deliberate, confirmed and recorded.

## 2. Why the store is not the first step

- **The damage we just found is the case for integrity, not for dedup.** Four rows of the shared store were silently damaged (an integrity check found them; nothing in AgentMux did). A per-part hash with a periodic verify would have found them on the day. That is layer B, and it works on the store as it is.
- **Audit is already durable.** `backend/reactive/audit_store.rs` appends every jekt audit entry (a hash and length of each message, never its text, plus sender, target, outcome, note, channel) to `shared/audit/jekt-audit.jsonl`, never rotated. What does not survive a restart is the Warden Audit view, which reads `GET /agentmux/reactive/audit`, an in-memory ring of 100 entries. The fix is to read the file.
- **Dedup's value moved.** AgentY measured 2.5 GB (44%) of the provider files as repeated large fields, but those are Claude's own session files, read by the CLI on resume; making them a cache means rebuilding byte-exact JSONL on every resume, a deep change. For the pane store (15.4 GB) the compression spec measures 5.6x from 1 MiB frames alone; what dedup adds after that is mostly fork copies, and forks are rare today.
- **Deletion needs a safe zone delete, which does not exist** (I had to leave a 1.5 MB orphan zone after a live test because there was no way to remove it).

## 3. Layer A: on today's store

### A1. Storage: stored size, logical size, and what is duplicated

- Per store and per agent: stored bytes and logical bytes (decoded; equal until frames exist), the ratio, and bytes not yet compressed. `history.usage` already reads file sizes; it adds `SUM(length(data))` over the part and frame tables.
- **Duplication, as it is today:** two real kinds. *Fork copies*: a fork's zone starts as a copy of its parent's (`fork.json` names the parent session); the duplicated bytes are the fork zone's size up to the fork point. *Per-channel copies*: every srv writes each pane transcript to its own store (`SPEC_AGENT_TRANSCRIPT_SINGLE_COPY`); measured as the channel databases (6.55 GB over 34 files on this machine). Shown as "kept twice" with the reason and, for channel copies, the setting that stops it.
- Cost: a few queries on a cached result, like today's walk.

### A2. Audit: read the durable file

- Warden Audit reads `jekt-audit.jsonl` through a command `audit.page` (newest first, by time range, sender, target, outcome), instead of the in-memory ring, and merges the live ring for entries not yet flushed. A restart no longer empties the view.
- The file is shared by every srv on the machine and each line names its channel; the view shows the channel and can filter on it.
- **Scope:** jekts only, as today. The history report (§3.5) names what is not recorded anywhere outside the transcript: permission grants, tool approvals, account switches. Adding them is a separate, small change to the same file with the same rule (hashes and lengths, never content), proposed here but not part of A2.
- Reads the file by seeking from the end; a gigabyte of audit is not loaded.

### A3. Deliberate deletion

A "Delete history" action on an agent's row in Warden > History > Agents. Nothing deletes history any other way; the archive cap and the silent cleanups are gone (#5080).

**What it lists before asking** (all measured, not guessed):

| Store | What a delete does | Notes |
|---|---|---|
| Shared transcript store: the agent's current and archived zones (`agent:<id>:current`, `agent:<id>:archive:*`) | removes the zones' rows in one transaction | the pane history and the Agent History view |
| Per-channel copies (each version's `filestore.db`) | removes the block zones of this agent's panes | optional, listed with bytes |
| The CLI's own session files | **not touched by default** | deleting them ends native resume of those sessions; offered as a separate checkbox with that consequence spelled out |
| Archives (`.jsonl.gz`) | listed, not deleted by default | |

**The confirmation:** the dialog shows the agent's name, its sessions and dates, and the bytes in each store; the person types the agent's name to confirm. A deleted agent that is still listed (history is kept for deleted agents) deletes the same way.

**Export first (optional):** "Save a copy before deleting" writes the agent's transcript as `.jsonl.gz` to a folder the person picks (the host picks it, not the pane).

**The safe delete** (`FileStore::delete_zone_safe`): one transaction per zone; it refuses a zone with an `output` written in the last minutes or a pane open on it; it deletes the part rows, the file rows and the sidecars, then records the deletion. A damaged row (the 2026-10-20 case) is deleted by dropping the rows, not by walking the broken chain: if SQLite fails, the zone is left and reported (the offline recovery tool handles it). It also removes the orphan zones left by tests.

**The record:** an entry in the audit file (A2) of kind `history.deleted`: who asked (the operator, by the pane), when, which zones, bytes and line counts, and, once layer B exists, the log's checkpoint at that moment, so the log stays verifiable across the deletion (§4.4). It never contains transcript text.

**Agents never delete history.** There is no tool for it; an agent that asks gets a refusal that says to ask the operator.

## 4. Layer B: the integrity log

### 4.1 What it is

A hash per **sealed part** and a signed, append-only log over those hashes.

- A part is sealed when it is cold: behind the end of its file by the safety margin of the frames spec (8 MiB and 24 hours), so it will not be rewritten. The tail part, rewritten by every append, is never sealed.
- `db_history_seal (zoneid, name, partidx, sha256 BLOB, sealed_ms)`: 32 bytes per 64 KiB, about 0.05% of the data (7.5 MB for 232,000 parts).
- **The log:** per zone, a Merkle tree over its seals in part order (RFC 9162 style: an inclusion proof for a part, a consistency proof that an older root is a prefix of a newer one). A **checkpoint** (zone, tree size, root, time) is signed with the instance's Ed25519 key, the key that already signs WAN jekts (`agentmux_common::jekt_sign`). Checkpoints are append-only rows.
- If frames exist (task #49) the seal is the frame's `check` (a BLAKE3 of the frame's logical bytes) and the seal unit is the frame; the hash cost is already paid. Without frames the unit is the part. The Merkle layer is the same.
- **Cost:** hashing at seal time (a cold 1 MiB hashes in about a millisecond); the verify reads and hashes everything it checks (15 GB in a few minutes on this machine's disk, at low priority), which is why it is incremental and per zone.

### 4.2 What it detects, honestly

- **Detects:** silent corruption (the WAL-reset kind, bit rot, a restore from a stale backup), a part rewritten by something that does not hold the key, a zone replaced or truncated, a deletion that was not recorded.
- **Does not detect:** anything done by a process with the key. The key is in the same user account as the store, the same accepted limit as in the WAN spec. A rewrite plus re-signing by that user succeeds. The mitigation is an external anchor (§6): checkpoints published to a place the machine's user cannot rewrite. Until then this is an integrity check, not a proof for an auditor, and the view says so.

### 4.3 Verify history

`history.verify` (zone, or all; resumable; low priority) re-reads each sealed part or frame, recomputes its hash, compares it with the log, verifies the checkpoint signatures, and checks that each checkpoint's root is consistent with the next. It returns counts and findings: **ok**, **mismatch** (a sealed part's bytes changed), **missing** (a sealed part has no row), **unreadable** (a damaged row), **redacted** or **deleted** (recorded in the log: reported, not failures). A scheduled pass (a slice per day) runs without being asked and raises an incident (#5204's mechanism) on a finding.

### 4.4 Deletion and redaction stay verifiable

- **Deletion:** `history.deleted` (A3) writes a log record with the zone's final root; the zone's seals are then removable, and the log still shows that the zone existed, what it held (size, root) and when it was deleted.
- **Redaction** (removing one secret from a transcript, which is the only edit the store ever allows): the affected part is rewritten, its old seal is kept in a `redacted` record (old hash, new hash, reason, who, when), and the new part is sealed. Verify reports it as redacted. Redaction is a deliberate action with the same confirmation as deletion; the UI for choosing what to redact is a later piece of work and is not specified here.

### 4.5 The Integrity view in Warden > History

| Part | Shows |
|---|---|
| **Status** | the last full verify: when, how long, parts checked, result; "Verify now" for one agent or everything, with progress; the next scheduled slice |
| **Checkpoints** | the latest signed checkpoint per zone: size, root (short), signed time, which instance signed it (`host~id8`, as in `TRUST=wan-verified`); "Copy proof" for a part (an inclusion proof plus its checkpoint) |
| **Findings** | each mismatch, missing or unreadable part: agent, file, part, what to do (a link to the recovery procedure and the offline tool) |
| **Redactions and deletions** | the recorded list: what, when, who, bytes; deletions link to their `history.deleted` entry |
| **Limits** | one line: "Checkpoints are signed by this install's key and are not anchored outside this machine" until §6 exists |

## 5. Layer C: a content-addressed store (deferred)

AgentY's proposal (§6.1 to §6.4: blobs keyed by SHA-256, events as recipes with placeholders, a transcript as an ordered list of event keys, forks as `parent @ fork point`, provider files as a cache rebuilt on resume) is sound, and it is large. It is a new `history.db`, a new ingest path for every provider, a rebuild of provider JSONL that must be byte-exact, and a change to how resume works.

**Why not now:** layers A and B deliver what phase 3 promised without it; the main dedup targets are the provider files, which belong to the CLIs; and compression already takes the pane store to about 2.7 GB. **When to revisit:** after A, B and the frames, measure again. Build C if (a) forks become common and copied prefixes become a real share, or (b) the provider files dominate what is left and the operator wants them shrunk. A cheaper step for (a) exists before C: a fork's zone could share its parent's frames up to the fork point (copy the frame rows, not the bytes), which falls out of the frames design. **If C is built, it replaces the frame layout rather than sitting next to it**, and the seal and log of layer B carry over unchanged (they are over bytes, not over storage).

## 6. Layer D: a cloud mirror (a separate spec)

History stays on this machine by default (the 2026-10-17 decision). An optional mirror, end-to-end encrypted, is a separate spec, because it brings keys, a key directory, bandwidth, and a privacy question each user must answer. It is also the natural **external anchor** for layer B: publishing checkpoints (not data) to the account's relay lets another install of the same account detect a rewrite on this one. That is small and can come before any data mirror.

## 7. Order, dependencies, and checks

| Step | What | Depends on | Check |
|---|---|---|---|
| A2 | Warden Audit reads the durable file | nothing | restart the app: the view keeps its entries; a month of audit pages quickly |
| A1 | Storage: stored and logical size, duplication, channel copies | nothing (ratio numbers appear with frames) | numbers equal `du` and the integrity-check counts on a copy |
| A3 | Safe zone delete and the Delete history action | the repair (#34), so no damaged row is deleted by surprise | live: a throwaway agent's history deleted; a damaged zone refused and reported; the record appears in Audit |
| B | Seals, the log, `history.verify`, the Integrity view | frames (#49) if its `check` is the seal; otherwise parts | a copy of the store with one flipped byte: verify finds exactly that part |
| C, D | as §5, §6 | measurements after the above | operator's decision |

Each row is its own PR, in this order, with its own tests and a live check in an isolated `task dev` build against a copy of the real store. None writes to the live store without the operator's go-ahead.

## 8. Decisions for the operator

1. **Take the layering?** A then B, C deferred, D separate.
2. **Deletion defaults:** by default delete the shared zones and the per-channel copies, never the CLI's session files unless ticked. Right?
3. **Type-the-name confirmation** for deleting an agent's history. Enough, or also a delay (a "scheduled for deletion in 24 hours" state)?
4. **Redaction:** in scope later as a deliberate action, as §4.4 describes? Or never (history is never edited)?
5. **Audit scope for A2:** jekts only now; add permission grants, tool approvals and account switches as a follow-up?
6. **Integrity wording:** is "integrity check, not proof for an auditor, until anchored" the right claim for the first version?

## 9. Not covered

Searching message text across all agents (a window search is its own design); the provider's own deletion (`cleanupPeriodDays`, seeded to 36,500 days in #5080); the order-of-magnitude storage numbers (the compression spec); repairing the damaged rows (task #34).
