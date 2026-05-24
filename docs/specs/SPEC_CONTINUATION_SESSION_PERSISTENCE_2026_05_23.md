# SPEC: Streamlined Session Continuation — restore full history on agent reopen

**Date:** 2026-05-23
**Status:** Design analysis — needs decision on the storage model before implementation.

---

## Problem statement

User clicks an existing agent (e.g. Maks) and expects to see the **full conversation transcript from where they left off** — same scrollback, same nodes, same tool history. Today they get an empty pane and the CLI silently `--continue`s; the model knows the prior turn but the UI shows nothing.

## Critical scoping decision: session is bound to the AGENT, not the IDENTITY

A session's history follows the **agent definition** (e.g. Maks), not the identity bundle (the OAuth/credential bundle) the agent currently happens to use.

Why this matters: identities are reusable credentials. Tomorrow I might create AgentY that also uses the `default-claude` OAuth bundle. AgentY should NOT inherit Maks' conversation thread. Maks and AgentY are distinct personas with distinct histories that happen to share auth.

In schema terms:
- A session zone is keyed by `definition_id` (the agent), not `identity_id` (the bundle).
- The continuation chain `continueOfBlockId → blockId` is purely a per-agent linkage.
- The "default click on Maks = continue last" lookup uses `WHERE definition_id = '<maks-defid>'`, not identity.

