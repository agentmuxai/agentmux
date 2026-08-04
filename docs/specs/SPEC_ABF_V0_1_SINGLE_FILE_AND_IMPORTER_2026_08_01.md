# Spec: ABF v0.1 — Single-File Format + Importer (Phase 2)

**Date:** 2026-08-01
**Status:** Spec — implementation follows in this same effort (Phase 2, §4 below).
**Relationship to prior work:** refines and partially supersedes
`docs/specs/REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16.md` §5 (the
original ABF proposal). That report's research (§1–§4, §7) stands unchanged —
this doc only revises the packaging decision in its §5.4 and specifies the
importer its §6 Phase 2 left as a one-paragraph sketch. Do not re-litigate the
research; it was adversarially verified across four passes and nothing here
contradicts it.

---

## 1. What actually shipped since the original report (ground truth, verified against code)

The original report was written as "no implementation yet." That's now stale:

| Phase | Report's plan | Actual state (2026-08-01) |
|---|---|---|
| **0** — align skills with Agent Skills (SKILL.md) | — | **Shipped.** `SKILL_TYPE_AGENT_SKILL`, `render_skill_md`, `.claude/skills/<slug>/SKILL.md` materialization all live in `agent_config.rs` / `agent-config-builder.ts`. |
| **1** — exporter | "Pure read-side; zero schema risk" | **Shipped**, and more thoroughly than the report sketched: `agentmux-srv/src/backend/bundle_export.rs` (1194 lines) — instructions/context files/skills/MCP servers/inferred credential requirements, plus **secret redaction** in exported MCP configs (env, headers, CLI args, URL userinfo/query params — none of this was in the original report, added across PRs #2325/#2333 after real security findings). Wired via `bundle.export` RPC (`server/app_api/bundle.rs`), including a **zip archive option** (`zip_bundle_export`, `format: "zip"`) the report's §5.4 had framed as a later, optional phase.
| **2** — importer | "validation... create rows... account resolution" | **Not started.** No `bundle_import.rs`, no `bundle.import` RPC, nothing. This is what §4 below specifies. |
| **3** — Armory UI (export/import buttons) | — | **Not started.** No UI surface for either direction yet. |
| **4/5** — OCI distribution, public registry | — | **Not started.** |

## 2. Revision: `.abf` (zip) is the primary interchange format for v0.1, not a later phase

The original report's §5.4 filed zip under "Distribution (phase-gated, not
required for v0.1)," treating the loose directory tree (§5.1) as the v0.1
baseline and zip as an optional convenience added later. That framing is now
inverted for two reasons:

1. **It's already built.** `zip_bundle_export` exists, is tested, and is
   reachable via `format: "zip"` on the existing RPC — there's no remaining
   work to "add" zip support, only to formalize it as the recommended form.
2. **"An ABF file that gets loaded" needs a file, not a directory.** The
   directory-tree layout (§5.1 of the original report) is the right *logical*
   structure — it's what an importer parses internally, and it's the right
   shape for a bundle checked into a git repo unpacked — but it is not
   something a user can point a "load bundle" action at as a single artifact.
   A zip removes that gap with zero new format invention: it's the same
   directory tree, byte-identically laid out inside the archive.

