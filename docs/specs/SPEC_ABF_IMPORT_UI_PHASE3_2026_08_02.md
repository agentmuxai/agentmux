# Spec: ABF Import UI (Phase 3) — Selective Import + Collision Handling

**Date:** 2026-08-02
**Status:** Spec — not yet implemented.
**Relationship to prior work:** builds on
`docs/specs/SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01.md`, whose §4.1
explicitly scoped Phase 2 to "backend + RPC only... no UI (Phase 3, still
separately tracked)." This is that Phase 3. The existing `bundle.import` RPC
(merged, `agentmux-srv/src/backend/bundle_import.rs` +
`agentmux-srv/src/server/app_api/bundle.rs`) is all-or-nothing: it parses an
`.abf`/directory tree and writes everything found in one shot. This spec adds
a preview step and a selective commit, and specifies collision handling for
each importable item type — none of which the current RPC does.

---

## 1. Why a new phase, not an extension of `bundle.import`

The user-facing requirement is: pick a file, see what's inside, choose what
to bring in, get told about (and resolve) anything that collides with
existing data. `bundle.import` today can't support that even as a UI-only
addition — it's a single atomic write, so "select items" would either be
decorative (everything gets imported regardless of the checkboxes) or
require a genuine backend split. This spec makes that split explicit:

- **`bundle.import.preview`** — pure parse, zero Store writes. Returns
  everything the modal needs to render a selection screen, including
  collision status computed against current Store state.
- **`bundle.import.commit`** — takes the same file bytes plus the user's
  selections (and any collision resolutions) and writes only what was
  selected.

Both RPCs are stateless request/response, like everything else in this
engine — no server-side session or preview-caching token. The frontend holds
the picked file's bytes in modal state and sends them to both calls; a
multi-MB base64 payload twice is a non-issue at the sizes this format
already caps itself to (`MAX_TOTAL_UNCOMPRESSED_BYTES` = 50MB uncompressed,
typically far less compressed).

## 2. What can collide, and what can't (ground truth, verified against code)

This determines which parts of the modal need collision UI at all.

| Item | Collision surface today | Source |
|---|---|---|
| **Skills** | Yes — `skill_upsert_unique_global` rejects a new skill whose `name` matches an existing **global** skill (`is_global = 1`), regardless of id. Create-only-if-unique; no overwrite-by-name path exists. | `agentmux-srv/src/backend/storage/skills.rs` |
| **MCP servers** | **None.** A global MCP catalog (`db_mcp_servers`, `mcp_server_upsert_unique_global`) exists, but `bundle.import` never calls it — imported MCP servers are written straight into the new bundle row's own `mcp_servers` JSON blob, never promoted to the global catalog. There is nothing for them to collide with. | `agentmux-srv/src/backend/storage/mcp_servers.rs`; confirmed absent from `bundle_import.rs`/`bundle.rs` |
| **Bundle name** | **None enforced.** `bundle_memory_upsert` only conflicts on `id` (always a fresh UUID for an import); `name` has no uniqueness constraint — two bundles can freely share a display name. | `agentmux-srv/src/backend/storage/memory_bundles.rs` |
| **Context files / instructions** | **None.** Each import creates a brand-new bundle row; there's nothing existing for a fresh bundle's `instructions`/`context_files` to collide with. | — |
| **Account requirements** | **None** — read-only, informational (`resolved_requirement_ids` / `unresolved_requirements`), never written anywhere. | `bundle.rs` §4.5 handling |

**Implication:** the only *hard* collision this UI needs to resolve is
**skills**. Everything else either has no collision concept (MCP servers,
context files, requirements) or is a soft, non-blocking naming nicety
(bundle name). The spec below reflects that — most of the "collision
handling" work is one well-scoped mechanism (skill name conflicts), not a
generic system for every item type. MCP servers never being promoted to the
global catalog is a **known, deliberate asymmetry with skills** carried over
from the current backend, not something this UI spec resolves — flagged as
an open question in §7.

## 3. RPC design

### 3.0 Required Phase 2 amendment: stable per-item source IDs

