# SPEC: Armory — Shared Provider Setup (provider-agnostic, versioned, dual-authored)

**Date:** 2026-09-05
**Status:** Draft — design, pre-implementation. Supersedes the buildout options in
`docs/reports/REPORT_SHARED_PROVIDER_CONFIG_STATE_AND_BUILDOUT_2026_09_05.md` §4
(that report recommended keeping the surface read-only; the operator directed
otherwise — see §0).
**Author:** Agent2

---

## 0. The ask, verbatim

> inside the armory, we dont need to know it is for Claude Code, or any of the
> .md files. The user is interested in only shared provider setup. It is global,
> applies to all agents. behind the scenes, you setup the right provider's file.
> we want change managemenet, make it updateable by the user, and the agent.

Five requirements, each load-bearing:

1. **Provider-agnostic UI** — no "Claude Code" branding, no `.md` filenames, no
   paths in the primary surface.
2. **One concept: "shared provider setup"** — global, applies to every agent.
3. **AgentMux resolves the target file per provider**, invisibly.
4. **Change management** — versioned, auditable, revertible.
5. **Writable by both the user (UI) and the agent (tool).**

This replaces today's read-only preview (`REPORT_…_2026_09_05.md` §1.2), which
shows the badge *"Claude Code — shared provider config"* and a raw
`~/.agentmux/shared/providers/claude/CLAUDE.md` path — i.e. exactly the two
things requirement 1 removes.

---

## 1. Current state (verified 2026-09-05)

- **Storage:** none. One hand-maintained file per provider dir; no table, no
  schema.
- **UI:** a single read-only block under Armory → Memory → Global
  (`global-brain-manager.tsx:163-192`) — path + `<pre>`, no editor.
- **RPC:** `getclaudeglobalconfig` (read-only, no write counterpart by design).
- **Reach:** the Claude CLI reads `$CLAUDE_CONFIG_DIR/CLAUDE.md` itself. AgentMux
  never injects it — unlike Global Memory / skills / MCP, which are all
  materialized per-agent at launch.
- **Scope:** Claude only. The seeding helper early-returns for any provider whose
  `auth_dir_name != "claude"` (`providers.rs:679-681`).

---

## 2. What already exists that this must reuse

Three pieces of infrastructure make this mostly composition, not invention.

### 2.1 Provider→filename resolution already exists

`ProviderConfig::startup_instructions_filename: Option<&'static str>`
(`providers.rs:120`) already maps every provider to its instructions file:
`CLAUDE.md` (`:220`), `AGENTS.md` (`:252`), `GEMINI.md` (`:279`), `QWEN.md`
(`:323`), `.pi/APPEND_SYSTEM.md`, and `None` (`:364`) for providers with no such
concept.

**This is requirement 3, already built.** The UI never names a file; the
registry answers "which file, for this provider."

`None` is meaningful and must not be papered over: for such a provider, shared
setup has nowhere to go, and the UI must say so rather than silently no-op.

### 2.2 Change management has a working precedent — mirror it

`db_agent_native_memory` + `db_agent_native_memory_versions`
(`migrations.rs:760-798`, `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md`)
is exactly requirements 4+5 already solved for a sibling surface:

- content table (current state) + append-only `_versions` table (history),
- `content_hash`, `parent_version_id`, `created_at`,
- **`source TEXT NOT NULL DEFAULT 'agent_inferred'` + `source_detail` +
  `session_id`** — i.e. *who wrote this, user or agent* is already a modelled
  column, not something to invent,
- backing impls `memory_history_impl` / `memory_diff_impl` / `memory_revert_impl`
  (`app_api/mod.rs:1032-1188`),
- agent-facing MCP tools `MemoryRead/Write/List/History/Diff/Revert`.

**Do not design a second versioning scheme.** Mirror this one, including its
hard-won details (`app_api/mod.rs:958` — history/diff/revert hard-fail rather
than silently degrade; `:988`/`:1151` — reagent P1 fixes around revert staleness
that a fresh implementation would rediscover the hard way).

### 2.3 Launch-time materialization has a precedent

`agent_config.rs` already writes a provider-resolved instructions file, skills,
`.claude/settings.json` and `.mcp.json` into the working directory at launch,
and `write_claude_md_respecting_ownership` (`agent_config.rs:1106`) already
solves "don't clobber a file the user owns" via `CLAUDE_MD_MANAGED_MARKER`.

---