`listrecentsessions` (PR #1000) already exposes `definition_id` on every row — the lookup is a one-line filter.

## Current system map

### Storage layers

| Layer | Keyed by | Holds | Code path |
|---|---|---|---|
| `objects.db` `db_block` | `blockId` (UUID per pane) | Pane metadata (agentId, cmd, env, working dir, sessionid, resume flag) | `agentmux-srv/src/backend/blockstore.rs` |
| `objects.db` `db_agent_definitions` | `definition.id` | Agent identity (name, icon, prompt, defaults) | `agentmux-srv/src/server/agent_handlers.rs` |
| `filestore.db` `db_wave_file` | `(zoneid=blockId, name=output.state.json)` | Full UI snapshot — every node (markdown, tool, user_message, etc.) | `agentmux-srv/src/server/filestore_handlers.rs` |
| `filestore.db` `db_wave_file` | `(zoneid=blockId, name=output)` | Raw stream NDJSON for crash-recovery replay | same |
| CLI's own state (Claude: `~/.claude/`, Codex: `~/.codex/`, etc.) | Provider-specific | The model's actual conversation history | external — opaque to AgentMux |

### Continuation today (PRs #977 + #1000)

1. User clicks a row in the launch modal's **Recent Sessions** list (PR #1000).
2. `launchAgentDefinition` is called with `continueOfInstanceId` + `workDirOverride`.
3. srv creates a **new** agent instance (new `instance_id`) and **new** `db_block` with **new** `blockId`.
4. The spawned CLI gets `--continue` (Claude), `-r` (Codex CLI), etc. — model resumes at the provider level.
5. Frontend pane mounts → `useHistoryPagination` reads `output.state.json` from **its own new blockId's zone**.
6. That zone is empty (it's a fresh block). Falls back to NDJSON replay — also empty. **Pane shows blank.**

### Why the pane is blank

The snapshot is keyed by `blockId`, and a new continuation creates a new `blockId` with no snapshot. The OLD block's snapshot still exists in filestore but the new pane has no pointer to it. The continuation succeeds at the model layer; only the UI surface is empty.

This is the gap.

---

## Design options

Three approaches, ordered by lift.

### Option F — Snapshot copy on continuation (minimal, ~1 PR)

**Mechanism:** when `CreateAgentInstance` runs with `continueOfInstanceId`, the srv copies `<oldBlockId>/output.state.json` (and `output`) into `<newBlockId>/<same name>` BEFORE the pane mounts.

**Pros:**
- One srv-side change + zero frontend change. Existing read path works as-is.
- Each block remains the canonical owner of its own zone. No new pointer / chain concept.
- Reversible — old zones are untouched; if continuation fails we can fall back to fresh.

**Cons:**
- Each continuation duplicates the snapshot (snapshots are typically 100KB–1MB; 10 continuations × Maks = ~10MB extra). Garbage collection of orphaned old zones isn't free.
- Two simultaneous tabs of the same identity (`continueOfInstanceId` from the same row, twice) would each get an independent copy and then diverge — no cross-tab sync.
- Snapshots can grow unbounded over a long-running conversation; copying a 50MB snapshot on every continuation is wasteful.

**Effort:** 1 PR, ~200 lines (srv blockstore + CreateAgentInstance handler + a small test).

### Option B — Linked zones, follow the chain on read (medium, ~1 PR)

**Mechanism:**
- Each block stores `agent:continueOfBlockId` in its `db_block` metadata.
- `useHistoryPagination` reads its own zone first. If empty AND `continueOfBlockId` is set, reads from the linked zone INSTEAD.
- New messages still write to the current block's zone — so over multiple continuations, the history is split across N zones (block 1: original + first 50 turns; block 2: next 30 turns; block 3: …).
- Merging on read: walk the chain backward, concatenate node arrays in chronological order.

**Pros:**
- No data duplication.
- The chain reflects the user's mental model: "this is a continuation of that earlier session".
- Can be implemented incrementally — start with read-from-immediate-parent only; iterate to full chain merge later.

**Cons:**
- More complex read path (chain walk + merge). Each hop is an IPC round-trip.
- "First continuation" pattern: continuing block A from `continueOfInstanceId=A` → new block B. Reading B requires hitting A's zone. Now continue again from B → new block C. Reading C requires hitting B's zone (which is itself empty), which redirects to A. Two hops. The chain length grows with usage.
- Subtle: the new block's NEW nodes go to its OWN zone. So zone A has nodes 1-50, zone B has nodes 51-80, zone C has nodes 81-90. To show the full history, you merge all three. Performance becomes a function of continuation depth.

**Effort:** 1 PR, ~400 lines (srv reads + frontend read merge + tests).

### Option E — Agent-anchored sessions (structural, ~3 PRs)

**Mechanism:**
- Each **agent definition** owns ONE active session zone: `filestore.db` zone `agent:<definitionId>:current`.
- Blocks become **views over** the agent's session zone, not owners of their own.
- Continuing an agent = open a new block pointing at the same zone. Cross-tab sync becomes natural (both tabs read+write the same zone; reactive store fans the changes out).
- "+ New session" action: archive the current zone to `agent:<defId>:archive:<timestamp>` (or just snapshot it), then clear and start fresh. Old archived sessions remain queryable as "previous Maks conversations".
- Two agents that share an identity (e.g. Maks and a future AgentY both on `default-claude`) keep entirely separate session zones. The identity bundle is just an auth provider; it has no session role.

**Pros:**
- Matches the user's mental model: an agent is a persona that remembers its conversation history; a block is a window into it.
- Cross-tab consistency for free (e.g. two side-by-side panes of Maks see each other's tokens stream in real-time).
- "Recent sessions" becomes a list of Maks' archived conversations, with the active one always at the top.
- No data duplication, no chain walking.
- Switching the identity bundle on an agent (e.g. rotating the OAuth credential under Maks) doesn't touch session history — the identity change is orthogonal.

**Cons:**
- Storage schema change: blocks shift from "owns a zone" to "references a zone". Migration story needed for existing blocks (one-time backfill: each block becomes its own archived session under its agent definition, keyed by its old blockId).
- Cross-tab write coordination: two tabs writing to the same zone concurrently. Needs either a single-writer (one tab is "the writer", others are read-only mirrors via a pub-sub) OR CRDT-style merge.
- ~3 PRs of work: (1) agent-zone schema, (2) frontend re-read paths, (3) cross-tab write coordination + migration.

**Effort:** 3 PRs, ~1500 lines spread across srv + frontend + migration.

---

## Recommendation

**Ship Option F now**, treat it as the streamlined path the user is asking about. It delivers the visible win (full transcript on continuation) with minimum risk and is reversible. Plan Option E as the structural follow-up when cross-tab consistency or sub-100MB-budget snapshots become priorities.

**Option B is a tar pit.** The chain-walk read path will look fine at chain length 2 but ages poorly — by continuation #10 you're doing 10 IPC round-trips and merging fragmented node arrays in the renderer hot path. Not worth the implementation cost vs. the cleaner Option E.

## UX side of "streamlined"

Independent of storage choice:

1. **Default click action** on an agent card with prior sessions = **continue last session** (not the launch modal's full picker). Surface "+ New" as a secondary affordance, and "Recent sessions…" as a tertiary one (the full picker we already have).
2. Implementation: in `AgentPicker.tsx`'s `handleSelect`, look up the agent's prior rows via `listrecentsessions` filtered by **`definition_id`** (the agent), NOT `identity_id`. If any rows exist, default to `handleReattach(rows[0])` instead of `openLaunchModal`. Modifier keys (Shift / Ctrl) can override to force the modal.
3. The pane title should reflect continuation: "Maks · continued from 2 hours ago" or similar.

## Decision needed

- (a) Ship Option F now (1 PR) and revisit later, OR
- (b) Ship Option E now (3 PRs, more disruption) for the structural fix.

The UX changes (default-continue button) apply equally to both — they can be a small extra PR in either path.