**codex P1 on PR #2381:** `ParsedSkill.slug` comes from SKILL.md's own
`name:` frontmatter (`parse_skill_md`), not from the manifest directory
reference — and `parse_bundle_import`'s skill dedup (`seen_skill_dirs`) is
keyed on that *directory* string, not on the resulting slug. Two different
`components.skills` directories whose SKILL.md files both declare the same
`name:` produce two `ParsedSkill` entries with an **identical** `slug`,
which the parser never catches (name-uniqueness is only enforced later, at
write time, by `skill_upsert_unique_global`'s DB constraint). MCP servers
are worse: `mcp_servers: Vec<Value>` is arbitrary parsed JSON with **no
required `"name"` field at all** — dedup (`seen_mcp_paths`) is keyed on the
manifest path, not on any field inside the JSON. A selection request keyed
on `slug` or a `"name"` value therefore cannot reliably identify — or
distinguish between — two colliding rows.

**Fix, needed in `bundle_import.rs` before Phase 3 can be built, not just in
the new RPCs:**
- `ParsedSkill` gains a `source_dir: String` field — the exact
  `components.skills` directory reference that produced it (already
  computed and already unique per entry, since `seen_skill_dirs` dedups on
  it; just not currently retained on the struct).
- `mcp_servers` changes shape from `Vec<Value>` to
  `Vec<ParsedMcpServer { source_path: String, config: Value }>` — same
  idea, using the already-unique `components.mcpServers` path reference
  that's discarded today after the `by_path` lookup.
- Both source identifiers are **selection keys only** — everything else
  about how skills/MCP servers get written (via slug, via raw JSON
  respectively) is unchanged.

This is a small, mechanical addition to structs that already compute these
values and throw them away — not a re-design of the parser.

### 3.1 `bundle.import.preview`

**Request** — identical shape to today's `bundle.import`:
```jsonc
{ "zip_base64": "..." }   // or: { "files": [{ "path": "...", "content": "..." }] }
```

**Response:**
```jsonc
{
  "name": "Backend Dev Bundle",
  "description": "...",
  "instructions_preview": "Be concise. Prefer existing patterns...",
  "context_files": [
    { "path": "conventions.md", "size_bytes": 39 }
  ],
  "skills": [
    { "source_dir": "skills/deploy-checklist", "slug": "deploy-checklist", "description": "...", "collision": "none" },
    { "source_dir": "skills/code-review-v2", "slug": "code-review", "description": "...", "collision": "name_conflict" }
  ],
  "mcp_servers": [
    { "source_path": "mcp/github.server.json", "config": { "name": "github", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"] } }
  ],
  "requirements": [
    { "id": "req-1", "provider": "github", "env": "GITHUB_TOKEN", "resolved": false, "match_count": 0 }
  ],
  "warnings": [ "components.instructions: ... skipped" ],
  "name_collision": false   // true if an existing bundle already has this exact name (soft, informational)
}
```

`source_dir`/`source_path` are the §3.0 selection keys — always present,
always unique per row, independent of whatever the row's own `slug`/JSON
`name` field says. `config` is passed through verbatim for display; the UI
falls back to `source_path`'s basename when `config.name` is absent.

Implementation notes:
- `parse_bundle_import` already produces everything here except
  `collision`/`name_collision`/the requirement `resolved`/`match_count`
  fields — three new, read-only additions the RPC handler makes:
  - **Skill collisions**: one call to `skill.catalog.list`'s underlying
    `wstore.skill_list_global()` (`agentmux-srv/src/server/app_api/skill.rs`,
    `register_skill_catalog_list`), name-matched against each parsed
    skill's slug. **Not** `skill.list` (agent-scoped, requires `agent_id`)
    and **not** a `skill.list_global` command — that command doesn't exist
    (codex P1 on PR #2381; corrected from an earlier draft of this spec).
  - **Bundle name collision**: a name scan over existing bundles (whatever
    read method `bundle.list`'s handler already uses).
  - **Requirement resolution** (codex P2 on PR #2381): `parse_bundle_import`
    does not resolve requirements against connected accounts — it only
    returns the raw `id`/`provider`/`kind`/`env`/`optional` fields parsed
    from `accounts/requirements.json` (see its own doc comment: "ready for
    the RPC handler to resolve accounts against"). The **exact** resolution
    logic already exists in `bundle.rs`'s current `bundle.import` handler
    (`match_count_by_provider`, a per-provider-deduplicated
    `id_store.identity_list` lookup — not `wstore`, per that code's own
    round-4/5 history) and must be extracted into a shared helper both the
    current commit-path handler and the new preview handler call, rather
    than duplicated inline in a second place where it could drift.
- `instructions_preview` is the full instructions string, not truncated
  server-side — truncate for display client-side if needed (keeps the RPC
  contract simple; the size cap already bounds this to something reasonable
  in the worst case).
- Same `MAX_ENTRY_COUNT`/`MAX_ENTRY_UNCOMPRESSED_BYTES`/
  `MAX_TOTAL_UNCOMPRESSED_BYTES`/`MAX_ACCOUNT_REQUIREMENTS`/
  `MAX_IMPORTED_SKILLS` caps from Phase 2 apply unchanged — preview reuses
  `parse_bundle_import`/`unzip_bundle_import`/`enforce_raw_files_caps`
  as-is, just skips the Store-write half of today's handler.

### 3.2 `bundle.import.commit`

**Request** — the same file payload, plus selections:
```jsonc
{
  "zip_base64": "...",             // or files[], same as preview — re-sent, not a cache token
  "bundle_name": "Backend Dev Bundle (2)",   // user-editable, defaults to the parsed name
  "include_instructions": true,
  "include_context_files": ["conventions.md"],       // paths to include; omitted path = excluded
  "include_skills": [
    { "source_dir": "skills/deploy-checklist" },
    { "source_dir": "skills/code-review-v2", "import_as": "code-review-team-x" }   // rename to dodge a collision
  ],
  "include_mcp_servers": ["mcp/github.server.json"]   // source_path values, not names -- see §3.0
}
```

**Response** — same shape as today's `bundle.import` response
(`bundle_id`, `imported_skill_ids`, `skipped_skills`,
`resolved_requirement_ids`, `unresolved_requirements`, `warnings`), since
it's the same underlying write path, just filtered to the selection.

Implementation notes:
- The commit handler re-runs `parse_bundle_import` (or reuses a shared
  internal parse helper) against the freshly-sent bytes — it does **not**
  trust client-supplied preview data for anything that gets written. The
  selections (`include_*`) are a filter applied to the freshly-parsed
  result before the existing write loop runs; nothing about the write
  loop's own logic (per-skill `skill_upsert_unique_global` call,
  conflict → warn+skip, rollback on infra failure) changes.
- `include_skills`/`include_mcp_servers` filter the freshly-parsed
  `skills`/`mcp_servers` lists by matching each entry's `source_dir`/
  `source_path` (§3.0) — never by `slug` or a JSON `"name"` field, which
  aren't guaranteed unique across entries.
- `import_as`: when present, the commit handler substitutes it for the
  matched skill's own parsed slug before constructing the `Skill` row —
  the one backend behavior change this spec requires beyond "parse once,
  write a filtered subset" and the §3.0 struct additions. `ParsedSkill`'s
  `slug` field is otherwise always derived from the SKILL.md's own `name:`
  frontmatter (Phase 2, `parse_skill_md`); this is the first path that
  overrides it.
- Server-side re-validation of `skill_upsert_unique_global` is the
  authoritative check regardless of what the preview said — a name
  becoming taken between preview and commit (another import, another user)
  degrades to the existing warn+skip behavior, not a failure. Client-side
  collision detection (§4) is a UX optimization, not the enforcement
  point.
- `bundle_name` collision (if the chosen name still matches an existing
  bundle at commit time) is never blocking — `bundle_memory_upsert` has no
  name constraint, so it's accepted either way. The modal surfaces this as
  a soft warning before commit (§4), not a hard gate.

## 4. Modal flow

Three steps, following the existing app's `ModalLayerApi.replace()`
sequential-modal pattern (used today for the `agent-prereqs` →
`install-agent` → `launch-agent` chain in
`frontend/app/element/modal-dispatch.tsx`) rather than one component
carrying internal step state — this codebase has no existing "wizard
panel," and threading step state through modal-dispatch's request-kind
union keeps each step's props typed and testable independently.

### Step 1 — Select file

- Entry point: an "Import Bundle" action in the Armory → Bundles tab,
  opening the modal at this step.
- File picker: **codex P1 on PR #2381 — `showOpenFileDialog()` cannot be
  reused as originally specified.** It takes no arguments
  (`frontend/util/cef-api.ts:392-394`) and its host-side handler
  (`agentmux-cef/src/commands/platform.rs`, `show_open_file_dialog`) has a
  hard-coded `rfd::FileDialog` filter list of image/video/audio extensions
  only — there's no way to make it show `.abf` files through the existing
  command. This needs a **new host-side dialog command** (e.g.
  `show_open_bundle_dialog`, mirroring the existing command's shape but
  with an `["abf"]` filter) plus a matching `cef-api.ts` entry — new
  host-side plumbing, not a reuse of what's already there. (Exporting a
  `.abf` to disk would need an analogous `show_save_*_dialog`, which also
  doesn't exist yet — out of scope here since export already has a working
  `zip_base64`-download-style path; noted only so it isn't conflated with
  this spec's needs.)
- On selection: read the file, base64-encode, call
  `bundle.import.preview`. Parse/validation errors (malformed zip, missing
  `armory.json`) surface inline on this same step — don't advance.

### Step 2 — Preview & select

Renders the `bundle.import.preview` response as a checklist:

- **Bundle name** — editable text field, pre-filled with the parsed name.
  If `name_collision` is true, an inline hint ("a bundle named this already
  exists") with a one-click suggested alternate (`"<name> (2)"`,
  incrementing) — never blocking, since the backend allows duplicates.
- **Instructions** — single checkbox ("Include instructions"), checked by
  default, with a collapsible preview of `instructions_preview`.
- **Context files** — one checkbox per file (path + size), checked by
  default.
- **Skills** — one checkbox per skill (slug + description), checked by
  default. A skill with `collision: "name_conflict"` shows a collision
  badge and switches its row to a text input pre-filled with the slug,
  where the user types an alternate name to import under (empty = skip on
  commit — same effect as unchecking). This is the one item type with real
  collision UX; see §4.1.
- **MCP servers** — one checkbox per server (name + command), checked by
  default. No collision UI — per §2, there's nothing for these to collide
  with under the current backend design.
- **Account requirements** — read-only summary, no checkboxes ("Depends on
  N account(s): github (resolved), openai (not connected)"). Always
  included; nothing is written from this list regardless.
- Any `warnings` from the parse: a dismissible banner, not blocking.

### Step 3 — Confirm & import

- Summary line built from the current selection state (client-side count,
  no extra RPC): "Importing: instructions, 1 context file, 2 skills, 1 MCP
  server."
- "Import" button calls `bundle.import.commit` with the file bytes +
  selections built from step 2's checklist state. On success, close the
  modal and navigate to the new bundle (mirrors whatever "just-created
  bundle" navigation `bundle.upsert`'s own callers already do, if any
  exists — otherwise a toast + the new bundle appearing in the Bundles
  list is sufficient). On a partial failure (e.g. a skill got skipped
  server-side because of a last-second name conflict), surface the
  response's `warnings`/`skipped_skills` — this is a real, expected
  outcome under the race described in §3.2, not an error state.

### 4.1 Skill collision resolution, precisely

1. Preview's `collision: "name_conflict"` is computed from a global-skill
   name lookup taken at preview time — a snapshot, not a live constraint.
2. The modal fetches the full existing global skill-name list **once**,
   up front, via **`skill.catalog.list`** — the window-scoped, no-`agent_id`
   route Armory already uses (`register_skill_catalog_list`,
   `agentmux-srv/src/server/app_api/skill.rs`). Not `skill.list` (requires
   an `agent_id`, agent-scoped) and not `skill.list_global`, which doesn't
   exist as a command (codex P1 on PR #2381; corrected from an earlier
   draft). This lets the rename text input validate the user's typed
   alternate **client-side, instantly** (grey out / inline error if the
   typed name is itself already taken), without a round-trip per
   keystroke. This is advisory only.
3. At commit, the server is the sole authority: `skill_upsert_unique_global`
   runs its own check regardless of what the client validated. A
   client-side "looks available" name can still lose a race server-side —
   handled by the existing warn+skip behavior (§3.2), surfaced in the
   commit response.
4. Leaving a colliding skill's rename field empty and its checkbox checked
   is treated as "skip this skill" at commit (equivalent to unchecking) —
   never silently sent through with its original, known-conflicting slug.

## 5. What this spec deliberately does not do

- **Does not add MCP servers to the global catalog.** They stay
  bundle-scoped, matching current backend behavior (§2). Promoting them
  (with the same collision machinery as skills) is a bigger, separate
  backend decision — noted in §7, not decided here.
- **Does not add an "overwrite" path for skill collisions.** Only
  skip-or-rename. Silently replacing another skill's content by name is a
  much higher-risk operation (could stomp a skill the importing user
  doesn't own/didn't write) that this spec is not taking a position on.
- **Does not add drag-and-drop file selection.** The new
  `show_open_bundle_dialog` command (§4 Step 1) follows the existing
  per-purpose dialog-command pattern; drag-and-drop is a plausible
  follow-up enhancement, not required for a working Phase 3.
- **Does not change anything about `bundle.export`** or add a "Save File"
  dialog. Export already works via the existing `zip_base64` response;
  wiring that to an actual save-to-disk action is separate, smaller scope
  this spec doesn't block on.
- **Does not persist partial progress across modal close.** Closing the
  modal mid-flow discards the picked file and selections; re-opening
  starts at step 1. No draft-saving.

## 6. Testing

- **Backend:** `bundle.import.preview` — pure-function-level tests mirror
  Phase 2's style (`bundle_import.rs`'s existing test module): collision
  flags computed correctly against a seeded fake global-skill list;
  `name_collision` computed correctly against a seeded bundle-name list.
  `bundle.import.commit` — selection filtering by `source_dir`/
  `source_path` (only checked items get written, including the case of
  two entries with a colliding `slug`/`name` but distinct source paths),
  `import_as` substitution, and the pre-existing warn+skip / rollback
  behavior all still hold when driven through a partial selection rather
  than "everything."
- **Manual/e2e:** a sample `.abf` was generated for this purpose via
  `agentmux-srv/src/backend/bundle_export.rs`'s existing `export_bundle` +
  `zip_bundle_export` (instructions + 1 context file + 2 skills — one of
  which should be pre-seeded as a name collision for manual collision-UI
  testing — + 1 MCP server + an inferred account requirement). Not checked
  into the repo (binary fixture, generated on demand); regenerate via a
  throwaway `#[ignore]`d test calling those two functions and
  `std::fs::write`-ing the result, or once this UI exists, via the app's
  own export action on any real bundle.

## 7. Open questions

1. **Should MCP servers get global-catalog collision handling too**,
   matching skills, for symmetry? Today's asymmetry (skills promoted
   globally, MCP servers bundle-scoped only) predates this spec and isn't
   something a UI change should silently paper over or silently extend.
   Needs a product decision, not an engineering default.
2. **Should a colliding skill's rename be free-text, or constrained** (e.g.
   auto-suggest `"<slug>-imported"` and only let the user accept/edit that
   suggestion, rather than a blank field)? Free-text is simpler to spec and
   implement; a suggested default reduces friction. Either is compatible
   with §3.2/§4.1 as written — this is a UX call, not an architectural one.
3. **Post-import navigation** — does anything today navigate to a
   just-created bundle after `bundle.upsert`, worth mirroring for
   `bundle.import.commit`'s success case? Not confirmed during this spec's
   research; check at implementation time.