## 3. Design

### 3.1 Concept and naming

One Armory rail entry: **"Provider Setup"**.

- Global. Applies to every agent. Stated in the UI in those words.
- No provider branding in the primary view, no filenames, no paths.
- Provider specificity is an *advanced* detail, not the headline (§3.4).

### 3.2 Data model

Two new tables, mirroring §2.2 field-for-field where the semantics match:

**Content model — decided (operator, 2026-09-05): one shared body, materialized
everywhere.** The user authors once; AgentMux writes that same body into each
provider's own instructions file. Per-provider content is explicitly *not* in
scope for v1.

The row is still keyed by `provider_id` rather than collapsing to a single
singleton row — the storage cost is nil and it keeps a later per-provider
override a UI change rather than a migration. v1 simply writes the same content
to every provider key, and the UI presents one editor.

```sql
-- Current state. Keyed per provider for future flexibility; v1 keeps every
-- row's content identical and presents a single editor (§3.4).
CREATE TABLE IF NOT EXISTS db_provider_setup (
    provider_id   TEXT PRIMARY KEY,
    content       TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL DEFAULT 0,
    updated_at    INTEGER NOT NULL DEFAULT 0
);

-- Append-only history. Same shape and same reasons as
-- db_agent_native_memory_versions.
CREATE TABLE IF NOT EXISTS db_provider_setup_versions (
    id                TEXT PRIMARY KEY,
    provider_id       TEXT NOT NULL,
    content           TEXT NOT NULL,
    content_hash      TEXT NOT NULL,
    parent_version_id TEXT,
    source            TEXT NOT NULL DEFAULT 'user',   -- 'user' | 'agent' | 'import'
    source_detail     TEXT NOT NULL DEFAULT '{}',     -- {agent_id, tool, …}
    session_id        TEXT NOT NULL DEFAULT '',
    created_at        INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_provider_setup_versions_lookup
    ON db_provider_setup_versions(provider_id, created_at);
```

Note the one deliberate divergence: `source` defaults to `'user'` here, not
`'agent_inferred'`. Native memory is *primarily* agent-written with the user
occasionally intervening; provider setup is the reverse. A wrong default here
silently mis-attributes the audit trail, which is the whole point of the table.

**DB is the source of truth. The on-disk file becomes a build artifact.** That
is the substantive change from today, and it's what makes versioning meaningful
— you cannot revert a file that anything else is free to overwrite.

### 3.3 Materialization

On write (not only at launch — see §6 Q3), for each provider with a non-`None`
`startup_instructions_filename`:

```
<provider_auth_dir(provider)>/<startup_instructions_filename>
```

Written with a managed-ownership marker, reusing the
`CLAUDE_MD_MANAGED_MARKER` pattern (`agent_config.rs:904`) so a hand-edited file
is never silently clobbered.

