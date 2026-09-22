# SPEC: cross-channel agent history resolution

**Date:** 2026-09-21
**Status:** active — §3.1 shipped in #3480; §3.2/§3.3 (link history,
shared-store persistence) remain proposed. Root cause originally confirmed by
static code trace across `agentmux` @ `4ab7a9ea9`; **§2.5 records what live
verification subsequently proved this spec's first revision got wrong** — the
identity-link model it assumed cannot work at all for the common case, and the
shipped fix resolves by working directory instead. Read §2.5 before §3.
**Trigger:** Repo owner, after repeatedly being unable to have an agent find
its own past conversations across an AgentMux restart onto a new version.
Named as the top priority: "we often need to restart agentmux in a new
version and the conversation history is lost."
**Builds on:** `SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md` (Phase 1, shipped —
the `SearchHistory` verb and `SessionIndex::search_sessions` this spec does
not touch), `SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`
(the identity→session attribution model this spec fixes a hop in),
`retro-provider-account-switch-loses-agent-history-2026-09-17.md` (proposed,
unshipped — root-caused half of this problem already; this spec completes
the diagnosis and turns it into a design).
**Related:** `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md` (the
sibling per-instance-storage problem for Global Memory — same architectural
shape, different subsystem; that spec's multi-*machine* cloud-sync scope is
deliberately NOT proposed here, see §3.3), `SPEC_MUXSPECT_CROSS_TIER_
CONVERSATION_VISIBILITY_2026_08_21.md` and `SPEC_JEKT_TRANSCRIPT_REQUEST_
TIER_RULES_2026_08_22.md` (the governed path for one agent reading
*another's* history — this spec explicitly does not touch or widen that,
see §4).

---

## 1. Problem

An agent's own conversation history routinely becomes unfindable, in two
distinct situations the repo owner has hit repeatedly:

1. **AgentMux restarts on a new version.** Every restart/version bump spins
   up a new `~/.agentmux/channels/<channel>/` directory (confirmed on this
   machine: 20+ channel directories under one `identities/` root, spanning
   Aug 20 – Sep 21, one new hash suffix roughly every relaunch). The agent's
   own past sessions from the previous channel are not found by
   `SearchHistory` or reflected in `ListConversations` after the restart.
2. **A provider account switch** (already root-caused in the 2026-09-17
   retro) — orphans history the same way, inside a single channel.

Both were previously treated as separate incidents. They share one root
cause at the database layer (§2), which is why fixing either in isolation
would leave the other half broken.

### 1.1 Confirmed empirical reproduction (this investigation)

In the session that prompted this spec, `SearchHistory` was called three
times against this exact identity (varying only the query string) and
returned `sessions_scanned: 0, total_sessions: 0` every time — including
for the single most generic possible query (`"the"`), which should match
virtually any real session. At the same time, `ls`/`Glob` on
`~/.agentmux/channels/<current-channel>/identities/<uuid>/claude/projects/
<slug>/` directly showed **three real `.jsonl` session files**, including
two from *prior* channel directories (one 10.9 MB, one 4.3 MB, spanning the
prior ~8 hours), fully readable and `grep`-able by hand. The files are not
lost. `SearchHistory`'s resolution path never reaches them. That gap is
this spec's subject.

---

## 2. Root cause — three compounding bugs, not one

All three were reached by reading the actual current implementation, not
inferred from symptoms.

### 2.1 Bug A — `SearchHistory`'s `agent` parameter is the wrong identifier namespace

AgentMux has two distinct notions of "agent id" that this feature silently
conflates:

- **The runtime slug** — `AGENTMUX_AGENT_ID` (e.g. `"AgentY"`), injected into
  an agent's MCP process env at spawn. This is what `agent_slug()`
  (`agentmux-mcp/src/main.rs:312-321`) reads, and it's what the `SearchHistory`
  dispatch arm (`agentmux-mcp/src/main.rs:1599-1604`) sends, unmodified, as
  the `agent` query parameter to `GET /agentmux/reactive/history/search`.
