# SPEC: Armory — Shared Provider Setup (provider-agnostic, versioned, dual-authored)

**Date:** 2026-09-05
**Status:** Draft — design, pre-implementation. **Blocked on §6 Q4** (see below).
Supersedes the buildout options in
`docs/reports/REPORT_SHARED_PROVIDER_CONFIG_STATE_AND_BUILDOUT_2026_09_05.md` §4
(that report recommended keeping the surface read-only; the operator directed
otherwise — see §0).
**Revised 2026-09-05 after Codex review of PR #2994** (2×P1, 2×P2, all accepted):
the materialization target was wrong for non-Claude providers (§2.1/§3.3),
identity-bound agents were missed (§3.3), storage was keyed incoherently
(§3.2), and migration created an ownership deadlock (§4). The first P1 also
collapsed the distinction between this feature and Global Memory for every
non-Claude provider, which is why Q4 is now blocking rather than advisory.
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

### 2.1 Provider→filename resolution exists — but for the *working directory*, not the provider home

`ProviderConfig::startup_instructions_filename: Option<&'static str>`
(`providers.rs:120`) maps every provider to its instructions file: `CLAUDE.md`
(`:220`), `AGENTS.md` (`:252`), `GEMINI.md` (`:279`), `QWEN.md` (`:323`),
`.pi/APPEND_SYSTEM.md`, and `None` (`:364`) where no native convention is
confirmed.