**Interaction with the isolation seeding (important):**
`seed_claude_md_placeholder_if_missing` exists so an isolated `CLAUDE_CONFIG_DIR`
never falls through to the operator's personal `~/.claude/CLAUDE.md` — measured,
not inferred (`REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §2).
Writing **real** shared-setup content to that same path satisfies the same
invariant: the seeding function already no-ops when a file exists
(`providers.rs:697`), so this composes correctly. It must stay a no-op-if-present
and must not be "simplified" away — with shared setup empty, the placeholder is
still the only thing standing between an agent and the operator's personal file.

### 3.4 UI

Armory → **Provider Setup**:

- A single editor for the shared setup content. Markdown, same editing affordance
  as Global Memory bundles.
- Save writes a new version (`source='user'`).
- **History panel** — versions list, diff against any prior version, revert.
  Reuse the Global Memory audit UI wholesale rather than building a second one.
- Status line: *"Applies to all agents"* + how many providers it materializes to
  — **not** the filenames.
- **Advanced/disclosure only:** the resolved per-provider destination paths, and
  an explicit callout for any configured provider whose
  `startup_instructions_filename` is `None` (§2.1) — those agents will *not*
  receive shared setup, and hiding that would be a silent lie.

The existing read-only "Claude Code — shared provider config" block is removed;
this replaces it.

### 3.5 Agent write path

New MCP tools, named and shaped after the `Memory*` family so there is one
mental model:

| Tool | Purpose |
|---|---|
| `ProviderSetupRead` | current content |
| `ProviderSetupWrite` | replace content → new version, `source='agent'`, `source_detail` carries `agent_id` |
| `ProviderSetupHistory` | version list |
| `ProviderSetupDiff` | diff two versions |
| `ProviderSetupRevert` | revert to a version |

Every agent write is attributed and revertible by construction — that is what
makes "updateable by the agent" safe to grant.

**Security posture — decided (operator, 2026-09-05): any agent may write, fully
audited.** No per-agent gating, no allowlist.

This is a *global* surface: an agent with `ProviderSetupWrite` changes
instructions every other agent reads on next launch. That is a real privilege
vector — an agent editing what all its peers are told — and the decision is to
accept it in exchange for capability, with auditability as the control. That
places weight on the audit trail actually being trustworthy, so these are
requirements, not nice-to-haves:

- **Every write is attributed.** `source='agent'`, `source_detail` carries
  `agent_id` and tool name, `session_id` populated. A write that cannot be
  attributed must be rejected, not recorded as anonymous.
- **History is append-only.** Revert creates a *new* version; nothing rewrites or
  deletes history, so an agent cannot cover its tracks through the same API.
- **The UI must surface agent-authored changes.** An agent-written version that
  looks identical to a user-written one in the history list defeats the control
  — show the author, and consider surfacing recent agent writes proactively
  rather than only on request.
- **Content is instructions, not code.** It is read by a model, not executed —
  which bounds the damage to influence rather than arbitrary execution. Worth
  stating plainly so nobody later assumes a stronger guarantee than exists.

---

## 4. Migration

1. On first run, if `<provider_auth_dir>/<instructions_filename>` exists and is
   **not** the placeholder, import it as version 1 with `source='import'`.
2. If it is the placeholder (or absent), start empty — the placeholder is not
   user content and must not become version 1.
3. Leave the file on disk; from then on it is regenerated from the DB.

---

## 5. Test plan

**Rust**
- Provider→filename resolution covers each registry entry, including `None`.
- Write creates exactly one version row with correct `source`/`content_hash`/
  `parent_version_id`.
- Revert produces a *new* version (never rewrites history) — mirror the existing
  native-memory revert tests, including the staleness case from
  `app_api/mod.rs:1151`.
- Materialization writes the right file per provider and **skips** `None`
  providers without erroring.
- **Isolation regression:** with shared setup empty, an isolated dir still gets
  the placeholder; with content, the real content. Neither state leaves the dir
  without a file.
- Ownership: an unmanaged hand-edited file is not clobbered.

**Frontend**
- Editor saves → new version appears in history.
- Diff/revert against the version list.
- A `None`-filename provider is disclosed, not hidden.

**Manual**
- Edit setup in Armory, launch an agent, confirm the agent's context reflects it
  — using the calibrated arm-3 prompt from
  `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §1, not a yes/no
  question. That report's own first pass got a false null from an uncalibrated
  instrument; anyone verifying this must not repeat it.

---

## 6. Decisions and remaining questions

### Decided (operator, 2026-09-05)

- **Agent writes: any agent, fully audited.** No gating. See §3.5 for the
  auditability requirements this decision makes load-bearing.
- **Content: one shared body, materialized to every provider's file.**
  Per-provider content deferred; schema already accommodates it. See §3.2.

### Still open

**Q3. Materialize on write, or at launch?**
On-write is simpler to reason about and makes the disk state match the DB
immediately. At-launch matches how every other shared resource behaves. Recommend
**both**: write-through on save, plus a launch-time reconcile so a hand-deleted
file self-heals.

**Q4. Relationship to Global Memory.** Both are global, both end up as
instructions the agent reads. The honest distinction is *destination*: Global
Memory composes into the agent's **working directory** instructions file; this
composes into the **provider config dir** file. A user will reasonably ask why
there are two. Worth an explicit answer in the UI copy, or a decision to merge
them later. **Not blocking**, but it should be answered deliberately rather than
allowed to drift — this spec's own predecessor chain
(`SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md`) needed three self-corrections
largely because these two concepts kept being conflated.

---

## 7. Related

- `docs/reports/REPORT_SHARED_PROVIDER_CONFIG_STATE_AND_BUILDOUT_2026_09_05.md` — current-state audit this builds on
- `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md` — the versioning pattern to mirror
- `SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md` + `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` — the isolation invariant §3.3 must preserve
- `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` — the managed-marker pattern
- `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` — what this replaces
- `SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md` — `startup_instructions_filename`'s own spec
