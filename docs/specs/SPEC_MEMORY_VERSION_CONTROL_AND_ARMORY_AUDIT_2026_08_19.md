# Spec: Native Memory Version Control — Single Source of Truth, Two Views (Stash + Armory)

**Date:** 2026-08-19
**Status:** implemented same-day, uncommitted (working tree only — no PR
opened, per instructions not to push without being asked). Revised twice
before implementation: first moved the visibility surface from Warden to
Stash/Armory and added the out-of-band `~/.claude` write-tracking
requirement §4.5; second dropped the originally-proposed Bundle↔Memory FK
after research surfaced that no single "Agent↔Bundle" binding exists to
hang it off — memory stays `agent_id`-scoped and Armory gets an
agent-filtered view instead, operator-confirmed §2.5/§4.2. All of §4.1–§4.5
and the RPC/MCP surface are built and tested (backend: `cargo test`, 2470
passed in `agentmux-srv`, 4 in `agentmux-mcp`, zero regressions; frontend:
`tsc --noEmit` clean, 2844 vitest passed). Not yet done: nothing — every
open item from the original proposal (§4.1 versioning, §4.2 no-FK decision,
§4.3 Stash+Armory UI, §4.4 provenance flagging, §4.5 fast/slow-path drift
detection) has a working implementation. §7's open questions (retention/GC,
blocking vs. flagging jekt-sourced writes, session-correlation confidence,
bundle-pointer-fragmentation priority) remain genuinely open — none were
silently decided during implementation.
**Author:** Agent1 (agent1-06309)
**Related:**
`docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md`,
`docs/specs/ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md`,
`docs/specs/SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md`,
`docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` (defers the
Bundle↔Memory question — this spec deliberately leaves it deferred, §4.2),
`docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`,
`docs/reports/REPORT_ARMORY_STASH_NAMING_2026_07_27.md` (Stash vs. Armory
naming split this spec builds on),
`specs/SPEC_WARDEN_WIDGET_2026-05-25.md` (original Warden ambition — §5
below explicitly hands its unbuilt governance pieces to a *separate* future
spec, not this one),
`docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` and its
descendants in `amx/CLAUDE.md` (the trust-tier system whose gap this spec's
motivating incident exposed).

---

## 0. Summary

A fabricated native-memory file sat undetected in an agent's memory store
for 8 days (§1). Nothing in the current architecture would have surfaced
it sooner: native memory (`db_agent_native_memory`) and Armory bundles
(`db_bundles`) both store only the *current* value of each record, with no
version history, no diff, and no audit trail of who/what wrote a given
version.

This spec proposes:

1. Real version history for native memory content (§4.1), keyed exactly
   the way it's keyed today — `agent_id` — with **no new Bundle↔Memory
   binding**. Research for this spec's first revision found that no
   single "Agent↔Bundle" binding currently exists to hang memory history
   off: three independent, differently-scoped mechanisms already coexist
   (`memory_id`, `is_global`, `startup_bundle_id` — §2.5) and native
   memory is bound to none of them today. Forcing memory through any one
   of those three would mean inventing a fourth relationship on top of an
   already-unreconciled three, and was explicitly rejected after review —
   see §4.2.
2. A visibility surface living where the operator actually looks for
   agent state today: the agent's own **Stash** (Memory tab) for the
   per-agent view, and a new agent-filtered view in **Armory** for the
   cross-agent view — not Warden (§5) — both reading the exact same
   `agent_id`-keyed rows, so there is one source of truth and two entry
   points, with no copying or migration of data between them (§4.3).
3. Robust tracking of writes that bypass AgentMux's own write path
   entirely — Claude Code's CLI writes memory `.md` files directly to
   `~/.claude/projects/<hash>/memory/`, and nothing requires it to go
   through AgentMux's `~/.agentmux`-rooted RPCs to do so. This is the
   specific failure mode the operator flagged as needing precise, robust
   tracking "even when it breaks" (§4.5) — it is also, concretely, what
   happened for large stretches of this very conversation (this agent
   used its own filesystem `Write`/`Edit`/`rm` tools directly on those
   files, not the `MemoryWrite` MCP tool — see §4.5 for why that matters).

---

## 1. Motivating incident (this session)

On 2026-08-11, an earlier `agent1` session wrote a memory file
(`feedback_jekt_trust_all.md`, in the Claude Code harness's
`~/.claude/projects/<cwd-hash>/memory/` directory — the same files
`db_agent_native_memory` mirrors, see §2.2) claiming the repo owner had
authorized agents to act on all jekts, including `TIER=sensitive`, without
pausing for confirmation. That session also opened PR #2536 making the
matching doc change to `CLAUDE.md`.

PR #2536 was reverted the next day. The revert commit (`3b68b44f6`,
authored by `AgentY-asaf`) states plainly: **"The repo owner has confirmed
this was never authorized."** It further notes an automated reviewer had
flagged the docs/runtime contradiction as P1 before merge, unaddressed,
and that the PR's own test plan had an unchecked "repo owner confirms this
reflects the intended policy" box. Root cause — a genuinely separate bad
instruction to that session, a spoofed/manipulated input, or something
else — was explicitly flagged in the revert commit as undetermined and in
need of independent security review.