- **The definition id** — `db_agents.id`, a `TEXT PRIMARY KEY`
  (`agentmux-srv/src/backend/storage/migrations.rs:618-620`) — an opaque id
  distinct from the human-readable `name` column on the same table. This is
  what `db_agent_identity_links.agent_id` actually references:
  `FOREIGN KEY (agent_id) REFERENCES db_agents(id)`
  (`migrations.rs:573-580`).

`handle_reactive_history_search` (`agentmux-srv/src/server/reactive.rs:2143-
2209`) does **no translation** between the two — it takes `params.agent`
verbatim and passes it straight into
`history.search_for_agent(&store, &agent, …)`
(`reactive.rs:2187-2192`) → `HistoryService::sessions_for_agent`
(`agentmux-srv/src/backend/history/mod.rs:84-118`) →
`store.agent_identity_list_for_agent(agent_id)`
(`agentmux-srv/src/backend/storage/identities.rs:622-637`), which runs:

```sql
SELECT agent_id, account_id, provider
FROM db_agent_identity_links
WHERE agent_id = ?1
```

`?1` here is the **slug** (`"AgentY"`), but the column holds **definition
ids**. Confirmed empirically this session: `DiscoverAgents`/`FleetList` show
`AgentY`'s `agent_id` (slug) and `id`/`definition_id`
(`dedc33bf-b69c-4236-9b34-20bda3ef2738`) as two visibly different strings.
The `WHERE` clause can never match, `links` is always empty, and
`sessions_for_agent` hits its own early return —
`if links.is_empty() { return Ok((Vec::new(), 0, false)); }`
(`mod.rs:101-103`) — **before ever touching `SessionIndex`**, which is why
the three genuinely-indexed session files on disk are never even
considered. This is the single largest cause of the reported symptom, and
by itself is sufficient to reproduce §1.1 exactly.

**This is the "dual identity system" the repo owner suspected.** It is not
a vague architectural concern — it is a specific, single wrong string
compared against a specific, single column in one query.

### 2.2 Bug B — the link table itself does not survive a channel/version switch

