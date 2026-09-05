# SPEC: Global Memory is the only concept — remove shared provider config, materialize into the provider's file

**Date:** 2026-09-05
**Status:** Draft — design, pre-implementation. **Q4 resolved; no longer blocked.**
One design question outstanding: whether §4.1a's Kind B is needed at all
(recommendation: no).
**Author:** Agent2

> **Filename kept deliberately.** This file was created earlier today as
> "Armory — Shared Provider Setup" and is already referenced by PR #2994 and its
> commits. Same precedent as
> `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md`, which kept its name after
> §5 changed what it targeted. **The concept named in the old title is now
> deleted by this spec** — read the title above, not the filename.

---

## 0. Revision history of this spec (short, because it matters)

1. **v1** — proposed a new "Shared Provider Setup" feature alongside Global
   Memory: own tables, own UI, own MCP tools.
2. **v2** (Codex review, PR #2994) — the materialization target was wrong for
   every non-Claude provider, which collapsed the destination distinction
   between the new feature and Global Memory. Q4 ("what *is* the difference?")
   became blocking.
3. **v3 — this version.** Operator resolved it:

> right, Global memory and shared provider config are the same thing, that is
> what appears on the Memory -> Global page .. are you saying they are coded
> seperately?

> get rid of the shared provider config entirely .. we just need the concept of
> Global Memory that behind the scenes gets into the provider's special file

They were coded separately (§1). They should not be. **v1's entire proposed
feature is deleted; what remains is a deletion plus two additions to Global
Memory.**

---

## 1. Why this was confusing — verified

Two unrelated mechanisms render on one page (Armory → Memory → Global) and both
end in a file named `CLAUDE.md`:

| | **Global Memory** | **Shared provider config** |
|---|---|---|
| Storage | `db_bundles WHERE is_global = 1` (`memory_bundles.rs:161`) | a file on disk, `~/.agentmux/shared/providers/claude/CLAUDE.md` |
| Authored via | Armory editor — create / edit / order | nothing; hand-edited on disk |
| RPC | `bundle_memory_*` (full CRUD) | `getclaudeglobalconfig` — read-only, no write counterpart |
| Reaches the agent | AgentMux **composes** it into `<work_dir>/CLAUDE.md` at launch (`agent_open.rs:757-766`) | AgentMux never touches it; the CLI reads it via `CLAUDE_CONFIG_DIR` |
| Applies to | every agent | default (non-identity-bound) Claude agents only |

The page's own heading admits the split — *"Claude Code provider config —
reference only, not part of Global Memory"* — which is honest and still
misleading: a read-only, hand-maintained, Claude-only artifact sits directly
beneath a managed, all-agents one.

---

## 2. Target state

**One user-facing concept: Global Memory.** Global, applies to every agent,
authored in Armory. Where its content physically lands is an implementation
detail the user never sees — including the provider's own config file.

Three pieces of work:

- **A. Delete** the shared-provider-config surface (§3).
- **B. Materialize** Global Memory into the provider's special file (§4).
- **C. Add change management + an agent write path** to Global Memory (§5) —
  the two requirements from the original ask that Global Memory does **not**
  have today (verified: `db_bundles` has no versions table, and the `Memory*`
  MCP tools target the agent's *own* native brain, not global bundles).

---

## 3. A — Delete the shared provider config surface

Remove:

- the read-only block and its section heading (`global-brain-manager.tsx:163-192`),
- `getclaudeglobalconfig` + `COMMAND_GET_CLAUDE_GLOBAL_CONFIG` (`commands.rs:254`,
  handler `agent_handlers/memory.rs:240-250`),
- `GetClaudeGlobalConfigCommand` (`rpc-api/memory.ts:81-87`) and the
  `claudeGlobalConfigAtom` / fetch in `global-brain-model.ts`.

**Keep `read_claude_global_config`** if any other caller remains (check before
deleting — it was written as a generic dir+filename reader).

**Do NOT delete `seed_claude_md_placeholder_if_missing`.** It is unrelated to
the UI surface and is load-bearing — see §4.3. This is the single most likely
thing to be "cleaned up" by mistake while doing A.

The on-disk file is not deleted; it becomes a generated artifact (§4) with a
one-time adoption of any pre-existing content (§6).

---

## 4. B — Materialize Global Memory into the provider's file

### 4.1 Two destinations, both already understood

**Kind A — working directory** (`<work_dir>/<startup_instructions_filename>`).
Already implemented for every provider. Unchanged by this spec.

**Kind B — provider config dir.** New. Only where a provider is *independently
verified* to read from it. Today that is **Claude only**
(`$CLAUDE_CONFIG_DIR/CLAUDE.md`, measured in
`REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §2).

`startup_instructions_filename` must **not** be reused to derive Kind B — its
contract (`providers.rs:111-119`) is explicitly working-directory-relative, and
it carries a "set only where independently verified, not guessed" discipline.
Kind B needs its own verified per-provider mapping. (This was Codex's P1 on v2;
the guard test in §7 exists to stop it being reintroduced.)

**Why this is no longer a limitation.** Under v1's framing, "Kind B only works
for Claude" was a hole in a provider-agnostic feature. Now it isn't: Global
Memory already reaches **every** provider through Kind A. A provider without a
verified Kind B destination loses nothing.

### 4.1a ⚠ Is Kind B needed at all? (Codex P2 — likely not)

Both paths are read by Claude. Kind A already injects
`format_global_brain_block` into `<work_dir>/CLAUDE.md`
(`agent_open.rs:752-766`), and the isolation report establishes
`$CLAUDE_CONFIG_DIR/CLAUDE.md` is *also* read. **Adding Kind B therefore puts the
same Global Memory text into a Claude agent's context twice** — wasted context,
and duplicated directives can read as emphasis to a model.

The operator's requirement was *"Global Memory that behind the scenes gets into
the provider's special file."* Kind A **already satisfies that literally**:
`CLAUDE.md` *is* Claude's file, `AGENTS.md` is Codex's, and so on — the
per-provider filename comes from `startup_instructions_filename`, which is
exactly the "behind the scenes, right file per provider" behaviour asked for. It
has worked for every provider since before this spec.

**Recommendation: drop Kind B.** Section B of this spec then reduces to
"already done — verify and document it," and the real work is §3 (delete the
surface) plus §5 (versioning + agent authorship).

**What must NOT be dropped with it:** the placeholder seeding in the provider
config dir (§4.3). That is an isolation control, not a content-delivery
mechanism, and it stays regardless of Kind B's fate.

If Kind B is kept anyway — e.g. to reach Claude's *user-level* memory tier
specifically, which has different precedence from project memory — the spec must
define deduplication so the content is not emitted twice. Do not ship both paths
without resolving this.

### 4.2 Identity-bound agents

An agent bound to an OAuth account has its provider config env var overwritten
with that account's own dir (`inject.rs:591-598, 667`) — the default shared dir
is never read. Kind B must therefore be reconciled into the **resolved** config
dir at first-turn injection (`inject.rs`, alongside the existing seed call), not
only written once to the shared dir. Otherwise identity-bound agents silently
keep a stale file while the UI says Global Memory applies to everyone.

### 4.3 The isolation invariant — must not regress

`seed_claude_md_placeholder_if_missing` exists because an isolated
`CLAUDE_CONFIG_DIR` with **no** `CLAUDE.md` falls through to the operator's
personal `~/.claude/CLAUDE.md` — measured, not inferred
(`REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §2).

Writing real Global Memory content to that path satisfies the same invariant,
and the seeder already no-ops when a file exists (`providers.rs:697`), so the
two compose. **But when Global Memory is empty, the placeholder is still the
only thing preventing the leak.** Required behaviour:

| Global Memory | `$CLAUDE_CONFIG_DIR/CLAUDE.md` |
|---|---|
| non-empty | the composed content |
| empty | the placeholder |
| emptied after being non-empty | **revert to the placeholder — never leave it absent or zero-length** |

That third row is the dangerous one and needs an explicit test (§7).

### 4.4 Ownership

Written with the managed marker (`CLAUDE_MD_MANAGED_MARKER`,
`agent_config.rs:904`) so a hand-maintained file is never silently clobbered,
subject to the adoption step in §6.

---

## 5. C — Change management and agent authorship for Global Memory

Both are genuine gaps today, not existing behaviour to reuse:

- `db_bundles` has **no** version history table.
- The `Memory*` MCP tools (`agentmux-mcp/src/main.rs:623-660`) are scoped to
  *"your own native memory (brain) markdown files"* — `db_agent_native_memory`,
  per-agent. They cannot touch global bundles.

### 5.1 Versioning

Mirror `db_agent_native_memory_versions` (`migrations.rs:778-798`) — do not
invent a second scheme. That table already models `content_hash`,
`parent_version_id`, `source`, `source_detail`, `session_id`, and its
history/diff/revert impls (`app_api/mod.rs:1032-1188`) carry reagent-P1 fixes a
fresh implementation would rediscover the hard way.

```sql
CREATE TABLE IF NOT EXISTS db_bundle_versions (
    id                TEXT PRIMARY KEY,
    bundle_id         TEXT NOT NULL,
    content           TEXT NOT NULL,          -- the bundle's instructions
    content_hash      TEXT NOT NULL,
    parent_version_id TEXT,
    source            TEXT NOT NULL DEFAULT 'user',   -- 'user' | 'agent' | 'import'
    source_detail     TEXT NOT NULL DEFAULT '{}',
    session_id        TEXT NOT NULL DEFAULT '',
    created_at        INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_bundle_versions_lookup
    ON db_bundle_versions(bundle_id, created_at);
```

**Declare it in all three schemas and bump all three counters (Codex P2).**
Verified: `db_bundles` is declared three times in `migrations.rs`
(`:472`, `:1125`, `:1411`) — shared `store.db`, the per-channel `objects.db`
fallback, and `identity-store.db` for parity — and the precedent table
`db_agent_native_memory_versions` is likewise declared three times
(`:778`, `:1237`, `:1557`). Declaring the new table in only one schema makes
history calls fail with `no such table` depending on which store `id_store`
resolved to — a bug that reproduces only in the fallback configuration, i.e.
the one least likely to be tested.

Required: add the table to **all three** schema definitions and bump
`SHARED_STORE_SCHEMA_VERSION` (`:59`), `OBJECT_SCHEMA_VERSION` (`:270`) and
`IDENTITY_STORE_SCHEMA_VERSION`, with coverage for each open mode (§7).

Keyed per bundle, because Global Memory genuinely *is* multiple bundles the user
orders — unlike v1's singleton, this key is real, not speculative.
`source` defaults to `'user'` (Armory bundles are user-authored by default),
inverting the native-memory default.

Scope note: this versions **bundles**, which includes non-global ones. That is
the right seam — versioning `is_global` bundles only would be an odd carve-out
in the same table — but it means the UI must not imply history is a
global-only feature.

### 5.2 Agent write path

New MCP tools, named so the *scope difference* from the existing per-agent
`Memory*` family is unmissable:

| Tool | Purpose |
|---|---|
| `GlobalMemoryList` | list global bundles |
| `GlobalMemoryRead` | read one |
| `GlobalMemoryWrite` | create/replace → new version, `source='agent'` |
| `GlobalMemoryHistory` / `GlobalMemoryDiff` / `GlobalMemoryRevert` | audit + undo |

**🔒 System-tier bundles are OFF LIMITS to agents (Codex P1, PR #2997).**
`bundle_memory_list_global()` returns `is_system` rows too — it selects
`WHERE is_global = 1` and merely *orders* by `is_system DESC`
(`memory_bundles.rs:161-170`). Those rows carry an existing invariant that only
`bundle_memory_upsert_system` / `bundle_memory_delete_system` may modify them.
Defining the agent tools over "all global bundles" would hand an agent the
operator-controlled, highest-priority tier — and a `GlobalMemoryRevert` modelled
directly on native-memory revert would write the row straight past the generic
upsert guard.

Required:

- `GlobalMemoryWrite` / `GlobalMemoryRevert` **must reject any `is_system`
  bundle**, explicitly, at the tool boundary — not rely on a lower-layer guard
  a revert path might bypass.
- `GlobalMemoryList` / `Read` may include system bundles (visibility is fine and
  useful), but must mark them read-only.
- A rejection test per mutating tool (§7). This is a privilege boundary, so it
  needs a test that fails loudly if someone later "simplifies" the filter.

**Security posture — operator-decided (2026-09-05): any agent may write
non-system global bundles, fully audited.** This is content *every other agent*
loads at launch, so auditability is the only control and these are requirements,
not niceties:

- unattributable writes are **rejected**, never recorded as anonymous;
- history is strictly append-only — revert creates a new version, so an agent
  cannot erase its own trail through the same API;
- the UI must visibly distinguish agent-authored versions from user-authored;
- content is instructions read by a model, not executed — this bounds the
  exposure to influence, not arbitrary execution, and should be stated plainly
  rather than assumed stronger.

Tool descriptions must say the content is global and affects every agent. The
existing `MemoryWrite` description's `provenance` pattern (source `human` /
`jekt` / default `agent_inferred`) is a good model to copy for attribution.

---

## 6. Migration and adoption

1. On first run, if a Kind B destination holds a non-placeholder file, import it
   as a global bundle (`source='import'`) **and adopt the file** — rewrite with
   the managed marker, content byte-identical. Without adoption the writer would
   treat it as user-owned forever, so the DB would advance while the file the
   provider reads never changed (Codex P2 on v2).
2. Placeholder or absent → import nothing, leave the placeholder.
3. Adoption is one-time and **user-visible**; AgentMux is taking ownership of a
   file the user may have maintained by hand. The `@import` side-file pattern
   (`SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md`) is the opt-out.

---

## 7. Test plan

**Deletion (A)**
- No reference to `getclaudeglobalconfig` remains; the Armory page renders
  without the block.
- `seed_claude_md_placeholder_if_missing` still exists and is still called.

**Materialization (B)**
- Kind B destination resolution **asserts no non-Claude provider has one** —
  the guard against reintroducing v2's P1.
- Non-Claude agents still receive Global Memory via Kind A (unchanged).
- Identity-bound agent gets current Global Memory in *its own* resolved dir.
- **Isolation matrix (§4.3), all three rows** — especially emptying Global
  Memory restoring the placeholder.
- Partial file-write failure leaves DB/history intact and self-heals on
  reconcile.
- An un-adopted hand-edited file is not clobbered; an adopted one is updated.

**Change management (C)**
- One save → exactly one version row, correct `source`/`hash`/`parent`.
- Revert creates a new version; history is never rewritten.
- `GlobalMemoryWrite` records `source='agent'` + `agent_id`; an unattributable
  write is rejected.
- **`GlobalMemoryWrite` and `GlobalMemoryRevert` REJECT an `is_system` bundle**
  — one test each. Privilege boundary; must fail loudly if the filter is ever
  "simplified" away.
- **Schema parity:** history works in all three open modes (shared `store.db`,
  per-channel `objects.db` fallback, `identity-store.db`) — the fallback mode
  especially, since that's where a single-schema declaration would silently
  produce `no such table`.

**Manual**
- Edit Global Memory, launch a Claude agent, confirm the content is present —
  using the calibrated arm-3 prompt from
  `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` §1, **not** a
  yes/no question. That report's own first pass got a false null from an
  uncalibrated instrument.

---

## 8. Open

**Q3 (carried).** Materialize on write, at launch, or both? Recommend **both** —
write-through on save so disk matches the DB immediately, plus a launch-time
reconcile so a hand-deleted file self-heals. Kind B for identity-bound agents is
necessarily launch-time regardless (§4.2).

*(Q4 — the Global Memory relationship — is resolved by this revision: there is
no second concept.)*

---

## 9. Related

- `docs/reports/REPORT_SHARED_PROVIDER_CONFIG_STATE_AND_BUILDOUT_2026_09_05.md` — current-state audit
- `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md` — versioning pattern to mirror
- `SPEC_ISOLATE_HOST_CLAUDE_MD_2026_08_31.md` + `REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md` — the §4.3 invariant
- `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` — managed marker / `@import` opt-out
- `SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` — the spec chain that created the deleted surface
- `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` — Global Memory's system tier (note: contains a known-wrong path claim)
- `SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md` — `startup_instructions_filename`'s contract