> **⚠ Corrected after review (Codex P1, PR #2994).** The first draft treated this
> field as a provider-*home* filename and proposed writing
> `<provider_auth_dir>/<startup_instructions_filename>`. **That is wrong.** The
> field's contract (`providers.rs:111-119`) states the path is *"relative to the
> agent's working directory"*, and `build_config_files` writes it there. Nothing
> establishes those filenames as provider-home read paths — OpenClaw's own entry
> (`providers.rs:400-408`) explicitly flags `AGENTS.md` as an *unverified*
> workspace bootstrap. Reusing the field as proposed would let saves succeed
> while non-Claude agents never receive the setup.
>
> The field also carries an explicit discipline — *"Set only where independently
> verified against the provider's own docs, not guessed"* — which the original
> proposal quietly violated by inferring a second meaning for it.

**What is actually verified today:**

| Destination | Verified? |
|---|---|
| `<work_dir>/<startup_instructions_filename>` | **Yes** — the field's own contract; `build_config_files` already writes here for every provider |
| `$CLAUDE_CONFIG_DIR/CLAUDE.md` (Claude provider home) | **Yes** — measured in `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §2 |
| `<any other provider's home>/<file>` | **No** — unverified for every non-Claude provider |

`None` remains meaningful and must not be papered over: that provider gets
nothing, and the UI must say so rather than silently no-op.

**Consequence for this spec: the destination is now an explicit, per-provider,
independently-verified mapping — see §3.3.** It is not derivable from
`startup_instructions_filename` alone.

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

> **Corrected after review (Codex P2).** The first draft keyed both tables by
> `provider_id` "for future flexibility" while exposing one shared body and
> provider-less read/write APIs. That is incoherent: with N provider rows there
> is no canonical value to read or revert, and the spec contradicted itself —
> §3.3 required writing every provider row while the test plan required exactly
> one version row per save. Worse, rows could silently diverge after a partial
> write failure. **Fixed: a single canonical content/version stream; providers
> are materialization targets only, never storage keys.**

```sql
-- Singleton: exactly one row, id = 'shared'. Providers are materialization
-- targets (§3.3), not storage keys — there is one canonical body to read,
-- diff and revert.
CREATE TABLE IF NOT EXISTS db_provider_setup (
    id            TEXT PRIMARY KEY,          -- always 'shared' in v1
    content       TEXT NOT NULL,
    size_bytes    INTEGER NOT NULL DEFAULT 0,
    updated_at    INTEGER NOT NULL DEFAULT 0
);

-- Append-only history for that single stream. Same shape and same reasons as
-- db_agent_native_memory_versions. No provider_id: one save = exactly one
-- version, matching the read/revert API and the test plan.
CREATE TABLE IF NOT EXISTS db_provider_setup_versions (
    id                TEXT PRIMARY KEY,
    setup_id          TEXT NOT NULL,                  -- always 'shared' in v1
    content           TEXT NOT NULL,
    content_hash      TEXT NOT NULL,
    parent_version_id TEXT,
    source            TEXT NOT NULL DEFAULT 'user',   -- 'user' | 'agent' | 'import'
    source_detail     TEXT NOT NULL DEFAULT '{}',     -- {agent_id, tool, …}
    session_id        TEXT NOT NULL DEFAULT '',
    created_at        INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_provider_setup_versions_lookup
    ON db_provider_setup_versions(setup_id, created_at);
```

**Per-provider content, if ever wanted, is a v2 migration** — adding a
`provider_id` column and a grouped-version concept. Carrying the key now without
grouped-version semantics buys nothing and creates the divergence risk above.
The `setup_id` column exists so that migration doesn't have to reshape the
primary key.

**Materialization is not atomic with the DB write.** Writing N files can
partially fail. The DB commit is the source of truth and succeeds or fails
alone; file writes are a best-effort projection, retried by the launch-time
reconcile (§6 Q3). A partial file write therefore self-heals and can never
corrupt history — which is the main reason storage must not be keyed per
provider.

Note the one deliberate divergence: `source` defaults to `'user'` here, not
`'agent_inferred'`. Native memory is *primarily* agent-written with the user
occasionally intervening; provider setup is the reverse. A wrong default here
silently mis-attributes the audit trail, which is the whole point of the table.

**DB is the source of truth. The on-disk file becomes a build artifact.** That
is the substantive change from today, and it's what makes versioning meaningful
— you cannot revert a file that anything else is free to overwrite.

### 3.3 Materialization — two destination *kinds*, only one of which is verified everywhere

Reworked after Codex P1 ×2. A destination is only legitimate if the provider is
**independently verified** to read from it.

**Kind A — working-directory instructions file (verified for every provider).**
`<work_dir>/<startup_instructions_filename>`, composed at launch by the existing
`build_config_files`. This already works, for every provider, today.

**Kind B — provider home / config dir (verified for Claude only).**
`$CLAUDE_CONFIG_DIR/CLAUDE.md`, measured in
`REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §2. **No equivalent
is verified for any other provider**, and per §2.1's discipline none may be
assumed.

**Rule:** shared setup materializes to Kind A for all providers, and
*additionally* to Kind B where a verified home path exists (today: Claude only).
Adding a Kind B destination for another provider requires the same evidentiary
bar — a documented read path, ideally measured — and is a per-provider change,
not a config tweak.

> This means Kind A is the load-bearing path for non-Claude providers, and Kind A
> is *also* where Global Memory composes. **§6 Q4 is therefore no longer
> optional** — see there.

**Identity-bound agents (Codex P1).** An agent with an explicit OAuth account has
its provider config env var **overwritten** with that account's own
`SecretRef::OAuthConfigDir` (`inject.rs:591-598, 667`) — the default
`provider_auth_dir` is never read. Materializing only to the shared dir would
leave those agents on a stale file or bare placeholder while the UI claims the
setup "applies to all agents." Since identity dirs are created and seeded at
first-turn injection, **Kind B materialization must be reconciled into the
resolved identity directory at that same point** (`inject.rs`, alongside the
existing seed call), not only written once to the shared dir on save. The
shared-dir write remains correct for default, non-identity-bound agents.

Both kinds are written with a managed-ownership marker, reusing the
`CLAUDE_MD_MANAGED_MARKER` pattern (`agent_config.rs:904`), so a hand-edited file
is never silently clobbered — subject to the adoption rule in §4.

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

## 4. Migration and adoption

> **Corrected after review (Codex P2).** The first draft imported a pre-existing
> file's content but left the file **unmarked**. The §3.3 writer then treats an
> unmarked file as user-owned and refuses to overwrite it — so every later save
> would update the DB and history while the file the provider actually reads
> never changed. A silent, permanent divergence. Import must therefore include an
> explicit *adoption* step.

1. On first run, for each verified destination (§3.3), if a file exists and is
   **not** the placeholder: import its content as version 1 with
   `source='import'`, **and adopt the file** — rewrite it with the managed marker
   prepended, content otherwise byte-identical. Adoption is what makes subsequent
   writes legal.
2. If it is the placeholder, or absent: start empty and leave the placeholder in
   place. The placeholder is not user content and must never become version 1.
3. **Adoption is one-time and must be visible**, not silent — AgentMux is taking
   ownership of a file the user may have hand-maintained. Surface it in the UI
   (and the version's `source_detail` records the adopted path). If the operator
   declines, the alternative is the side-file composition path
   (`SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md`'s `@import` +
   `AGENTMUX_MEMORY.md` companion), which leaves the original untouched — that
   pattern already exists and should be reused rather than reinvented.
4. After adoption the file is regenerated from the DB on every save/reconcile.

---

## 5. Test plan

**Rust**
- Destination resolution covers each registry entry, including `None`, and
  **asserts no Kind B destination exists for a non-Claude provider** — a guard
  against the exact §2.1 mistake being reintroduced.
- Write creates **exactly one** version row (singleton stream) with correct
  `source`/`content_hash`/`parent_version_id`.
- Revert produces a *new* version (never rewrites history) — mirror the existing
  native-memory revert tests, including the staleness case from
  `app_api/mod.rs:1151`.
- Materialization writes Kind A for every non-`None` provider and Kind B only
  for Claude; `None` providers are skipped without erroring.
- **Partial materialization failure** leaves the DB/history intact and is
  repaired by the reconcile pass — no divergence, no orphaned version.
- **Identity-bound reconcile:** an agent with an OAuth account gets the current
  setup in *its own* resolved config dir, not a stale file or bare placeholder.
- **Isolation regression:** with shared setup empty, an isolated dir still gets
  the placeholder; with content, the real content. Neither state leaves the dir
  without a file.
- **Adoption:** importing a pre-existing unmarked file marks it, and a
  subsequent save actually rewrites it (the §4 deadlock, pinned).
- Ownership: an unmanaged, *un-adopted* file is not clobbered.

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

**Q4. Relationship to Global Memory — now BLOCKING (upgraded after Codex P1).**

The original framing was "they differ by destination: Global Memory → working
directory, provider setup → provider config dir," and that distinction is what
made two features defensible. **§2.1's correction removes it for every provider
except Claude.** Since only Claude has a verified provider-home read path, shared
setup for all other providers must materialize into the working-directory
instructions file — which is exactly where Global Memory already composes.

So for non-Claude providers the two features are, mechanically, the same feature
writing to the same file. That has to be resolved before implementation, not
after:

- **(a) Merge.** Shared provider setup becomes a section of Global Memory —
  arguably what the operator's *"only shared provider setup"* framing already
  describes, with the versioning/agent-write requirements applied to Global
  Memory instead.
- **(b) Keep separate, define composition order.** Two authored bodies, one
  destination file, with an explicit and documented precedence.
- **(c) Claude-only v1.** Ship against the one verified Kind B path and defer
  other providers until their home paths are verified — honest, but directly
  contradicts the provider-agnostic requirement.

**Recommendation: (a) or (b), not (c)** — (c) reintroduces the Claude-specificity
the operator explicitly asked to remove. This is the one decision that should be
made before any code is written, and it is precisely the conflation that made the
predecessor chain (`SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md`) need three
self-corrections.

---

## 7. Related

- `docs/reports/REPORT_SHARED_PROVIDER_CONFIG_STATE_AND_BUILDOUT_2026_09_05.md` — current-state audit this builds on
- `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md` — the versioning pattern to mirror
- `SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md` + `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` — the isolation invariant §3.3 must preserve
- `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` — the managed-marker pattern
- `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` — what this replaces
- `SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md` — `startup_instructions_filename`'s own spec