**Decision:** the **`.abf` file** — a zip archive whose internal layout is
exactly the directory tree from the original report's §5.1 (`armory.json` +
`instructions/` + `skills/` + `mcp/` + `accounts/requirements.json`) — is the
**primary, recommended interchange format for ABF v0.1**. The unpacked
directory tree remains a fully valid, equivalent representation (useful for
hand-editing or committing a bundle's source into a repo unpacked) — an
importer MUST accept either. There is no third format; this is not a new
schema, only a packaging decision.

- **Extension:** `.abf`
- **MIME type:** `application/vnd.agentmux.bundle+zip` (informative; not
  currently registered anywhere, matches the pattern of `.mcpb`'s
  `application/vnd.anthropic.mcpb`)
- **Internal structure:** unchanged from the original report's §5.1 — a zip
  is just bytes-on-disk for the same tree `zip_bundle_export` already
  produces. No new manifest field, no new schema version needed for this
  change alone.
- **Compression:** Deflate (already what `zip_bundle_export` uses via the
  `zip` crate's `["deflate"]` feature — no new dependency).

This does **not** change §5.4's later phases (OCI distribution) — those
remain future work, layered on top of the same `armory.json` content via a
different transport, exactly as originally proposed.

## 3. `armory.json` — no schema change, one clarification

The manifest schema from the original report's §5.2 is unchanged. One thing
worth stating explicitly, since the exporter's own code comments already
flagged it as a real, deliberate gap (`bundle_export.rs`'s module doc
comment, citing Codex P1 on PR #2325): `mcp/<slug>.server.json` currently
contains **AgentMux's own runtime MCP config shape** (`{type, command, args,
env}` — the same object `.mcp.json` uses), not the official MCP registry
`server.json` schema (`packages[].registryType`/`identifier`/
`environmentVariables`). This spec does not resolve that gap — real
conversion between the two shapes would require fabricating fields a bare
stdio command doesn't contain. The importer (§4) reads back exactly the
shape the exporter writes (AgentMux's runtime shape); accepting genuine
upstream `server.json` files is out of scope for v0.1 and tracked as a
follow-up, not silently guessed at here.

## 4. Phase 2: the importer

### 4.1 Scope

Backend + RPC only for v0.1 — no UI (Phase 3, still separately tracked).
Mirrors `bundle_export.rs`'s shape and quality bar: pure functions for
parsing/validation (no I/O, no Store access), a thin RPC handler
(`bundle.import`) that owns the Store side-effects, symmetric
warnings/skipped-item reporting so a lossy import is never silent.

### 4.2 Input

`bundle.import` accepts **either**:
- `zip_base64`: a base64-encoded `.abf` zip (mirrors `bundle.export`'s own
  `zip_base64` response field), **or**
- `files`: the raw `[{path, content}]` list (mirrors `bundle_export.rs`'s
  `BundleExportFile[]` shape) — lets a caller who already has an unpacked
  directory tree (e.g. read directly off disk) skip the zip round-trip.

Exactly one of the two must be present; both or neither is a request error.

### 4.3 Validation (in order — first failure wins, no partial import)

1. **`armory.json` must exist and parse as JSON.** Missing or malformed →
   reject the whole import with a clear error, nothing is written.
2. **`$schema` and `version` are read but not enforced as a hard gate for
   v0.1** — no schema registry exists yet to validate against (the
   `https://docs.agentmux.ai/schemas/armory-bundle/v0.1/bundle.schema.json`
   URL the exporter writes is aspirational, not yet a real, fetchable
   document as of this spec). Record both verbatim on the created bundle's
   metadata for forward compatibility; do not fail import on an unrecognized
   version — warn instead. Revisit once a real schema exists to validate
   against.
3. **Every path referenced in `components` must exist among the provided
   files/zip entries.** A `components.instructions` entry pointing at a
   missing file is a warning (skip that entry), not a hard failure — mirrors
   the exporter's own "warn, don't silently vanish, but don't block on a
   partial problem" philosophy for `context_files`/`mcp_servers`.
4. **Path safety.** Every file path in the archive/list is re-validated with
   the same rules `sanitize_context_relative_path` already enforces on
   export (no absolute path, no `..` traversal, no drive letter) — an
   untrusted `.abf` file is exactly the kind of input this must defend
   against on the way *in*, even though the exporter only had to defend
   against it on the way *out*. A path that fails this check is dropped with
   a warning, never written anywhere, never used to escape the intended
   unpack scope (there is no on-disk unpack for the DB-import path — this
   guards against a future filesystem-materializing importer inheriting the
   same code, and against any path used as a lookup key).
5. **`accounts/` files other than `requirements.json` are rejected outright**
   — same invariant the original report's Phase 2 sketch stated explicitly
   ("Never import secret material even if present in a malicious bundle").
   Any other file under `accounts/` is neither read nor written anywhere; its
   presence is recorded as a warning.

### 4.4 What gets created

- **One `Memory` (bundle) row**, `is_global: false` by default (an import
  should not silently become injected into every agent's CLAUDE.md without
  explicit user action — matches the existing `bundle_memory_upsert`
  contract, which never flips `is_global` on its own). `instructions` from
  `instructions/AGENTS.md` (concatenated if the manifest lists more than
  one instructions-component path, in manifest order — mirrors how
  `format_global_brain_block` already joins multiple bundles with `---`,
  reused here at the file level for a multi-file `instructions` component).
  `context_files` from every `instructions/context/*` file the manifest
  references. `mcp_servers` from every `mcp/*.server.json` file, written
  **verbatim, `${VAR}` placeholders and all** — see the correction in §4.5
  below on why this stays templated rather than being "resolved" in place.
- **One `Skill` row per `skills/<slug>/SKILL.md`**, created via
  `skill_upsert_unique_global` (global, matching how an imported *bundle*
  — a shared resource — should behave, not bound to any one agent).
  `skill_type: "agent-skill"`. Parsed from the SKILL.md's YAML frontmatter
  (`name`, `description`) + body, using the **existing** frontmatter parser
  if one already exists for a different call site (check
  `agent_config.rs`/`skills.rs` before writing a new one); if none exists,
  a minimal parser is in scope here since `render_skill_md`'s inverse
  doesn't currently exist anywhere in the codebase. A skill name colliding
  with an existing global skill is a warning + skip (not a hard failure —
  matches `skill_upsert_unique_global`'s own `NameConflict` error being
  surfaced as a per-item warning, not an aborted import).
- **Nothing under `db_accounts`, ever.** Import never creates, modifies, or
  reads an actual credential. `accounts/requirements.json` is read-only
  input to §4.5's resolution; it is not itself imported as a row anywhere.

### 4.5 Account requirement resolution — informational only, never substituted

**Correction to this spec's original draft**, made during implementation:
the draft said a single unambiguous account match should be "substituted"
into the created `mcp_servers` row in place of the `${VAR}` placeholder.
That's wrong and was cut before shipping. Unlike CLI-provider identity
(`resolver/inject.rs`, `SecretRef`-backed, resolved fresh at every spawn),
MCP server `env` values have **no equivalent spawn-time indirection** — a
value written into `db_bundles.mcp_servers` is materialized into
`.mcp.json` **verbatim**, no resolver pass in between. "Substituting an
account binding" for an MCP server would therefore mean writing the
account's *actual secret value* into a DB column that is, by construction,
the same column an `armory export` of this very bundle reads from — i.e.
exactly the credential-leak-into-a-shareable-artifact shape the exporter's
redaction logic (`redact_mcp_entry`) exists to prevent. Resolution here
stays **read-only and purely informational**:

For each entry in `accounts/requirements.json`, look up existing
`db_accounts` rows by `provider` (exact match) — **read-only, no
creation, no write of any kind.** The original report's Phase 2 sketch
says "prompting to link or create" — the *prompting* half is a Phase 3 UI
concern; creating a new account (which necessarily means the user
supplying a real credential) is never something an import of untrusted
bundle data should trigger unattended, and linking an existing one to an
MCP server's env var is exactly the credential-injection surface this
importer must not touch. The lookup result is reported back
(`resolved_requirement_ids` for exactly-one-match, `unresolved_requirements`
for zero-or-multiple, per requirement id) purely so a future Phase 3 UI can
show "N of M requirements have a matching account — link them" without
this backend ever having written a secret anywhere. `mcp_servers` in the
created bundle always contains the placeholders exactly as exported, full
stop — resolving them is deferred entirely to whatever mechanism eventually
lets a user fill in an MCP server's env value through the normal Armory
MCP Servers editor (already exists, untouched by this spec).

### 4.6 Result shape (symmetric to `BundleExport`)

```rust
pub struct BundleImport {
    pub bundle_id: String,
    pub imported_skill_ids: Vec<String>,
    pub skipped_skills: Vec<String>,          // name conflicts, malformed SKILL.md
    pub resolved_requirement_ids: Vec<String>,   // exactly one matching account found (read-only lookup, nothing written)
    pub unresolved_requirements: Vec<Value>,     // zero or multiple matches — see §4.5
    pub warnings: Vec<String>,                // missing/unsafe paths, rejected accounts/* files, unenforced schema version, etc.
}
```

### 4.7 RPC

`bundle.import` — window-scoped, same as `bundle.export` (bundles aren't
agent-specific). Request: `{ zip_base64?: string, files?: [{path, content}] }`.
Response: `BundleImport` (§4.6) as JSON.

## 5. What this spec deliberately does not do

- **No UI.** Phase 3 (Armory Export/Import buttons + import-review sheet)
  remains separately tracked, unchanged from the original report's §6.
- **No OCI distribution.** Phase 4/5 unchanged.
- **No `server.json` schema conversion.** §3 above.
- **No dynamic memory.** The original report's §5.5 non-goal stands
  unchanged — nothing in this spec touches it.
- **No schema-registry validation.** §4.3.2 — deferred until a real,
  versioned JSON Schema document exists to validate against.