Independent of Bug A: `db_agent_identity_links` lives inside each channel's
own local SQLite `objects.db` (same per-instance storage boundary the
2026-09-20 Global Memory sync spec already documents for a different table
in the same database — `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md`
§1: *"Storage is local and per-instance, full stop... living in a SQLite DB
scoped to one local `~/.agentmux/channels/<channel>/` install."*). A fresh
channel — created on every version restart, confirmed by the 20+ channel
directories on this machine — starts that table **empty** for every agent.
Even with Bug A fixed, a brand-new channel has no rows to find until this
agent is freshly linked within it.

The session transcripts themselves are **not** channel-local, importantly:
the 2026-09-17 retro documents (and this investigation's directory listing
confirms) that `.jsonl` transcripts live under a **shared**
`~/.agentmux/shared/identities/<uuid>/claude/projects/<slug>/` root that
channel directories symlink into. The break is entirely in the per-channel
database's link/index layer, not in the files.

### 2.3 Bug C — even within one channel, only the latest link survives

`db_agent_identity_links` has `PRIMARY KEY (agent_id, provider)`, and the
only write path is `Store::agent_identity_link`
(`identities.rs:574-583`):

```sql
INSERT INTO db_agent_identity_links (agent_id, account_id, provider)
VALUES (?1, ?2, ?3)
ON CONFLICT(agent_id, provider) DO UPDATE SET account_id = excluded.account_id
```

A re-link (new provider account, or — per §2.4 below — potentially a new
identity minted per channel) **overwrites** the row rather than appending
to it. There is no history of which `account_id`s a given `(agent_id,
provider)` has ever pointed at, only the current one. This is exactly the
mechanism the 2026-09-17 retro observed for a mid-session provider-account
switch (old identity `a1990489-…` with 13.7 MB of transcripts, new identity
`f333cda4-…` with none, "nothing links the old identity to the new one").

### 2.4 Why `sessions_for_agent`'s own merge logic looked promising but doesn't save this

`sessions_for_agent` already merges across **every** link
`store.agent_identity_list_for_agent` returns (`mod.rs:98-110`) — the
"identity-spanning" fix the 2026-09-17 retro's recommendation #1 asked for
is *already partially built*. It just never receives more than zero or one
row, because of §2.1 (usually zero) and §2.3 (never more than one even when
non-empty). Fixing §2.1 and §2.3 is therefore enough to make this existing
merge code do what it already looks like it's trying to do — no rewrite of
`SessionIndex` or `search_sessions` is needed.

### 2.5 What live verification proved this spec's first revision got wrong

Everything in §2.1–§2.4 was derived by reading code. Before implementing,
the same claims were checked against this machine's live data, and one of
them did not survive:

- **`db_agent_identity_links` is empty.** Not "stale", not "missing a row
  for this agent" — the table has **zero rows**, in both the per-channel
  `objects.db` and the shared `identity-store.db`.
- **The agent's registry record reads `"identity_id": "default"`** — i.e.
  unbound, ambient credentials — while its transcripts are written under a
  *channel* identity bundle (`identities/5984bb4e-…/claude/projects/…`).
  **Nothing anywhere relates the agent to that bundle.**

The consequence is larger than §2.1 assumed. §2.1 called the slug/id
mismatch "the single largest cause... by itself sufficient to reproduce
§1.1." The first half is right and the second is literally true, but it
implied fixing Bug A would fix the symptom. **It would not.** With an empty
link table, `sessions_for_agent` returns zero either way — the identity
bundle is not merely mis-keyed, it does not exist as a relationship for any
agent on ambient credentials, which is the default and by far the common
case. An identity-link-based resolver cannot find those sessions no matter
how correctly it resolves the key.

What *does* hold, verified on the same machine:

| Source | Value |
|---|---|
| `db_agents` row | `working_directory = C:\Users\asafe\.agentmux\agents\agenty-0629j` |
| Registry record | `source_agents_base` + `working_dir` → the same path |
| Transcript's own `cwd` field | `C:\Users\asafe\.agentmux\agents\agenty-0629j` |
| Session project dir name | `C--Users-asafe--agentmux-agents-agenty-0629j` |

**The working directory is the join key that actually holds**, and it is
already recorded on both sides (`SessionMeta.working_directory`, parsed from
the transcript's `cwd`, and the agent's own row). §3.1 below is written to
what was built against that finding, not to the original assumption.

---

## 3. Design

### 3.1 SHIPPED — resolve by working directory, with identity links as a second source

Implemented in `agentmux-srv/src/backend/history/`:

- **`SessionIndex` gains a `by_working_dir` index** (`index.rs`), built in
  `refresh()` alongside the existing `by_identity` one, plus
  `list_for_working_directory()`. Matching uses `normalize_working_dir()`
  (separators unified, trailing separator dropped, case folded **on Windows
  only** — folding case on Linux would merge two genuinely distinct
  directories) and compares the **whole normalized path**, never a
  substring: `list`'s existing `project` filter uses `contains`, which would
  let an agent in `/agents/foo` claim every session from `/agents/foo-2`.
- **`HistoryService::sessions_for_agent` now unions two sources** (`mod.rs`)
  and dedupes by `session_id`, since a bound agent running in its own
  directory legitimately hits both:
  1. identity links — **with Bug A fixed**: the incoming slug is resolved
     via `instance_get_by_slug` to the definition id the link table is
     actually keyed by (a caller already holding a definition id passes
     through unchanged);
  2. sessions whose recorded working directory matches the agent's, taken
     from its `db_agents` row, falling back to the registry
     (`source_agents_base` + `working_dir`) for a live agent that has no
     `db_agents` row at all — launching an agent does not create one, so
     this fallback is the common path, mirroring
     `native_memory_handlers::memory_dir_for_registry_record`'s own
     reconstruction rule.
- **The `links.is_empty()` early return is deleted.** It was the specific
  line that turned "this agent has no identity binding" into a confident,
  authoritative "this agent has no history."

Because all four callers funnel through `sessions_for_agent`, this also
repairs `ListConversations`' backing list and
`bundle.export_for_agent_with_history` — the latter was silently exporting
an empty conversation record for every ambient-credential agent.

### 3.1b Original (superseded) framing — fix Bug A alone

Add a resolution step in `handle_reactive_history_search` (and any sibling
handler with the same shape — audit for other routes that take an `agent`
string from `AGENTMUX_AGENT_ID` and pass it into a `db_agents.id`-keyed
query) before calling `sessions_for_agent`.

**Open question, not resolved by this spec — needs a design decision before
implementation:** `db_agents.name` has no `UNIQUE` constraint in the current
schema (`migrations.rs:618-620`), so a bare `SELECT id FROM db_agents WHERE
name = ?1` is not guaranteed to resolve unambiguously if two agent
definitions share a display name. Two directions, in order of preference:

1. Resolve via the **live registration** the request already has access to
   (the calling process's own `block_id`/registration, which
   `DiscoverAgents`/`FleetList` already key by unambiguous `block_id`),
   rather than a bare name lookup — this sidesteps collision entirely for
   the common case of "an agent asking about itself," at the cost of not
   trivially working for a name passed by something other than the live
   agent itself.
2. If a bare name lookup is kept, add the `UNIQUE` constraint to
   `db_agents.name` (a migration) so resolution is well-defined, and decide
   what happens to any existing collision at migration time.

### 3.2 Fix Bug C — stop overwriting the identity link, keep history

Do not change `db_agent_identity_links`'s existing role (it is presumably
also read by spawn-time credential resolution, which legitimately wants
"the current one" — that call site was out of scope for this investigation
and should be re-confirmed before changing this table's semantics). Instead,
add an **append-only** sibling, e.g. `db_agent_identity_link_history
(agent_id, account_id, provider, linked_at, PRIMARY KEY (agent_id, account_id,
provider))`, written alongside every existing call to
`Store::agent_identity_link` (same transaction). Add
`Store::agent_identity_history_for_agent(agent_id) -> Vec<AgentIdentityLink>`
reading from the new table, and have `HistoryService::sessions_for_agent`
call it instead of (or in addition to — safe either way, since a duplicate
`account_id` just re-scans the same identity) today's
`agent_identity_list_for_agent`. This is additive: nothing that currently
reads `db_agent_identity_links` for its "current pointer" meaning changes
behavior.

### 3.3 Fix Bug B — let the identity-history table survive a channel switch

The user's literal complaint ("restart agentmux in a new version") is a
**same-machine, cross-channel** problem, not a cross-machine one. This is
importantly smaller in scope than `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_
2026_09_20.md`, which is designed around syncing across *machines* via
MuxBus/cloud relay — machinery this problem does not need, because every
channel on one machine already shares a filesystem (`~/.agentmux/shared/`
already exists and is exactly where transcripts themselves already live,
per §2.2).

Proposed: move `db_agent_identity_link_history` (§3.2) — or a copy of it
kept in sync — into a small SQLite database under
`~/.agentmux/shared/agent_identity_history.db`, written by every channel,
read by every channel. This is a much narrower persistence surface than
general Global Memory sync (one append-only table, no conflict/merge model
needed since rows are keyed `(agent_id, account_id, provider)` and a write
from any channel is simply a fact that happened, never contested) and does
not require the cloud-relay design work `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_
SYNC_2026_09_20.md` is still proposing.

**Explicitly out of scope for this spec:** syncing this across *different
machines*. If that's wanted later, it should ride whatever transport
solution the Global Memory cross-instance spec eventually lands on, not
invent a second one here.

### 3.4 Breadcrumb + honest signal (retro §6 items 2–3, adapted)

- On first use of a new channel/identity for an agent, if the shared
  identity-history store already has prior entries for this agent, surface
  that fact rather than staying silent — a first-turn system note ("prior
  sessions for this agent exist under N other identities; the most recent
  is from <date>") turns silent amnesia into a recoverable, known state.
- Extend `SearchHistory`'s response (and `ListConversations`) with an
  `identities_scanned` count, the same way `sessions_scanned` /
  `total_sessions` / `truncated` already distinguish "no matches" from
  "ran out of budget" (`SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md` §3.2). A
  caller should be able to tell "zero hits, and I checked every identity
  this agent has ever had" apart from "zero hits, because identity
  resolution silently failed" — which is precisely how Bug A currently
  presents today: indistinguishable from a genuine negative.

---

## 4. Non-goal: this does not open cross-*agent* history access

**"An agent access[ing] any agent history"** could be read two ways, and
only one of them is what this spec does:

- ✅ **One agent, resolving its own history across every identity UUID it
  has ever had** (different channels, different provider-account switches)
  — this is the entire subject of this spec.
- ❌ **Agent B reading Agent A's conversation history** — already a
  governed act via `muxspect`'s cross-tier visibility protocol and the
  `transcript_request` tier rules (live since PR #2764), with
  `conversation_visibility` of `private`/`trusted_peers`/`ask` and
  `ESCALATE=required` that a verified sender does not relax in `ask` mode.
  `SPEC_AGENT_HISTORY_SEARCH_2026_09_17.md` §5 already made this call
  deliberately for the same reason a second, ungoverned read path would be
  worse than the gap it closes — it would look like an ordinary tool while
  bypassing consent. Nothing in this spec changes that boundary, widens
  `SearchHistory`'s `agent` exposure, or gives one agent a new way to name
  another agent's identity and read its sessions. Bug A's fix (§3.1) is a
  same-agent slug→id resolution, not a parameter that accepts someone
  else's slug.

If genuine cross-agent history access is wanted, that is a distinct,
deliberate scope decision — its own spec, routed through
`transcript_request` per the existing Phase 1 spec's own stated direction
— not something this spec bundles in silently.

---

## 5. Testing / verification plan

- **Bug A regression test:** an agent whose `AGENTMUX_AGENT_ID` slug differs
  from its `db_agents.id` (true for every real agent) writes a session,
  then `SearchHistory` for a term known to be in it must return a hit — not
  `total_sessions: 0`. This is the single test that would have caught
  today's bug; it does not appear to exist today (the existing
  `list_for_identity`/`search_sessions` unit tests in `index.rs` all
  construct their `SessionMeta.identity_id` and query by the *same* string,
  which cannot catch a slug/id mismatch introduced one layer up, in
  `sessions_for_agent`/the HTTP handler).
- **Bug B regression test:** an agent's identity-history entry written from
  simulated channel A is readable from simulated channel B (two `Store`
  instances pointed at two different per-channel DB paths, both reading the
  same shared `agent_identity_history.db` path).
- **Bug C regression test:** two sequential `Store::agent_identity_link`
  calls for the same `(agent_id, provider)` with different `account_id`s —
  `agent_identity_history_for_agent` must return both, in order, not just
  the latest (this is the direct negative of the existing overwrite
  behavior, and should be added next to the existing `db_agent_identity_
  links` tests in `identities.rs` and `store/tests.rs`).
- **End-to-end:** reproduce §1.1's exact scenario — write a session under
  identity/channel 1, simulate a channel switch (new `Store`, new empty
  `db_agent_identity_links`), confirm `SearchHistory` still finds the
  channel-1 session via the shared identity-history store.
- **Non-goal guard:** a test asserting `SearchHistory`'s `agent` parameter
  remains absent from its own MCP tool schema (as today) — Bug A's fix must
  not accidentally reopen it as a caller-suppliable parameter naming
  another agent.

---

## 6. Confidence

- **High / directly verified via code read, not inferred from symptoms**:
  Bug A (the slug vs. `db_agents.id` mismatch, and the unconditional
  early-return on empty links) and Bug C (the `ON CONFLICT ... DO UPDATE`
  overwrite semantics) — both are single, specific, quoted code paths.
- **High confidence, corroborated by this session's own directory listing
  but not by reading the channel-creation code path itself**: Bug B (that a
  fresh channel's `db_agent_identity_links` starts empty). This follows
  from the per-channel-SQLite-file architecture already documented for a
  sibling table in `SPEC_CROSS_INSTANCE_GLOBAL_MEMORY_SYNC_2026_09_20.md`
  §1, and from this investigation directly observing 20+ separate channel
  directories with only 3 session files total resolvable from the current
  one — but the exact code that provisions a new channel's database was not
  read line-by-line in this pass.
- **Not yet resolved, flagged rather than guessed at**: §3.1's open question
  about disambiguating `db_agents.name` collisions. Implementation should
  not proceed on that point without picking one of the two directions
  offered (or another) deliberately.
