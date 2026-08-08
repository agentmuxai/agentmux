# SPEC: Durable, location-consistent, transparent native memory

**Date:** 2026-08-07
**Status:** Proposed
**Depends on:** `docs/reports/REPORT_ARMORY_STASH_MEMORY_SYNC_STATUS_2026_08_07.md`
(history + current-state analysis; read that first for full context — this
doc only restates what's load-bearing for the design below).

---

## 0. Decision (resolves that report's §7 open questions)

> "durability, location-consistency. we dont care how, we want it
> transparent, the user doesnt know about claude's system paths, he sees all
> the memory claude writes into his stash/armory"

Resolved:
- **Not content-sync.** Claude still owns *what* gets written; nothing here
  reconciles native memory against Bundle content, and the original
  "facts vs. rules" invariant (prior report §4) stands unchanged.
- **Durability + location-consistency, mechanism unspecified** — my call to
  make, outcome is what's fixed: no data loss, no silent "this agent has no
  memory" when it actually does, and zero user-visible awareness of
  `CLAUDE_CONFIG_DIR`/cwd-hash mechanics.
- **Surfaces in Stash, not Armory.** Stash's existing "Memory" tab
  (`AgentNativeMemoryModal`) is per-agent and already the right place per the
  architecture's own principle (per-agent data lives outside Armory).
  Duplicating it into Armory would deepen the three-way "Memory" naming
  collision the prior report documents. Read "stash/armory" in the ask as
  "the app's management UI," not a literal requirement for two surfaces —
  **flagging this explicitly since it's the one place I'm resolving an
  ambiguity rather than just picking a mechanism.**

---

## 1. Root cause, precisely

`agentmux-srv/src/server/native_memory_handlers.rs`'s three RPC handlers
(`agent:memory:list/read_file/write_file` — Stash's Memory tab) already key
off `agent.id`, the stable, global `AgentDefinition` id
(`wstore.agent_def_get(&cmd.agent_id)`, lines 306-309/410-413/470-473) — this
part is already channel-independent, no fix needed.

What's **not** stable is the *live filesystem path* they compute fresh on
every call: `memory_dir_for_cwd(config_dir, agent.working_directory)`. Both
inputs are legitimately channel-relative by design:

- `agent.working_directory` — for a live (not-yet-persisted) agent, resolved
  via `memory_dir_from_registry` as `source_agents_base.join(working_dir)` —
  **`source_agents_base` is explicitly "the channel/dev instance it lives
  in"** (the function's own doc comment, line 145). Each channel has its own
  agents-base directory by design (`CLAUDE.md`'s per-build-channel isolation
  section) — a fresh local build's filesystem *cannot* have another build's
  directory tree, structurally.
- `config_dir` (`CLAUDE_CONFIG_DIR`) — resolved per identity
  (`claude_config_dir_for_identity`), stable *for a given identity binding*,
  but an identity's own root (`~/.agentmux/shared/identities/<id>/claude`)
  still sits under a shared-dir tree that can itself be relocated between
  channels (`AGENTMUX_SHARED_DIR`).

So: **the same logical agent (same `AgentDefinition.id`), opened from two
different channels/instances, computes two different on-disk memory paths by
construction.** This isn't a bug in the resolution logic — it's a direct,
correct consequence of per-channel filesystem isolation applied to a path
formula that depends on a channel-relative input. No amount of fixing the
*formula* changes this; the fix has to stop depending on the live path being
the only copy of the truth.

## 2. Design

### 2.1 A durable mirror, keyed by the one thing that's already stable

Add `db_agent_native_memory` (global-scoped store, alongside `db_bundles`/
`db_accounts`/etc.), keyed by `(agent_id, filename)` where `agent_id` is the
same `AgentDefinition.id` the RPC handlers already resolve to:

```
db_agent_native_memory(
    agent_id      TEXT NOT NULL,   -- AgentDefinition.id (FK, ON DELETE CASCADE)
    filename      TEXT NOT NULL,   -- e.g. "MEMORY.md", "user_role.md"
    content       TEXT NOT NULL,
    metadata_type TEXT,            -- cached frontmatter `metadata.type`, avoids re-parsing on every list
    size_bytes    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,-- ms epoch, last time THIS row was written
    last_seen_path TEXT,           -- debugging aid: the live FS path this content was last read from
    PRIMARY KEY (agent_id, filename)
)
```

Migration `m0020_agent_native_memory_mirror.rs`, `MigrationScope::Global`
(matches `db_bundles`/`db_accounts` — these are cross-channel, not per-instance
tables), following `m0019`'s pattern (`Migration` trait, `id()`/`scope()`/
`description()`/`up()`).

### 2.2 Read path: merge live FS + durable mirror, write through on every read

Both `agent:memory:list` and `agent:memory:read_file` currently only touch
the live FS. Change them to:

1. Read whatever's live on disk (unchanged — same `read_dir`/`File::open`
   calls, same symlink/size-cap/TOCTOU handling already in place).
2. **Write through**: upsert every live file's current content into
   `db_agent_native_memory` keyed by `(agent.id, filename)` — a plain
   `INSERT ... ON CONFLICT (agent_id, filename) DO UPDATE`, cheap, no new
   round trip beyond one extra local SQLite write already on the same
   request.