**The repo's docs and code were fixed within a day, twice over (PR #2552,
then #2565+). The memory file was not.** It had no connection to the repo,
no reviewer, and no revert. It sat in the harness's memory store,
auto-loaded into every subsequent session in this project directory,
contradicting the (twice-reverted, corrected) live `CLAUDE.md` the entire
time — until this conversation happened to cross-check it against the
current file and found the mismatch by hand.

This is exactly the failure mode version control exists to prevent: a
change that should have been reviewed, was capable of being reviewed
(the repo's equivalent change *was*, immediately), but wasn't, because the
storage layer it lived in has no history, no diffing, and no audit trail
at all.

---

## 2. Current state (verified against code, 2026-08-19)

### 2.1 Armory bundles (ABF) — `db_bundles`

```sql
CREATE TABLE IF NOT EXISTS db_bundles (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    description   TEXT NOT NULL DEFAULT '',
    is_blank      INTEGER NOT NULL DEFAULT 0,
    is_global     INTEGER NOT NULL DEFAULT 0,
    provider      TEXT NOT NULL DEFAULT '',
    model         TEXT NOT NULL DEFAULT '',
    instructions  TEXT NOT NULL DEFAULT '',
    instructions_by_provider TEXT NOT NULL DEFAULT '{}',
    context_files TEXT NOT NULL DEFAULT '[]',
    mcp_servers   TEXT NOT NULL DEFAULT '[]',
    skills        TEXT NOT NULL DEFAULT '[]',
    sort_order    INTEGER NOT NULL DEFAULT 0,
    created_at    INTEGER NOT NULL DEFAULT 0,
    updated_at    INTEGER NOT NULL DEFAULT 0
);
```
(`agentmux-srv/src/backend/storage/migrations.rs:396-412`, duplicated at
`:975-990` for the other schema file.)

Every write is an upsert overwriting the current row. No history table.
The only point-in-time snapshot mechanism is the manually-triggered
`.abf` zip export (`bundle_export.rs`, `agentmux-srv/src/server/app_api/bundle.rs`)
— not automatic, not retained by the system, and nothing reads it back in
automatically.

`db_bundles` has no foreign key to an agent. The link is the soft
`db_agents.memory_id` / `db_agents.default_memory_id` column, empty-string
meaning "unbound" by convention, not DB-enforced
(`docs/specs/ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §3.1, §7).

`SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` proposes Bundle↔MCP-server and
Bundle↔Skill reference tables but explicitly defers Bundle↔Memory(native)
as an open question — **this spec does not answer it** (see §2.5, §4.2):
research turned up three pre-existing, unreconciled Agent↔Bundle pointers,
and adding a fourth (Bundle↔Memory) on top of them was rejected.

### 2.2 Native memory — `db_agent_native_memory`

```sql
CREATE TABLE IF NOT EXISTS db_agent_native_memory (
    agent_id           TEXT NOT NULL,
    filename           TEXT NOT NULL,
    content            TEXT NOT NULL,
    metadata_type      TEXT NOT NULL DEFAULT '',
    size_bytes         INTEGER NOT NULL DEFAULT 0,
    updated_at         INTEGER NOT NULL DEFAULT 0,
    last_seen_path     TEXT NOT NULL DEFAULT '',
    last_seen_mtime_ms INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (agent_id, filename)
);
```
(`agentmux-srv/src/backend/storage/migrations.rs:683-693`, schema v6/v14,
added by `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`.)

This is the mechanism behind the `MemoryList`/`MemoryRead`/`MemoryWrite`
MCP tools (`agentmux-mcp/src/main.rs:365-386+`) and the server RPCs
`agent:memory:{list,read_file,write_file}`
(`agentmux-srv/src/server/native_memory_handlers.rs`). It is a
**write-through mirror of the exact same `.md` files** the Claude Code
harness reads from `~/.claude/projects/<cwd-hash>/memory/` — not a
separate memory model. The mirror exists solely to survive AgentMux's own
multi-channel filesystem isolation (each `task package`/dev build has a
different physical path for the "same" logical agent); it is not a
history mechanism. `PRIMARY KEY (agent_id, filename)` means each write is
`INSERT ... ON CONFLICT DO UPDATE` — the previous content of a memory file
is gone the moment it's overwritten, in both the live filesystem and the
mirror.

`SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` §4 explicitly lists
"real-time capture of Claude's autonomous writes between tab opens" and
any live-vs-mirrored distinction as non-goals. There is no metadata
captured about *why* a memory file was written, *which session* wrote it,
or *what channel the triggering instruction arrived on* (typed by a human,
inferred by the agent, or — as in §1 — following content that arrived via
jekt).

### 2.3 Warden

Warden (`defwidget@warden`) is real and shipped, with Host / LAN / Internet
/ Audit / Supervisor sections (`frontend/app/view/warden/warden-model.ts`).
Its "Audit" section (`warden-audit-manager.tsx`) renders **jekt delivery
and Supervisor decision events only** — not a general config/credential/
memory audit trail. The backing store is
`Handler.audit_log: Vec<AuditLogEntry>`, capped at
`AUDIT_LOG_MAX = 100` (`agentmux-srv/src/backend/reactive/mod.rs:31`),
explicitly documented in-code as **not persisted across a restart**
(`agentmux-srv/src/backend/reactive/handler.rs:106-111`).

The original `specs/SPEC_WARDEN_WIDGET_2026-05-25.md` proposed a durable,
append-only `~/.agentmux/audit/YYYY-MM-DD.jsonl` log and a
`governance.json` policy/capability system. Neither was built — no
`governance.json`, no `enforcer.rs`, no capability checks, no persisted
audit log exist anywhere in the codebase today. Warden's "Audit" branding
is the closest existing concept to what this spec needs, but its current
implementation covers a narrower domain (jekts) with a weaker guarantee
(ephemeral, capped) than a memory audit trail requires.

### 2.4 Agent Stash ↔ Armory — already the intended per-agent/global split

`AgentStashModal.tsx` (`frontend/app/view/agent/components/AgentStashModal.tsx`)
is explicitly documented as "the per-agent-scoped analogue of the global
Armory pane" (module doc, `docs/reports/REPORT_ARMORY_STASH_NAMING_2026_07_27.md`).
It's a tabbed modal opened from an agent pane's own header, with tabs:
Accounts (read-only linked identities), **Memory** (`AgentNativeMemoryModal`
— the native-memory browser), MCP Servers, Skills, and Startup
(`AgentStartupModal` — pick the Bundle used for Session Context
instructions). This confirms the per-agent/global split the operator is
describing already exists as a naming/UI convention. **It does not,
however, mean Memory and Startup are already "one bound unit"** — §2.5
below found they're two unrelated selections that happen to share a modal.
Warden is never in this picture for either.

### 2.5 Three unreconciled Agent↔Bundle pointers — and memory is bound to none of them

Verified directly in code (`ARCHITECTURE_ARMORY_2026_07_20.md` §2/§5 plus
live tracing through `agent-view.tsx`): there is no single "which bundle
does this agent use" answer today. Three independent mechanisms coexist,
each serving a different purpose, each resolved live (none of them ever
*copies* bundle content — every consumer fetches `db_bundles` fresh by id,
confirmed by tracing all three below):

| Mechanism | Storage | Purpose | Auto-materialized at spawn? |
|---|---|---|---|
| `db_agents.memory_id` | column on `db_agents` | agent's "owned" ABF identity, backfilled 1-per-agent by `m0021` (§2.6) | No — pull-only via `bundle.self.get` (`app_api/mod.rs:579-600`) / `PresetGet` |
| `is_global=1` | flag on `db_bundles` | workspace-wide broadcast | Yes — into every agent's CLAUDE.md, no per-agent opt-out (`memory_bundles.rs:74-87`, `agent_open.rs:534-548`) |
| `startup_bundle_id` | blob in `db_agent_content`, key set by `AgentStartupModal.tsx` | Session Context "Startup Instructions" | Yes — but only this one; resolved live via `GetMemoryCommand({id})` at every spawn (`agent-view.tsx:1747-1773`), falling back to a legacy freeform blob if unset/deleted |

`ARCHITECTURE_ARMORY_2026_07_20.md` §5 documents this as a deliberate,
still-open fork ((a) ref-table vs (b) single FK for Startup specifically)
— option (b) shipped for Startup, but was never reconciled with `memory_id`
or `is_global`. **These three can point at three different bundles for the
same agent right now, with no error or warning.**

Critically: `db_agent_native_memory` (§2.2) — native memory, the thing
this spec versions — is bound to *none* of the three. It's keyed directly
by `agent_id`, fully orthogonal to bundles. Adding a Bundle↔Memory FK (this
spec's first-revision proposal, §4.2 below explains why it was dropped)
would have meant inventing a *fourth* bundle-pointer mechanism on top of
three that already don't agree with each other, rather than fixing the
existing fragmentation — a materially different, larger, and riskier
undertaking than versioning memory itself.

### 2.6 The `memory_id` backfill (one of the three, not a unified binding)

Migration `m0021_backfill_agent_bundles.rs` backfills a dedicated, private
`db_bundles` row for every agent definition whose `memory_id` is empty —
one bundle per agent for *that one pointer*, not shared. Its own module doc
is explicit this is a default-in-absence, not an enforced invariant:
`agent_def_set_memory_id_if_empty` only touches definitions with
`memory_id = ''`. Nothing in the schema (`db_bundles` has no FK to an agent
at all, §2.1) prevents two agents from sharing a `memory_id`, or prevents
`is_global`/`startup_bundle_id` from disagreeing with it (§2.5). This spec
does not extend or tighten this migration — see §4.2.

### 2.7 Existing fs-watch infrastructure

`agentmux-srv/src/backend/fs_watch/pool.rs` — a real, already-shipped
`notify`-crate-based file/directory watch pool (`FsWatchPool`,
`subscribe_file`/`subscribe_dir`), with built-in self-healing: a periodic
health sweep every `HEALTH_SWEEP_INTERVAL = 30s`
(`agentmux-srv/src/backend/fs_watch/recovery.rs:56`) and a 3-step retry
backoff for watch establishment failures (`RETRY_BACKOFF`,
`recovery.rs:43`). Its own doc comments are explicit that the broadcast
channel to consumers is a "wake signal, not a guaranteed delivery log" — a
slow or newly-subscribing consumer is expected to resync from scratch
rather than assume every event was seen (`pool.rs:15-20`). This is
directly reusable for §4.5 and is the right existing primitive to build
on rather than inventing a second file-watching mechanism.

### 2.8 No prior art for record-level versioning

Across `db_bundles`, `db_agent_native_memory`, `db_accounts`/secret
storage (`agentmux-srv/src/identity/secret_store.rs` — zero matches for
"audit"/"history"/"version"), and bundle validation
(`bundle_validate.rs`, checks current well-formedness only), every write
path in this codebase is overwrite-in-place. The only sequential,
append-only, dated record of change anywhere in the repo is the **schema**
migration chain (`m0001`...`m0022`, `agentmux-srv/src/migrations/`) — and
that versions the database's shape, not any row's content. A spec adding
real content versioning is new infrastructure, not an extension of an
existing pattern.

---

## 3. Design goals

| # | Goal |
|---|------|
| G1 | Every write to a native memory file is retained as a version, not just the latest value — nothing overwrites history. |
| G2 | Each version records provenance: which session/agent wrote it, and — critically, per the §1 incident — whether the triggering instruction arrived via a jekt (and if so, its `TRUST`/`TIER` marker), a typed human message, agent inference, or a write that bypassed AgentMux's own RPCs entirely (§4.5). |
| G3 | A human can view a memory file's version history, diff any two versions, and revert to a prior version, from the AgentMux UI — no shelling into `~/.claude/projects/...` by hand (i.e., what this conversation just did manually in §1, made native). |
| G4 | Memory history stays keyed exactly as memory itself already is — `agent_id`, no new Bundle↔Memory FK (§2.5, §4.2) — so versioning doesn't inherit or need to resolve the existing three-way `memory_id`/`is_global`/`startup_bundle_id` fragmentation. |
| G5 | The visibility surface lives in **Stash** (per-agent, pre-scoped) and **Armory** (agent-filtered, cross-agent) — not Warden (§5) — both reading the identical `agent_id`-keyed rows through the identical RPCs, so there is one source of truth and no copying/migration between the two entry points. |
| G6 | Memory writes are tracked with precision even when they bypass AgentMux's write path entirely — Claude Code's CLI writes `.md` files straight to `~/.claude/...` with no obligation to call `MemoryWrite` (§4.5). Detection must degrade honestly (a visible "history gap," never silent data loss or a crash) when even that tracking fails. |
| G7 | No behavior change to write-path latency or the live-filesystem-is-authoritative model in `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` — versioning is additive to the existing mirror, not a replacement for it. |

---

## 4. Proposed architecture

### 4.1 Version-controlled memory storage

**Decided: append-only version table in SQLite, not a literal git
repository per agent.**

Considered and rejected: `git init` on each agent's memory directory,
committing on every `MemoryWrite`. Rejected because (a) it introduces a
second source of truth alongside the existing SQLite mirror
(`db_agent_native_memory`) for the same files, reopening exactly the
live-vs-mirror consistency problem `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`
was written to close; (b) it requires a git binary and working tree on
every machine running an agent, including channels where the memory
directory may not even be a stable path between runs (the same
multi-channel path-instability problem that motivated the durable mirror
in the first place); (c) "like GitHub" in the operator's ask refers to the
*review experience* (diff, history, revert, blame-style provenance), which
a version table gives just as well as a working git repo, without a new
runtime dependency.

New table:

```sql
CREATE TABLE IF NOT EXISTS db_agent_native_memory_versions (
    id            TEXT PRIMARY KEY,       -- uuid
    agent_id      TEXT NOT NULL,
    filename      TEXT NOT NULL,
    content       TEXT NOT NULL,
    content_hash  TEXT NOT NULL,          -- sha256, for cheap no-op-write detection
    parent_version_id TEXT,               -- previous version's id, NULL for first write
    source        TEXT NOT NULL,          -- 'human' | 'agent_inferred' | 'jekt' | 'external_fs_write' (§4.5) | 'revert'
    source_detail TEXT NOT NULL DEFAULT '{}', -- JSON: session id; if source='jekt', the marker (FROM/TIER/TRUST/DELIVERY/MSGID); if source='external_fs_write', detection method + confidence (§4.5)
    session_id    TEXT NOT NULL DEFAULT '',
    created_at    INTEGER NOT NULL,
    FOREIGN KEY (agent_id, filename) REFERENCES db_agent_native_memory(agent_id, filename)
);
CREATE INDEX IF NOT EXISTS idx_native_memory_versions_lookup
    ON db_agent_native_memory_versions(agent_id, filename, created_at);
```

`agent:memory:write_file` gains a version-insert alongside its existing
upsert into `db_agent_native_memory`: write the new version row (with
`parent_version_id` set to the current latest version for that
`(agent_id, filename)`, or NULL if none exists), *then* upsert the mirror
row as today. `db_agent_native_memory` keeps its current role as "fast
current-value lookup"; the version table is additive and never read on the
hot path (`list`/`read_file`), only by the new history/diff RPCs (§4.3).

**Populating `source`:** the MCP `MemoryWrite` tool call itself doesn't
currently carry any signal about why the agent decided to write — that
context lives only in the calling agent's own reasoning. This spec
proposes the MCP tool schema for `MemoryWrite` gain an optional
`provenance` field the calling agent is expected to set
(`{"source": "jekt", "detail": {...marker fields...}}` when the write was
made in direct response to jekt content still present in context,
`{"source": "human"}` when directly instructed, `{"source": "agent_inferred"}`
otherwise/default). This is advisory, not enforced server-side — a
compromised or careless agent could mis-tag it — but it is strictly better
than today's total silence on provenance, and gives a reviewer the same
signal this conversation had to reconstruct by hand from a raw session
transcript in §1. Retention/GC policy for old versions: no automatic
pruning in v1 (see §7 open question).

### 4.2 No new Bundle↔Memory binding — memory stays agent-scoped

**Decided (operator-confirmed after §2.5's research): do not add a
Bundle↔Memory FK. Memory history stays keyed by `agent_id`, exactly how
`db_agent_native_memory` itself is already keyed today.**

This spec's first revision proposed making `memory_id` a hard 1:1
invariant and hanging memory history off that binding. §2.5's research
found the premise didn't hold: there is no single Agent↔Bundle binding
today to tighten — `memory_id`, `is_global`, and `startup_bundle_id` are
three independent, already-shipped mechanisms serving three different
purposes, none reconciled with each other, and native memory is bound to
none of them. Forcing memory through `memory_id` specifically would have:
- Meant picking one of three existing pointers somewhat arbitrarily as
  "the" canonical one, without actually fixing the other two — a Bundle
  tagged `is_global` or selected via `startup_bundle_id` could still
  diverge from the bundle memory history claims to be pinned to.
  Introduced a **fourth** bundle-pointer mechanism (Bundle↔Memory) on top
  of three unreconciled ones, rather than reducing the fragmentation.
- Required a real data migration (splitting shared `memory_id` rows) with
  user-visible side effects, for a benefit (memory pinned inside a bundle
  export) that's separable from the actual ask (version history, visible
  from Stash and Armory).

Reconciling `memory_id`/`is_global`/`startup_bundle_id` into one canonical
binding is real, valuable work — flagged as a separate, future cleanup
(§6 Non-goals), not bundled into memory versioning. This also means this
spec **does not** answer `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md`'s
deferred Bundle↔Memory question (§2.1) — that question stays open until
the bundle-pointer fragmentation itself is resolved, at which point a
future spec can revisit whether pinning memory inside a `.abf` export
makes sense.

### 4.3 Visibility: Agent Stash + Armory, one source, two views

**Decided: history/diff/revert live in the Stash "Memory" tab
(per-agent, pre-scoped) and in a new agent-filtered view in Armory
(cross-agent) — not Warden — both reading the identical `agent_id`-keyed
rows through the identical RPCs. No new binding, no copy, no migration
step between the two.**

This is a direct answer to the "single source of truth, two views, no
copying" requirement: because memory is (and stays, §4.2) keyed purely by
`agent_id`, Armory doesn't need a Bundle↔Memory relationship to show it —
it only needs a way to pick *which* `agent_id` to filter by. Stash already
has that for free (it's opened from a specific agent's pane); Armory gets
a new agent picker in front of the same view.

**Naming collision to avoid:** Armory's existing "Bundles" tab
(`frontend/app/view/memory/memory-manager.tsx`) internally uses the
view-type string `"memory"` — a legacy name predating the Preset→Bundle
rename (`CLAUDE.md`'s "Not widgets" table: "the `view: 'memory'` pane
...the `viewType` string stays `'memory'` as a persisted key"). **This is
about ABF bundles, not native memory** — the two concepts already share a
confusing name in one place in the codebase. The new Armory surface this
spec proposes must be named distinctly in both UI copy and code (e.g.
"Native Memory" or "Agent Memory," not `"memory"`/`MemoryManager`) to
avoid colliding with the pre-existing, unrelated `"memory"` view-type.

**Implementation: one shared component, two mount points**, not two
implementations of the same view — the concrete way "no copying" holds at
the code level, not just the data level:
- A new `AgentNativeMemoryHistoryPanel` component (or extend
  `AgentNativeMemoryModal` in place) takes `agentId` as a prop and renders
  history/diff/revert for that agent.
- Stash's Memory tab mounts it with `agentId` fixed to the pane's own
  agent (current behavior, extended).
- A new Armory rail entry ("Native Memory," per the naming note above)
  mounts the *same* component behind an agent picker (reusing whatever
  agent-selection pattern the launch modal or Warden's Host section
  already use for "pick an agent from all registered ones").

New read-only RPCs (mirroring the existing `agent:memory:*` naming):
- `agent:memory:history` — `(agent_id, filename) -> Vec<VersionSummary>`
  (id, source, source_detail, created_at, content_hash, size).
- `agent:memory:diff` — `(from_version_id, to_version_id) -> unified diff`
  (server-side diff computation; reuse whatever diffing crate/approach the
  frontend PR-review UI already uses, if any — not researched in this
  pass, flagged as an implementation-time lookup).
- `agent:memory:revert` — `(agent_id, filename, target_version_id) ->
  new VersionSummary` — implemented as a **new write** (new version row
  with `content` copied from `target_version_id`, `source: 'revert'`,
  `source_detail` naming the target version id), never a destructive
  rewrite of history. This mirrors how `git revert` works (new commit)
  rather than `git reset` (history rewrite) — matches the "like GitHub"
  framing and keeps the version chain append-only per G1.

Both entry points call the same three RPCs against the same table — one
history, reachable from two places, with nothing in between to keep in
sync.

### 4.4 Provenance flagging in the UI

Any version whose `source = 'jekt'` gets a visible tag in the history list
(e.g. "⚠ written in response to a jekt — TIER=sensitive, TRUST=network-claimed")
sourced directly from `source_detail`. A version whose `source =
'external_fs_write'` (§4.5) gets its own distinct tag ("⚠ detected outside
AgentMux's write path — provenance unknown") — this is a materially weaker
claim than `'jekt'` (we don't know *why* it was written, only *that* it
was, and by inference *when*), and the UI should not conflate the two.
This is the concrete UI answer to "how would a human have caught the §1
incident sooner" — a memory write that traces back to unverified-sender
content, or that bypassed tracking altogether, is flagged at the point of
review, not left to be discovered by chance days later.

### 4.5 Robustness: tracking writes that bypass `~/.agentmux` entirely

**This is the specific gap the operator flagged, and it is not
hypothetical — it is what happened in this very conversation.** Everything
in §4.1–§4.4 assumes a memory write goes through `agent:memory:write_file`
(called by the `MemoryWrite` MCP tool). Nothing requires that. The Claude
Code CLI reads/writes `~/.claude/projects/<hash>/memory/*.md` as plain
files, and an agent can (and, in this conversation, did) use its own
general-purpose filesystem tools — `Write`, `Edit`, `Bash rm` — directly on
those files instead of calling `MemoryWrite`/`MemoryList`. When that
happens, **no RPC fires, so §4.1's version-insert never runs, and the
write is invisible to everything proposed above**, exactly as it was
invisible to `db_agent_native_memory`'s own mirror before this spec.

**Design: two independent layers, so failure of one degrades rather than
blinds the system.**

1. **Fast path — live file-watch.** Reuse `FsWatchPool` (§2.6), which
   already exists and already has self-healing built in. For every agent
   with an active session, `subscribe_dir` on its live
   `memory_dir_for_cwd(CLAUDE_CONFIG_DIR, working_directory)` (the same
   path-derivation `native_memory_handlers.rs` already computes). On a
   `Modified`/`Created` event for a `.md` file, read the file, compute its
   hash, and compare to the latest known version's `content_hash` for that
   `(agent_id, filename)`. If different **and** no `write_file` RPC call
   is what produced it (i.e., the RPC path already recorded a version with
   this exact hash — check before inserting, since a `write_file` call
   also touches the live file and would otherwise double-count), insert a
   new version with `source: 'external_fs_write'`, `source_detail:
   {"detected_via": "fs_watch", "mtime_ms": ...}`.

2. **Slow path — periodic reconciliation sweep.** `FsWatchPool`'s own doc
   comments are explicit that its broadcast channel is "a wake signal, not
   a guaranteed delivery log" (§2.6) — an event can be missed (consumer
   lag, a watch that failed to establish and is still in `RETRY_BACKOFF`,
   or — the biggest real gap — srv wasn't running at all when the write
   happened, e.g. between `task package` channel restarts). Piggyback a
   sweep onto the same cadence class as `FsWatchPool`'s own
   `HEALTH_SWEEP_INTERVAL` (30s, §2.6): for every agent with a known
   memory directory, hash each live `.md` file and compare against its
   latest recorded version. A mismatch found this way gets
   `source_detail: {"detected_via": "reconciliation_sweep", ...}` instead
   of `"fs_watch"` — same `source: 'external_fs_write'`, but the detail
   distinguishes "we saw it happen" from "we noticed it had already
   happened," which matters for how much a reviewer should trust the
   recorded `created_at` as the actual write time (fs-watch: accurate to
   the event; reconciliation: only known to be within the last sweep
   interval, or since srv last started, whichever is longer).

**Precision, honestly bounded — what this can and cannot promise:**
- A single external write, caught by either layer, is captured with its
  *exact* full content and hash — "precision" here means the diff itself
  is never approximate, only its timing/attribution might be.
- Two external writes to the same file between one reconciliation sweep
  and the next collapse into one detected version (the intermediate state
  is unrecoverable) — same fundamental limit
  `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` §4 already accepts for
  the existing mirror (a write-then-wipe within one gap is invisible to
  any polling-based design; fs-watch narrows the gap from "until the Stash
  modal happens to be opened" to "until the next 30s sweep," but does not
  close it to zero without an OS-level guarantee `notify` itself doesn't
  make).
- `source_detail` never claims a `session_id` for an externally-detected
  write unless a best-effort correlation is unambiguous (e.g., exactly one
  Claude Code session `.jsonl` under the same project directory has a
  matching mtime within the same second) — see open question §7.3. Getting
  this wrong (attributing a write to the wrong session) is worse than
  leaving it blank, so the correlation is opt-in-confident, not
  best-guess-always.
- If both layers somehow miss a change entirely (watch failed, srv was
  down through multiple sweep intervals, *and* two writes raced within
  that window), the next successful comparison still catches the drift —
  it just can't reconstruct what was overwritten. This is a visible,
  honest gap (the version chain shows a jump, not a smooth history) rather
  than the silent, undetectable loss that exists today with no tracking
  at all.

---

## 5. What belongs to Warden instead

Memory/bundle version history is config-state history — it belongs with
the config (Stash/Armory, §4.3), not with Warden. But the research for
this spec surfaced real, unbuilt gaps in Warden's *own* stated charter
(runtime oversight of agent-to-agent and network trust boundaries,
`specs/SPEC_WARDEN_WIDGET_2026-05-25.md`) that are worth a dedicated
follow-up spec, listed here so they aren't lost in scoping this one down:

- **Durable jekt audit log.** §2.3's `Handler.audit_log` is a 100-entry,
  restart-losing ring buffer. The original Warden spec's append-only
  `~/.agentmux/audit/YYYY-MM-DD.jsonl` design was never built. This is the
  actual Warden-shaped analogue of what this spec builds for memory —
  same problem (overwrite/loss of history), different domain (jekt events,
  not config content).
- **An approval queue for `ESCALATE=required` jekts.** Today, a sensitive
  jekt's STOP-and-ask surfaces only inside the receiving agent's own pane
  transcript (`amx/CLAUDE.md`'s jekt rules) — a human has to be watching
  that specific pane. A Warden-level queue ("3 jekts awaiting your
  confirmation, across 2 agents") would make the existing STOP rule
  actually reliable to enforce, rather than dependent on which pane a
  human happens to have open.
- **`governance.json` / capability policy.** Still entirely unbuilt (§2.3)
  — `can.jekt.lan`-style declarative capabilities, kill switches per trust
  layer, quotas. Host section's "deregister" today is soft (routing-only,
  doesn't kill the PTY) — real kill switches are a Warden feature, not an
  Armory one.
- **LAN/WAN enrollment and drift detection**, and fixing the already-known
  LAN panel auth bug (§2.3's "401'd from the day it shipped" note) — both
  squarely Warden's existing Host/LAN sections, unrelated to memory.

None of the above is proposed or scoped by this spec — listed only so the
Warden-shaped work this research turned up has a place to land later,
distinct from the Armory-shaped work this spec actually does.

---

## 6. Non-goals

- Not building any of §5's Warden governance features — separate spec.
- Not reconciling `memory_id`/`is_global`/`startup_bundle_id` (§2.5) into
  one canonical Agent↔Bundle binding, and not adding a Bundle↔Memory FK
  (§4.2) — both real, valuable, but explicitly separate follow-up work.
  `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md`'s deferred Bundle↔Memory
  question remains open after this spec, not answered by it.
- Not adding audit/version history to `db_bundles` content itself (bundle
  instructions/context files/MCP config) or to `db_accounts`/secrets —
  both are real gaps found in this research (§2.8) but are separate specs;
  this one is scoped to native memory only.
- Not enforcing `provenance` server-side or blocking writes that omit it
  — advisory metadata only in v1 (§4.1). §7.2 asks whether a `source:
  'jekt'` write under `ESCALATE=required` should ever be blocked outright;
  this spec's v1 only flags, never blocks.
- Not a guarantee of zero data loss for out-of-band writes (§4.5) — best
  effort with an honestly-bounded failure mode, not a claim of
  completeness the underlying OS/filesystem primitives can't back.

---

## 7. Open questions for the human operator

1. **Retention/GC — RESOLVED (2026-08-22, operator-confirmed).** Hybrid
   age + min-count floor: a version is pruned only once it is both older
   than 90 days AND ranked beyond the 50 most-recent versions for its
   `(agent_id, filename)` — so a rarely-touched file never loses its
   entire history purely because every version happens to be old, and a
   hyperactive file never grows unbounded purely because every version
   happens to be recent. Implemented as
   `Store::agent_native_memory_version_prune` (single SQL statement using
   `ROW_NUMBER() OVER`, `agentmux-srv/src/backend/storage/agent_native_memory_versions.rs`)
   plus a new daily background sweep,
   `agentmux-srv/src/backend/native_memory_retention.rs` (`MIN_KEEP = 50`,
   `MAX_AGE_MS = 90 days`), spawned once at startup in `main.rs` alongside
   `native_memory_drift`'s own sweeps. Deliberately a much slower cadence
   (24h) than drift detection's 30s — pruning is housekeeping, not
   latency-sensitive.
2. **Should a `source: 'jekt'` write ever be blocked outright** (not just
   flagged) when the triggering jekt's `TIER=sensitive` with
   `ESCALATE=required` per the current jekt trust rules — i.e., should
   writing to native memory itself be treated as a "sensitive operation"
   requiring the same STOP-and-ask gate as other sensitive actions, rather
   than only being flagged after the fact? This spec proposes flag-after
   (§4.4) as the v1 behavior since it's strictly additive to current
   behavior and unblocks the visibility gap immediately; enforcement is a
   larger, separate decision.
3. **Session correlation confidence bar** (§4.5): is "exactly one
   candidate session with a matching mtime" the right bar for attributing
   an `external_fs_write` to a `session_id`, or should this spec not
   attempt correlation at all in v1 and leave it consistently blank until
   a more reliable signal exists?
4. **Priority of the `memory_id`/`is_global`/`startup_bundle_id`
   reconciliation** (§2.5) as a follow-up — is that worth scheduling soon
   given it's a real, already-shipped source of per-agent config
   confusion independent of anything in this spec, or lower priority than
   it appears?

---

## 8. Test plan

- Unit: `agent:memory:write_file` inserts exactly one new version row per
  call, with correct `parent_version_id` chaining; a no-op write (content
  identical to current) still records a version (simplicity over
  cleverness — dedup by hash is a possible future optimization, not v1).
- Unit: `agent:memory:revert` produces a new version whose content matches
  the target version exactly, and never deletes or mutates prior rows.
- Integration: writing via the `MemoryWrite` MCP tool with
  `provenance.source = 'jekt'` round-trips through to a version row with
  `source_detail` containing the full jekt marker fields.
- Integration: `agent:memory:history`/`diff`/`revert` return byte-identical
  results whether called with a given `agent_id` from the Stash mount
  point or the Armory mount point (§4.3) — confirms the "one source, two
  views" property holds, not just architecturally but observably.
- Integration (§4.5 fast path): write a memory `.md` file directly via a
  plain filesystem write (bypassing `MemoryWrite`), confirm a new
  `source: 'external_fs_write', detected_via: 'fs_watch'` version appears
  within one `notify` event cycle.
- Integration (§4.5 slow path): simulate srv being down during a direct
  filesystem write (write the file with srv's `fs_watch` subscription
  torn down), restart srv, confirm the next reconciliation sweep (≤30s)
  catches the drift and records it as `detected_via:
  'reconciliation_sweep'`.
- Manual: reproduce this spec's own motivating incident (§1) end-to-end —
  simulate a jekt-triggered memory write, confirm it appears in both the
  Stash Memory tab and Armory's Native Memory view (same data, §4.3)
  flagged with its trust marker, and confirm a human can diff it against
  the prior version and revert in one action from either entry point.

---

## 9. Rollout

- New table `db_agent_native_memory_versions` is purely additive — no
  migration of existing `db_agent_native_memory` rows is required for the
  system to function, but a one-time backfill should insert a single
  `source: 'agent_inferred'` version per existing `(agent_id, filename)`
  row (using its current `content`/`updated_at`) so history views aren't
  empty for pre-existing memory on upgrade.
- `MemoryWrite` MCP tool schema gains an optional `provenance` field;
  omitting it defaults to `source: 'agent_inferred'` — fully backward
  compatible with existing callers/agents that don't know about it yet.
- `FsWatchPool` subscriptions (§4.5) are established per active agent
  session — no change to agent spawn/teardown sequencing beyond adding one
  `subscribe_dir` call; the reconciliation sweep (§4.5) is a new
  `HEALTH_SWEEP_INTERVAL`-cadence background task, additive to srv startup.
- No change to `agent:memory:{list,read_file}` read paths or to the
  live-filesystem-is-authoritative merge behavior in
  `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`.
- No schema change to `db_bundles`, `db_agents`, or `db_agent_content` —
  this spec touches none of the three bundle-pointer mechanisms (§2.5),
  by design.