3. **Merge for the response**: union the live-FS filename set with the
   mirror's filename set for this `agent.id`. For a name present in both,
   serve the live-FS version (it's the freshest — Claude may have written
   moments ago, before step 2 even ran). For a name present **only** in the
   mirror (i.e. this channel's live FS doesn't have it — a different
   channel's write, or the live folder was wiped), serve the mirrored
   content transparently, with no distinguishing UI treatment. The user
   never sees "not found on this channel" — they see the file.

This means: the *very first* time any channel/instance opens Stash → Memory
for a given agent, this mirror starts filling in. From that point forward,
every subsequent open (even from a different channel) sees the union.
`write_file` (the Stash UI's own Save button) does the same upsert on its
write path — trivial, since it's already writing content it has in hand.

### 2.3 What this does and doesn't cover

- **Covers:** durability across channel/instance switches for any file that
  has been *viewed* (list or read) at least once from any channel — which
  in practice means every file, since listing already happens every time the
  Memory tab opens.
- **Does not cover:** a fact Claude writes autonomously, in a session that
  is *never reopened* in the Stash Memory tab before that channel's
  filesystem is wiped/replaced. This is a real, accepted gap under "we don't
  care how" — closing it fully would need either a live filesystem watcher
  on the memory directory (meaningfully more complexity: a watcher process,
  debouncing, handling watch-target deletion) or hooking into the
  persistent-controller's own I/O (deeper coupling to the provider process
  lifecycle). Not proposed here; flagged as a known residual risk, and an
  easy follow-up if it turns out to matter in practice (§4).

### 2.4 Why not just relocate the folder instead

Covered in the prior report's §5 in full; restated briefly because it's the
literal first half of the ask. `CLAUDE_CONFIG_DIR` (the root) is already
redirectable and already identity-scoped. The `projects/<cwd-hash>/memory/`
structure under it is a fixed Claude Code CLI convention — no flag or env
var decouples memory from the cwd-encoding, and there's no known symlink/
relocate trick in this codebase's own prior research. It doesn't matter:
AgentMux already computes and can always directly reach the exact path
Claude Code itself will use (that's the entire mechanism §1's handlers rely
on today) — there was never a need to relocate anything to get read/write
access. The actual gap was durability across channels, which §2.1–2.2 solves
independently of where the live folder happens to sit.

## 3. Rollout

- **Backfill:** none needed. The mirror starts empty and fills in
  organically the first time each agent's Memory tab is opened post-deploy —
  no migration needs to walk existing live memory folders (there's no
  reliable way to enumerate "every agent that ever existed across every
  channel" from a fresh install anyway; organic backfill-on-read is the only
  option that's actually complete over time).
- **Agent deletion:** no FK, no cascade. `db_agent_definitions` lives in a
  separate SQLite file from this table (duplicated into both objects.db and
  the shared store.db — §2.1), and SQLite can't enforce a foreign key across
  database files. Orphaned rows after an agent's deleted are inert rather
  than actively cleaned up: every RPC handler that would read this table
  first resolves and 404s on `agent_def_get`, so a dangling mirror row is
  never reached, let alone surfaced. (Updated from the original "ON DELETE
  CASCADE" draft above once the cross-database-file constraint surfaced
  during implementation — reagent P2 on PR #2459.)
- **Identity rebinding** (an agent's bound account changes, moving its
  `CLAUDE_CONFIG_DIR` root): the live FS path changes, but the mirror is
  keyed by `agent_id` alone, not by path — old content stays visible
  (merged in) even though the live folder it originally came from is now
  unreachable under the new identity. This is correct per the "user sees
  everything Claude has ever written for this agent" goal, though worth
  flagging as a case where the merge does real work, not just an unused
  safety net.
- **No behavior change for the App-API path** (`memory_dir_for_agent`, used
  by `bundle.self.get`-adjacent callers under a *slug*, not the RPC handlers'
  UUID). Out of scope for this pass — it's a programmatic surface, not the
  Stash UI the ask is about. Worth a follow-up if a caller there turns out
  to need the same guarantee.

## 4. Non-goals / explicit follow-ups if this turns out to be insufficient

- Real-time capture of Claude's autonomous writes between tab opens (§2.3).
- Any UI signal distinguishing "live" vs. "mirrored-only" content — the goal
  is the user never has to think about the difference, so deliberately not
  building an affordance that would surface it.
- Extending the same merge to `memory_dir_for_agent`'s App-API consumers.
- Anything about Bundle/native-memory content reconciliation (explicitly
  out of scope per §0).

## 5. Test plan

- Unit: `db_agent_native_memory` upsert-on-read behaves correctly for (a) a
  brand-new file, (b) a re-read of an unchanged file (no spurious
  `updated_at` churn beyond what content-hash comparison would avoid — or
  accept the churn if simplicity wins; call this out as an implementation
  judgment call, not load-bearing), (c) a file whose live-FS copy is deleted
  after being mirrored once (list still returns it, read still returns its
  last-known content).
- Unit: merge logic — live-FS-only file, mirror-only file, and a file present
  in both with different content (live wins) all produce the expected
  `list`/`read_file` results.
- Integration: simulate two "channels" against the same `agent.id` by
  pointing `memory_dir_for_cwd`'s inputs at two different temp directories in
  the same test — write via channel A, list via channel B, confirm channel
  B's list includes channel A's file with correct content, with no
  channel-awareness in the response shape.
- Manual: create an agent, write a memory file via Stash, then simulate a
  channel switch (e.g. open the same agent from a different local `task
  package` build) and confirm the file is still visible in that build's
  Stash Memory tab without any error state or "not found."
