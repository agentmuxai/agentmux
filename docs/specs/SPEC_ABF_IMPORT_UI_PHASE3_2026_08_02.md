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
engine — no server-side session or preview-caching token. **codex P1 on PR
#2381, round 3:** an earlier draft of this spec assumed re-sending a
base64-encoded payload on both calls was "a non-issue at the sizes this
format already caps itself to" — that's wrong. RPC traffic (confirmed by
tracing `frontend/app/store/rpc-client.ts` → `frontend/app/store/ws.ts` →
`agentmux-srv/src/server/websocket.rs::handle_ws`) rides a single
WebSocket whose message size is Axum's **unmodified default: 64 MiB**
(`agentmux-srv/Cargo.toml` pins `axum = "0.7"` → `axum 0.7.9` →
`tokio-tungstenite 0.24.0`, whose `WebSocketConfig::default()` sets
`max_message_size: Some(64 << 20)`). A bundle near
`MAX_TOTAL_UNCOMPRESSED_BYTES` (50 MiB) that doesn't compress well
produces a zip close to that size; base64 expands it ~4/3 to ~66.7 MiB —
over the transport ceiling — plus JSON envelope overhead on top. This is
actually a **pre-existing Phase 2 limitation** (`bundle.import` already
accepts `zip_base64` today with no wire-size cap, only a post-decode
content cap), not something Phase 3 introduces — but Phase 3 is what turns
it from a theoretical edge case into something a real UI import flow will
routinely brush up against, and its own preview+commit design would send
the same oversized payload **twice**. See §3.0.5 for the fix.

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
- Both source identifiers are **selection keys only** — the actual
  persisted *content* (the skill's slug/description/body, the MCP
  server's raw JSON config) is unchanged.

This is a small, mechanical addition to structs that already compute these
values and throw them away — not a re-design of the parser.

**codex P1 on PR #2381, round 2:** changing `mcp_servers`'s element type
breaks the existing write path silently if not called out explicitly.
Today, `bundle.rs`'s `bundle.import` handler does
`mcp_servers: serde_json::to_string(&parsed.mcp_servers)...` — serializing
the parsed value **directly** into `Memory.mcp_servers`. Once that value
is `Vec<ParsedMcpServer>` instead of `Vec<Value>`, that line would
literally persist `{source_path, config}` wrapper objects instead of raw
MCP configs, corrupting every bundle imported this way (every consumer of
`Memory.mcp_servers` — the agent-launch config builder, `bundle.export`'s
own re-export path — expects the raw config shape). **Every write site
that touches `parsed.mcp_servers` after this amendment (the retained,
now-unfiltered `bundle.import` route AND the new
`bundle.import.commit` handler) must project to `.config` before
serializing** — e.g. `parsed.mcp_servers.iter().map(|m| &m.config).collect::<Vec<_>>()`,
filtered to the selected `source_path`s for commit, unfiltered for the
existing route. This is a required part of the §3.0 amendment, not an
implementation detail left to chance.

### 3.0.5 Required transport fix: read the picked file server-side by path, not over the WebSocket

**codex P1 on PR #2381, round 3** (verified against the pinned dependency
versions — see §1): every RPC, `bundle.import.preview`/`.commit` included,
travels one WebSocket whose message size is Axum 0.7's unmodified default
of 64 MiB. A near-cap bundle base64-encodes to ~66.7 MiB — over that
ceiling — and this spec's own preview-then-commit design would hit it
**twice**.

**Fix:** `bundle.import.preview` and `bundle.import.commit` both gain a
third input option, `file_path: string` — a local filesystem path,
resolved and read **server-side**. This is not a workaround; it's the
natural shape for this specific flow, because the file was *already*
local: it came from `show_open_bundle_dialog` (§4 Step 1), which returns a
path, not bytes. Reading it server-side means the frontend never encodes
or ships the file's bytes over RPC at all for this flow — it sends the
path once to preview, the same path again to commit (still stateless, still
no caching token — just a few dozen bytes instead of tens of megabytes
each time).

This mirrors an existing precedent in this exact codebase:
`/agentmux/stream-local-file` (`agentmux-srv/src/server/mod.rs`,
`files::handle_stream_local_file`) already reads an arbitrary server-local
path and streams it via a dedicated HTTP route, entirely outside the WS
RPC transport, precisely because this is a desktop app where the backend
sidecar and the file the user just picked are on the same machine — not a
remote server receiving an untrusted upload. `bundle.import`'s new
`file_path` input is the same trust model, applied on the read side of a
request instead of a dedicated streaming response.

Validation for `file_path`: the path must exist and be a regular file
(reject directories / device files with a clear error, not a panic); no
extension allowlist is enforced server-side beyond that
(`show_open_bundle_dialog`'s own filter is the only `.abf` gate, and it's
advisory — a non-.abf file server-side just fails
`unzip_bundle_import`'s "not a valid zip archive" check same as today).

**codex P2 on PR #2381, round 6: "open the file, then check its
metadata" (as fixed in round 5, below) does NOT reject symlinks.**
`std::fs::File::open` transparently follows a symlink and hands back a
handle to — and metadata describing — the **target**, not the symlink
itself; there is no point in that sequence where "this path was a
symlink" is ever visible to check. Round 4's stated requirement to reject
symlinks was therefore never actually enforced by round 5's fix. **Fix:
open with an explicit no-follow flag**, atomically, so a symlink fails to
open at all rather than being silently resolved — `OpenOptionsExt::
custom_flags(O_NOFOLLOW)` on Unix (`std::os::unix::fs::OpenOptionsExt`);
the equivalent Windows mechanism needs confirming against this crate's
actual target platforms at implementation time (Windows symlinks require
elevated privileges to create by default, which narrows but doesn't
eliminate the risk — do not skip the check on the assumption it does).
This still composes with round 5's single-handle size check below: one
no-follow open call, then metadata + bounded read from that same handle,
still no second path resolution.

**codex P1 on PR #2381, round 4: the on-disk size must be checked BEFORE
reading, not after.** `unzip_bundle_import`'s caps (`MAX_ENTRY_COUNT`,
`MAX_ENTRY_UNCOMPRESSED_BYTES`, `MAX_TOTAL_UNCOMPRESSED_BYTES`) all apply
to *decompressed content*, evaluated only once the zip is already open and
being read — they do nothing to bound the file-path intake step itself. A
`file_path` pointing at a multi-gigabyte file (the picker's extension
filter is advisory, and this is server-side validation regardless of what
picked it) would be fully read into memory before any of those checks
ever run, since there's no size gate on the read call itself.

**codex P2 on PR #2381, round 5: a separate metadata-check-then-open is
itself a TOCTOU gap.** An earlier draft of this fix called
`std::fs::metadata(path)` and then, on a *later*, *separate* call,
re-resolved the same path to actually read it — leaving a window where
another local process could grow or replace the file in between,
defeating the size bound entirely. **Fix: open the file exactly ONCE and
never re-resolve the path.** `std::fs::File::open(path)` first; call
`.metadata()` on **that open handle** (not a fresh path lookup) and reject
if it exceeds `MAX_ABF_FILE_SIZE_BYTES` (100 MiB — generous headroom above
`MAX_TOTAL_UNCOMPRESSED_BYTES`'s 50 MiB for compression/container overhead
and imperfectly-compressed content); then read from that same handle
through a bounded reader that rejects one byte past the cap regardless of
what the metadata claimed — `handle.take(MAX_ABF_FILE_SIZE_BYTES + 1)`,
the identical hard-backstop-read pattern `unzip_bundle_import`'s own
per-entry decompression already uses one layer down (`bundle_import.rs`'s
`check_entry_size` / the `entry.by_ref().take(...)` call it guards). One
handle, one continuous read, no second path resolution for anything to
race against.

Once past that gate, the read bytes go through the **exact same**
`unzip_bundle_import`/content-cap pipeline as `zip_base64` today — no
change to that pipeline's own logic, just a new, bounded way to get bytes
in front of it.

**codex P1 on PR #2381, round 4 (generalized per codex P2, round 5):
commit must be bound to the exact bytes preview showed, for every input
mode, not just `file_path`.** A `file_path` is a live pointer to a mutable
resource — the file on disk can be overwritten between the preview call
and the commit call — and `source_dir`/`source_path` (§3.0) don't protect
against this: a replacement archive can keep identical source paths while
changing every skill's body, the instructions, or an MCP server's
executable config. The original round-4 fix scoped the digest requirement
to `file_path` only, reasoning that `zip_base64`/`files` callers "already
hold the bytes and resend them by construction" — codex round 5 correctly
points out that's an assumption about caller behavior the API does
nothing to enforce; a caller can rebuild, edit, or accidentally substitute
its payload between the two calls just as easily. **Fix: the digest
contract applies uniformly to all three input modes, not just
`file_path`.**

- `preview`'s response **always** includes `content_digest` — SHA-256,
  hex-encoded, over a canonical representation of whatever input was
  given:
  - `file_path` → hash of the raw file bytes read (round 4, unchanged).
  - `zip_base64` → hash of the **decoded** raw zip bytes (not the base64
    text itself, so identical zip content produces the same digest
    regardless of transport encoding).
  - `files` → **codex P1 on PR #2381, round 6: naively sorting the raw
    input by path is wrong.** An earlier draft did exactly that — but
    `parse_bundle_import`'s own `by_path` construction (Phase 2, round 4)
    *normalizes* each path first and, for two entries that normalize to
    the same key (e.g. `"armory.json"` and `"./armory.json"`), keeps
    **whichever appears first in the input array** and discards the rest
    with a warning. A naive path-sort can make two *differently-ordered*
    request bodies hash identically while the parser would actually
    import *different* content from each (whichever happened to be
    first) — defeating the entire point of the digest. **Fix: hash the
    parser's own effective, order-resolved representation, not the raw
    input.** Run the same normalize-then-first-wins reduction
    `parse_bundle_import` already performs to build its deduped map,
    *then* sort that deduped map by its normalized key and feed
    `len(normalized_path) || normalized_path || len(content) || content`
    for each surviving (winning) entry, in that sorted order, into the
    hash. This makes the digest a true reflection of "what will actually
    be imported" — order-independent for genuinely equivalent inputs,
    but sensitive to any input-array reordering that would actually
    change the parser's first-wins outcome. (This only reads
    `bundle_import.rs`'s existing normalize-then-dedup logic to compute a
    digest; it does not change that logic's own already-merged,
    already-reviewed first-wins behavior — that stays exactly as PR
    #2379 shipped it.)
- `commit`'s request **always** requires `expected_content_digest`,
  compared against a freshly-recomputed digest of whatever `commit`
  itself was given, using the identical per-mode canonicalization above.
  **codex P2 on PR #2381, round 6: commit must use the SAME input mode
  preview used.** An earlier draft claimed a preview/commit pair could
  freely mix input modes (e.g. preview via `file_path`, commit via
  `files`) as long as the digest matched — but `file_path`/`zip_base64`
  hash **raw zip bytes** while `files` hashes a **canonicalized list of
  already-extracted entries**; these are structurally different
  representations that never produce equal digests for the same
  underlying content, so that claim was actually unimplementable, not
  merely undocumented. **Fix: commit must supply the same input field
  (`file_path`/`zip_base64`/`files`) preview used** — the modal's own
  flow (§4) always uses `file_path` for both calls anyway, so this is a
  restriction on a capability nothing actually needs, not a loss of
  function. A mode mismatch, or a same-mode digest mismatch, both produce
  the identical hard-rejection error (the "re-select and preview again"
  message) — the API doesn't need to distinguish the two cases for the
  caller, only refuse to proceed on either.

  **codex P2 on PR #2381, round 7: stating the "same mode" requirement in
  prose doesn't enforce it — the digest itself has to carry that
  information, or nothing actually checks it.** `file_path` and
  `zip_base64` both canonicalize to the exact same thing (the raw zip
  bytes), so a `file_path` preview and a `zip_base64` commit of the same
  underlying archive produce an **identical** `content_digest` — the
  round-6 fix's own stated mode-mismatch case would silently pass a bare
  byte-digest comparison, exactly the scenario it was meant to reject.
  **Fix: mix a mode tag into the hash domain itself**, so the digest is a
  function of `(mode, canonical bytes)`, not bytes alone — hash
  `mode_byte || canonical_representation` where `mode_byte` is a fixed,
  distinct constant per input type (`0x01` for `file_path`, `0x02` for
  `zip_base64`, `0x03` for `files`). Two different modes now produce
  different digests for the identical underlying content by construction,
  closing the gap without adding a separate mode field for the mismatch
  check to drift from — the digest comparison alone is sufficient again,
  as originally intended.

`zip_base64` and `files` remain valid inputs on both new RPCs (unchanged
from today's `bundle.import`) for callers that don't have a local path —
e.g. a bundle received over the network by some future integration. Their
existing WS-transport exposure (§1's own limit) is a **pre-existing,
separately-tracked Phase 2 limitation** this spec does not resolve for
those input modes; it only ensures the flow this spec actually builds
(§4's modal, which always has a local path in hand) never depends on it.

### 3.1 `bundle.import.preview`

**Request** — today's `bundle.import` shape, plus `file_path` (§3.0.5,
the one the modal actually uses):
```jsonc
{ "file_path": "C:\\Users\\...\\bundle.abf" }
// or, unchanged from today: { "zip_base64": "..." }
// or: { "files": [{ "path": "...", "content": "..." }] }
```

**Response:**
```jsonc
{
  "name": "Backend Dev Bundle",
  "description": "...",
  "instructions_preview": "Be concise. Prefer existing patterns...",
  "context_files": [
    { "id": 0, "display_path": "conventions.md", "size_bytes": 39 }
  ],
  "skills": [
    { "source_dir": "skills/deploy-checklist", "slug": "deploy-checklist", "description": "...", "collision": "none" },
    { "source_dir": "skills/code-review-v2", "slug": "code-review", "description": "...", "collision": "name_conflict" },
    { "source_dir": "skills/code-review-old", "slug": "code-review", "description": "...", "collision": "duplicate_in_bundle" }
  ],
  "mcp_servers": [
    { "source_path": "mcp/github.server.json", "display": { "name": "github", "command": "npx" } }
  ],
  "requirements": [
    { "id": "req-1", "provider": "github", "env": "GITHUB_TOKEN", "resolved": false, "match_count": 0 }
  ],
  "warnings": [ "components.instructions: ... skipped" ],
  "warnings_truncated": false,   // true if the warnings list itself was capped (§3.1, round 8)
  "name_collision": false,   // true if an existing bundle already has this exact name (soft, informational)
  "instructions_truncated": false,   // true if instructions_preview was cut short (§3.1)
  "instructions_total_chars": 27,   // full (untruncated) instructions length, always present (§3.1, round 8)
  "content_digest": "8f14e45f..."   // SHA-256, canonical per input mode; required back at commit for EVERY input mode (§3.0.5, round 5)
}
```

`source_dir`/`source_path` are the §3.0 selection keys — always present,
always unique per row, independent of whatever the row's own `slug`/JSON
`name` field says. **codex P2 on PR #2381, round 7: `mcp_servers[].config`
must not be returned verbatim in preview.** An earlier draft passed the
full parsed `config` JSON through for display — but a parser-accepted
bundle can put a large share of its content budget into an MCP server's
JSON blob (arbitrary content, no size limit beyond the shared aggregate
cap), so returning every full `config` risks the same "supposedly
lightweight response actually approaches the aggregate cap" problem
`instructions_preview`'s own cap (below) already exists to prevent — the
modal only ever displays a server's name and command, never the full
config. **Fix:** preview returns a bounded `display` projection instead
of `config` — `{ name: string | null, command: string | null }`,
extracted defensively from whatever fields happen to be present (MCP
JSON has no required shape, per §3.0), each truncated to a small fixed
character cap (e.g. 200 chars) so even a maliciously oversized `name`/
`command` string can't reintroduce the problem. The UI falls back to
`source_path`'s basename when `display.name` is absent. The full,
untruncated `config` is never lost — it's read from the freshly-parsed
source at commit time (§3.2), exactly like `instructions`'s own full
value; only the preview response is bounded.

Implementation notes:
- **Found during this spec's own proactive re-audit after round 12 (not
  a codex finding) — two more fields in the identical unbounded class:**
  `name`/`description` (the bundle-level manifest fields — confirmed
  against `parse_bundle_import`'s actual code: `manifest.get("name")`/
  `.get("description")` have no length check at all) and
  `context_files[].path` (confirmed against `sanitize_context_relative_path`:
  it normalizes and rejects unsafe paths but never caps component or
  total length, and `capped_component_array`/`MAX_ENTRY_COUNT` bound the
  *number* of context files, not any one path's length).

  **codex P2 on PR #2381, round 13: truncating `context_files[].path` for
  display breaks selection, unlike skills' `slug`.** `include_context_files`
  (§3.2) is a list of *paths*, matched against the freshly-parsed FULL
  paths at commit — but a truncated `path` in preview is the only value
  the frontend has to build that list from, so a checked row whose path
  exceeded the cap would submit the shortened value, fail to match
  anything server-side, and be **silently dropped from the import** with
  no error. Skills don't have this problem because `source_dir` (the
  actual selection key) and `slug` (the truncatable display value) are
  already two separate fields (§3.0) — `context_files` never got that
  same split. **Fix: give context files the identical split.** Each
  entry gains a stable `id` (its 0-based index within *this* parse's
  `context_files` list — deterministic and reusable across `preview` and
  `commit` precisely because `expected_content_digest` already guarantees
  both calls parse identical content, so the same index always means the
  same entry); `path` is renamed `display_path` and stays bounded/
  truncated exactly as before, now purely for rendering. `commit`'s
  `include_context_files` becomes a list of `id` values, not paths — the
  frontend never needs the full untruncated path for anything, closing
  the gap the same way `source_dir` already does for skills, without
  needing to preserve a "full path" anywhere client-side at all.

  **codex P2 on PR #2381, round 13: truncating `name` for *display only*
  is unsafe here, unlike `description`.** §4 Step 2 pre-fills the
  editable bundle-name field from preview's `name` and, if the user makes
  no edit, submits that same value back as `bundle_name` — which round
  11's fix now genuinely applies verbatim to `Memory.name`. If `name`
  were truncated for display only while "the full value" is used
  somewhere else, a user who never touches the field would silently
  import under a shortened name, since the frontend was never given the
  full value to preserve. **Fix: bound `name` at the parse source, not at
  the response boundary.** `parse_bundle_import` itself truncates an
  oversized manifest `name` (with a warning, matching the existing
  "warn, don't silently guess" fallback already used when `name` is
  absent) — so the *canonical* name, from the moment parsing completes,
  is already the bounded value, identically for both `preview` (which
  displays it) and `commit` (which re-parses independently and would
  derive the identical canonical name if `bundle_name` were somehow
  omitted). There is no separate "real, longer" name floating around
  anywhere for the truncated display value to lose — genuinely
  pathological manifest names aren't a legitimate case worth preserving
  byte-for-byte, unlike free-form content such as `description`/
  `instructions`. `description` keeps the simpler response-boundary
  truncation from earlier in this list — it's pure display, never
  re-submitted by the user, so it doesn't share this failure mode.
- `parse_bundle_import` already produces everything here except
  `collision`/`name_collision`/the requirement `resolved`/`match_count`
  fields — three new, read-only additions the RPC handler makes:
  - **Skill collisions**: `collision` is computed in two passes, not one
    (codex P1 on PR #2381, round 2 — the first draft only checked the
    global catalog, so two skills *within the same bundle* sharing a slug
    that happens not to already be global were both marked `"none"`,
    letting the user check both; commit would then write the first and
    silently warn+skip the second with no rename ever offered):
    1. **Against the existing catalog**: one call to `skill.catalog.list`'s
       underlying `wstore.skill_list_global()`
       (`agentmux-srv/src/server/app_api/skill.rs`,
       `register_skill_catalog_list`), name-matched against each parsed
       skill's slug → `"name_conflict"`. **Not** `skill.list` (agent-scoped,
       requires `agent_id`) and **not** a `skill.list_global` command —
       that command doesn't exist (codex P1 on PR #2381, round 1).
    2. **Within the bundle itself**: group the parsed skill list by `slug`;
       every entry whose slug appears more than once, and that wasn't
       already flagged `"name_conflict"` in pass 1, is flagged
       `"duplicate_in_bundle"` instead (distinct from `"name_conflict"` so
       the UI can word the hint differently — "another skill in this
       import uses this name" vs. "already exists in your library" — the
       resolution is the same either way, see §4.1).
  - **codex P2 on PR #2381, round 10: `skills[].description` must be
    bounded too**, the same class of gap as `instructions_preview`
    (round 5), `mcp_servers[].config`/`display` (round 7), and `warnings`
    (rounds 8–9) — a fourth field nobody had capped yet. SKILL.md's
    frontmatter `description` field has no length limit of its own; with
    up to `MAX_IMPORTED_SKILLS` (200) skills and up to
    `MAX_ENTRY_UNCOMPRESSED_BYTES` (10 MiB) allowed per entry, a bundle
    that devotes most of its content budget to descriptions rather than
    bodies could still return tens of megabytes of description text.
    Unlike `instructions_preview` (one long free-form field worth showing
    a "truncated — N total characters" note for), skill descriptions are
    meant to be short one-line summaries by Agent Skills convention — a
    straightforward fixed-character truncation (e.g. 300 chars, with an
    ellipsis when cut, no separate boolean/total-count field needed) is
    proportionate here rather than replicating `instructions_preview`'s
    full truncation-metadata pattern. The full, untruncated description
    is unaffected at commit time, same as every other preview-only cap.
  - **codex P2 on PR #2381, round 12: `skills[].slug` needed the identical
    treatment and was missed** — `parse_skill_md` places no length limit
    on SKILL.md's `name:` frontmatter either, so a bundle with several
    oversized `name` values could still return tens of megabytes via
    `slug` even with `description` already capped. Fix: `slug` gets the
    same fixed-character truncation as `description` in preview's
    `skills[]`. `source_dir` (§3.0) remains the actual selection/rename
    key regardless — nothing about matching a checkbox or a rename to the
    right row depends on `slug`'s display length. `commit`'s `import_as`
    handling (§3.2) is validated against the freshly-parsed, full-length
    slug from `source_dir` regardless of what preview happened to
    display; nothing about the actual write decision depends on the
    truncated display value. **(codex P2 on PR #2381, round 14: this
    passage previously also said §4.1's rename input is "pre-filled"
    from this truncated `slug` and called that "fine" — corrected here.
    It is not fine, and round 13 changed the actual UI contract:** §4.1
    point 5 requires the rename input to **start blank**, with `slug`
    shown only as adjacent read-only label text, precisely because a
    pre-filled truncated value could get silently submitted as
    `import_as` without the user ever typing anything. This passage's
    only remaining claim is the narrower one above — the *write
    decision* never depends on the truncated display value — not
    anything about how the input is initialized, which §4/§4.1 alone now
    govern.)
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
  - **codex P2 on PR #2381, round 11: `requirements[]`'s `id`/`provider`/
    `env` strings must be bounded too** — a fifth field in the same class
    as skill descriptions (round 10). `MAX_ACCOUNT_REQUIREMENTS` (1,000)
    bounds the *count* of requirements and `MAX_ENTRY_UNCOMPRESSED_BYTES`
    (10 MiB) bounds `accounts/requirements.json`'s own file size, but
    neither bounds the length of an *individual field* within it — a
    valid 10 MiB requirements document could concentrate its content into
    oversized `id`/`provider`/`env` strings across up to 1,000 entries,
    and control-character JSON escaping compounds that the same way it
    does for `instructions_preview`. Fix: each of these three fields gets
    the same fixed-character truncation as skill descriptions when
    included in preview's `requirements[]`; the full, untruncated values
    are what the existing `id_store.identity_list` resolution and the
    commit write path actually use — this only bounds what's echoed back
    for display.
- **codex P1 on PR #2381, round 5:** an earlier draft returned the full
  instructions string untruncated, reasoning "the size cap already bounds
  this to something reasonable" — that's the wrong cap to reason from. A
  parser-accepted bundle can put nearly all of its
  `MAX_TOTAL_UNCOMPRESSED_BYTES` (50 MiB) budget into `instructions`
  alone, and JSON string-escaping can expand pathological content (NUL
  bytes, control characters — each becomes a 6-character `\u00XX`
  escape) up to 6×, so the RESPONSE could approach ~300 MiB — which
  would itself blow the same 64 MiB WS ceiling §3.0.5 just fixed on the
  request side, just on the way back out instead. Fix:
  `instructions_preview` is capped server-side to a fixed
  `MAX_INSTRUCTIONS_PREVIEW_CHARS` (50,000 characters — generous for a
  glance-and-decide preview; real instructions are almost always a few KB
  to a few tens of KB, and this bounds worst-case JSON-escaped output to
  ~300 KB, not ~300 MB). When truncated, the response's
  `instructions_truncated` field is `true` and the modal shows an
  "instructions truncated for preview — N characters total" note rather
  than silently presenting a partial document as complete. This caps the
  *preview* response only; the *committed* bundle's `instructions` field
  is written from the full, untruncated parsed value at commit time,
  exactly as today.
  - **codex P2 on PR #2381, round 8:** the spec promised that "N
    characters total" note but never actually specified a field carrying
    the count — `instructions_preview` alone can't yield it once the
    remainder has been discarded server-side. Fix: response always
    includes `instructions_total_chars` (the full, untruncated character
    count — present regardless of whether truncation happened, so the
    modal can show it unconditionally rather than branching on
    `instructions_truncated` for two different pieces of information).
- **codex P1 on PR #2381, round 8: `warnings` itself was never bounded,
  the same unbounded-response class as `instructions_preview` (round 5)
  and `mcp_servers[].config` (round 7).** An `.abf` containing thousands
  of unsafe/malformed entries (`MAX_ENTRY_COUNT` allows up to 10,000)
  makes `unzip_bundle_import`/`parse_bundle_import` push one warning
  string per bad entry, each embedding the entry's own raw name — and zip
  entry names aren't length-capped anywhere in this module, so the
  accumulated (and then JSON-escaped) warning text could reach hundreds
  of megabytes, blowing the same 64 MiB WS ceiling on the way out that
  the other two caps exist to prevent.

  **codex P2 on PR #2381, round 11: capping only at the RPC response
  boundary is too late — the fix must live at the parser's own
  warning-push sites, not just at serialization.** The round-8 design
  (as originally written: "applied at the RPC response boundary instead
  of within the parser") still lets `unzip_bundle_import`/
  `parse_bundle_import` build the ENTIRE unbounded `Vec<String>` — every
  full, untruncated warning, for every one of up to 10,000 entries — in
  memory first; only *after* that complete (potentially hundreds-of-MB)
  allocation finishes does a response-boundary projection trim it down.
  The backend can exhaust memory/time building the full list long before
  any capping ever runs. **Fix: enforce the cap where warnings are
  actually produced.** `parse_bundle_import`/`unzip_bundle_import`/
  `enforce_raw_files_caps` accept an explicit, optional warning-budget
  parameter (max individual string length + max total count) instead of
  pushing directly into a bare, unbounded `Vec<String>`; once the budget
  is exhausted, further warnings are dropped (with a trailing summary
  entry) at the moment they'd be produced, not retained-then-trimmed.
  **The existing `bundle.import` route passes an effectively-unbounded
  budget** (matching today's real, already-shipped, already-reviewed
  behavior exactly — this is a capability the parser gains, not a
  behavior change forced onto Phase 2's existing caller); the new
  `preview`/`commit` handlers pass the tight budget from the round-8
  design. One shared enforcement point, two different budgets per
  caller — not "cap everything, always" and not "cap only after the
  fact."
- Same `MAX_ENTRY_COUNT`/`MAX_ENTRY_UNCOMPRESSED_BYTES`/
  `MAX_TOTAL_UNCOMPRESSED_BYTES`/`MAX_ACCOUNT_REQUIREMENTS`/
  `MAX_IMPORTED_SKILLS` caps from Phase 2 apply unchanged — preview reuses
  `parse_bundle_import`/`unzip_bundle_import`/`enforce_raw_files_caps`
  as-is, just skips the Store-write half of today's handler.

### 3.2 `bundle.import.commit`

**Request** — the same file payload, plus selections:
```jsonc
{
  "file_path": "C:\\Users\\...\\bundle.abf",   // or zip_base64/files — same as preview (§3.1), re-sent, not a cache token
  "expected_content_digest": "8f14e45f...",    // required for EVERY input mode (§3.0.5, round 5); commit rejects on any mismatch
  "bundle_name": "Backend Dev Bundle (2)",   // user-editable, defaults to the parsed name
  "include_instructions": true,
  "include_context_files": [0],       // context_files[].id values to include (round 13 -- not paths; see §3.1); omitted id = excluded
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
it's the same underlying write path, just filtered to the selection —
**with one governing rule (codex P2 on PR #2381, round 12, generalizing
round 9's warnings-specific fix): every bounded-display projection this
spec defines for `preview` applies identically, by the same shared
helper, to the equivalent field in `commit`'s response.** Round 9 already
had to catch and fix this once for `warnings` specifically (applied to
`preview` only, forgotten for `commit`); round 12 found the same drift
recurring for `skipped_skills` (skill names can be as long as `slug`,
round 12's own finding, if the parser accepted them before the write
loop hit a collision) and for `resolved_requirement_ids`/
`unresolved_requirements` (the same unbounded `id`/`provider`/`env`
fields §3.1's round-11 fix bounds in `preview`, never bounded here).
Stating this as one rule instead of patching each field independently
again is deliberate — see the shared-helper note at the end of this
subsection. Concretely: `skipped_skills` entries and
`unresolved_requirements`' `id`/`provider`/`env` values get the same
fixed-character truncation `preview` already applies to the equivalent
data (skill names via `slug`'s truncation, requirement fields via
§3.1's round-11 truncation); `resolved_requirement_ids` gets the same
treatment for consistency, even though a resolved requirement's `id` is
less likely to be pathologically long in practice. None of this affects
what was actually written — `bundle_id` and the underlying Store rows are
already committed by the time these projections run; only what's echoed
back is bounded. `warnings`/`warnings_truncated` additionally use the
**identical bounded-warning treatment §3.1 (round 8/11) applies to
preview**, not today's unbounded `bundle.import` shape.

**codex P1 on PR #2381, round 9: the round-8 warnings cap was applied to
`preview` only — `commit`'s response still described "the same shape as
today's `bundle.import`," i.e. unbounded.** This is worse on the commit
side than the preview side: commit's Store writes (creating the bundle
row, the skill rows) can **succeed**, and only *then* does response
serialization/transmission fail against the same 64 MiB WS ceiling —
the caller sees a failure or disconnect for an import that actually
went through, and a plausible retry (nothing about `bundle.import.commit`
is idempotent; it always mints a fresh bundle UUID) creates a **second,
duplicate bundle** rather than merely losing some diagnostic text. Fix:
apply §3.1's bounded-warning treatment to `commit`'s response too — the
same warning-budget parameter §3.1's round-11 revision (above) has the
parse call itself enforce, so `commit`'s call into `parse_bundle_import`/
`unzip_bundle_import` is *already* producing a bounded list before the
write loop ever starts, not after. The write loop then appends its own,
separate, small number of additional warnings (skill-conflict
"already exists" messages — bounded by `MAX_IMPORTED_SKILLS`, 200, far
below the parse-time danger zone of up to 10,000 unsafe entries), and one
final pass applies the same per-warning-length/array-count bound to the
*combined* list before serializing the response — now a cheap
consistency/defense-in-depth step over an already-small input, not the
primary defense. This is exactly the kind of drift the spec's own
account-requirement-resolution fix (§3.1's implementation notes,
requirement resolution bullet) already called for extracting into a
shared helper to prevent. **codex P2 on PR #2381, round 12: generalize
this beyond warnings.** Implement every bounded-display projection this
spec defines — instructions truncation, MCP display projection, skill
description/slug truncation, requirement field truncation, and the
warnings budget — as **shared functions `preview` and `commit` both
call**, never independent per-endpoint copies. This is the second time a
"fixed in preview, forgotten in commit" gap has had to be found and
patched a round later (`warnings` in round 9, `skipped_skills`/
`resolved_requirement_ids`/`unresolved_requirements` in round 12,
directly above) — a shared-function requirement stated once, up front,
for every projection this spec defines is what actually closes that
class of drift, rather than continuing to re-discover it field by field.

Implementation notes:
- For every input mode (§3.0.5, round 5), the handler resolves the input
  to bytes, hashes them, and compares against `expected_content_digest`
  **before** doing anything else — a mismatch is a hard rejection with a
  clear "re-select/re-fetch and preview again" error, not a partial
  import against whatever content was actually given. When `file_path` is
  used, the same on-disk `MAX_ABF_FILE_SIZE_BYTES` check (via the
  single-handle open-then-bounded-read pattern, §3.0.5) applies here too,
  independently — commit re-reads the file from scratch; it never trusts
  preview's prior read or size check.
- The commit handler re-runs `parse_bundle_import` (or reuses a shared
  internal parse helper) against the freshly-sent bytes — it does **not**
  trust client-supplied preview data for anything that gets written. The
  selections (`include_*`) are a filter applied to the freshly-parsed
  result before the existing write loop runs; the write loop's own
  per-item logic (per-skill `skill_upsert_unique_global` call, conflict →
  warn+skip, rollback on infra failure) is otherwise unchanged.
- **codex P2 on PR #2381, round 11: `bundle_name` must actually be
  substituted — an earlier draft never said so, so the write loop as
  described would keep constructing `Memory.name` from `parsed.name`
  regardless of what the request's `bundle_name` said.** This makes the
  entire rename-before-import UX (§4 Step 2's editable name field, and
  its one-click `"<name> (2)"` suggestion for a soft name collision)
  silently inert — every import would keep the archive's original name no
  matter what the user typed or accepted. **Fix: the commit handler uses
  the request's `bundle_name` for `Memory.name` when constructing the
  bundle row — never `parsed.name` directly** (mirrors `import_as`'s
  override of a skill's parsed slug, immediately below, which already got
  this right for skills specifically; `bundle_name` needs the identical
  treatment at the bundle level). `bundle_name` defaults to the parsed
  name client-side when the field is first populated (§3.1's `name`), so
  a commit that never edited the suggested value still round-trips
  correctly — it isn't optional at the wire level, just pre-filled.
- `include_skills`/`include_mcp_servers` filter the freshly-parsed
  `skills`/`mcp_servers` lists by matching each entry's `source_dir`/
  `source_path` (§3.0) — never by `slug` or a JSON `"name"` field, which
  aren't guaranteed unique across entries. `include_context_files` filters
  by `id` (§3.1, round 13) — the 0-based index within this parse's
  `context_files` list, never by `display_path`, which is truncated for
  display and can't be relied on to round-trip byte-for-byte.
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
- On selection: call `bundle.import.preview` with `{ file_path: <the
  picked path> }` (§3.0.5) — the frontend never reads the file's bytes or
  base64-encodes anything itself; the path is the whole payload. Store
  both the path **and** the response's `content_digest` in modal state for
  the eventual `bundle.import.commit` call in step 3 (§3.0.5's
  digest-binding requirement). Parse/validation errors (malformed zip,
  missing `armory.json`, unreadable path, oversized file) surface inline
  on this same step — don't advance.

### Step 2 — Preview & select

Renders the `bundle.import.preview` response as a checklist:

- **Bundle name** — editable text field, pre-filled with the parsed name.
  If `name_collision` is true, an inline hint ("a bundle named this already
  exists") with a one-click suggested alternate (`"<name> (2)"`,
  incrementing) — never blocking, since the backend allows duplicates.
- **Instructions** — single checkbox ("Include instructions"), checked by
  default, with a collapsible preview of `instructions_preview`. When
  `instructions_truncated` is true, an inline note ("preview truncated —
  `instructions_total_chars` characters total, the full instructions are
  still imported") makes clear this is a display limit, not a loss of
  content (§3.1).
- **Context files** — one checkbox per file (path + size), checked by
  default.
- **Skills** — one checkbox per skill (slug + description), checked by
  default. A skill whose `collision` is `"name_conflict"` (already exists
  in the global catalog) or `"duplicate_in_bundle"` (another row in this
  same import shares its slug) shows a collision badge — worded
  differently per reason — and switches its row to an **empty** text
  input (the original `slug` shown alongside as read-only label text, not
  as the input's value — round 13, §4.1 point 5), where the user types an
  alternate name to import under (empty at commit = skip, same effect as
  unchecking). This is the one item type with real collision UX; see
  §4.1.
- **MCP servers** — one checkbox per server (name + command), checked by
  default. No collision UI — per §2, there's nothing for these to collide
  with under the current backend design.
- **Account requirements** — read-only summary, no checkboxes ("Depends on
  N account(s): github (resolved), openai (not connected)"). Always
  included; nothing is written from this list regardless.
- Any `warnings` from the parse: a dismissible banner, not blocking. When
  `warnings_truncated` is true, the banner's own final entry ("N more
  warnings not shown") is enough — no separate UI treatment needed (§3.1,
  round 8).

### Step 3 — Confirm & import

- Summary line built from the current selection state (client-side count,
  no extra RPC): "Importing: instructions, 1 context file, 2 skills, 1 MCP
  server."
- "Import" button calls `bundle.import.commit` with the same `file_path`
  + the `content_digest` stashed from step 1 (as `expected_content_digest`,
  §3.0.5) + selections built from step 2's checklist state. A digest
  mismatch (the file changed since preview) surfaces as a distinct,
  clearly-worded error directing the user back to step 1 — not a generic
  failure. On success, close the modal and navigate to the new bundle
  (mirrors whatever "just-created bundle" navigation `bundle.upsert`'s own
  callers already do, if any exists — otherwise a toast + the new bundle
  appearing in the Bundles list is sufficient). On a partial failure (e.g.
  a skill got skipped server-side because of a last-second name conflict),
  surface the
  response's `warnings`/`skipped_skills` — this is a real, expected
  outcome under the race described in §3.2, not an error state.

### 4.1 Skill collision resolution, precisely

1. Preview's `collision` (`"name_conflict"` / `"duplicate_in_bundle"`) is
   computed by the two-pass logic in §3.1 — a snapshot at preview time,
   not a live constraint.
2. The modal fetches the full existing global skill-name list **once**,
   up front, via **`skill.catalog.list`** — the window-scoped, no-`agent_id`
   route Armory already uses (`register_skill_catalog_list`,
   `agentmux-srv/src/server/app_api/skill.rs`). Not `skill.list` (requires
   an `agent_id`, agent-scoped) and not `skill.list_global`, which doesn't
   exist as a command (codex P1 on PR #2381; corrected from an earlier
   draft). A typed rename is validated **client-side, instantly** against
   the **union** of that fetched global list and the current in-progress
   slugs/renames of every *other* selected skill row in this same preview
   (codex P1 on PR #2381, round 2 — validating against the global list
   alone would let a user "resolve" a `duplicate_in_bundle` collision by
   typing a name that just collides with a third row in the same import
   instead), recomputed live as any row's checkbox/rename changes. This is
   advisory only.
3. At commit, the server is the sole authority: `skill_upsert_unique_global`
   runs its own check regardless of what the client validated. A
   client-side "looks available" name can still lose a race server-side —
   handled by the existing warn+skip behavior (§3.2), surfaced in the
   commit response.
4. Leaving a colliding skill's rename field empty and its checkbox checked
   is treated as "skip this skill" at commit (equivalent to unchecking) —
   never silently sent through with its original, known-conflicting slug.
5. **codex P2 on PR #2381, round 13: the rename input must start empty,
   not pre-filled with `slug`.** An earlier draft pre-filled the rename
   text input with the skill's (possibly truncated, round 12) display
   `slug` — for a skill whose *real* slug exceeds the truncation cap,
   that pre-filled value is neither the true original name nor a
   deliberate replacement, yet importing without editing the field would
   submit it as `import_as` regardless: a colliding skill silently
   renamed to a **truncated prefix** of its own name, rather than being
   skipped or genuinely renamed. Point 4's "empty = skip" rule also can't
   distinguish "user left the pre-fill untouched" from "user
   deliberately typed nothing" once the field starts non-empty. **Fix:
   the rename input starts blank.** The skill's (possibly truncated)
   `slug` is shown as adjacent read-only label/placeholder text — context
   for what's colliding, not a value ever submitted unedited. Point 4's
   rule (empty at commit time = skip) now has no ambiguous case: a
   colliding row is either left blank (skip) or has something the user
   actually typed (`import_as`), never a server-supplied default that
   happens to look non-empty.

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
  `name_collision` computed correctly against a seeded bundle-name list;
  **`"duplicate_in_bundle"` specifically** — two parsed skills sharing a
  slug that is NOT in the global catalog both get flagged, not silently
  passed as `"none"` (the exact round-2 gap codex found). `bundle.import.commit`
  — selection filtering by `source_dir`/`source_path` (only checked items
  get written, including the case of two entries with a colliding
  `slug`/`name` but distinct source paths), `import_as` substitution, **MCP
  server persistence writing raw `config` values, not `{source_path,
  config}` wrappers** (the exact round-2 gap codex found on the retained
  `bundle.import` route too — cover both write sites), and the
  pre-existing warn+skip / rollback behavior all still hold when driven
  through a partial selection rather than "everything." Both RPCs also
  need `file_path` input tests (§3.0.5, round 4): a valid path parses
  identically to the equivalent `zip_base64`; a missing/unreadable/
  non-file path produces a clear error rather than a panic; a file whose
  on-disk size exceeds `MAX_ABF_FILE_SIZE_BYTES` is rejected via the
  single-handle open-then-bounded-read (§3.0.5, round 5 — not a separate
  metadata call), verifiable via a sparse/pre-allocated file rather than
  actually writing 100MB+ to disk in a test. `bundle.import.commit`
  rejects with a clear error — no write happens — when
  `expected_content_digest` doesn't match, for **all three input modes**
  (round 5 generalization): `file_path` (simulate by preview-ing one
  file, modifying it, then committing with the stale digest),
  `zip_base64` (commit with a digest that doesn't match the decoded
  bytes), and `files` (commit with a digest computed over a different
  canonical ordering/content). **`files` digest correctness specifically**
  (round 6): two arrays that reorder *genuinely equivalent, non-colliding*
  entries must hash identically; but two arrays that reorder entries with
  a normalized-path collision — where reordering would change *which*
  entry the parser's first-wins rule actually keeps — must hash
  **differently**, proving the digest reflects `parse_bundle_import`'s
  real dedup outcome rather than a naive raw-input sort (the exact round-6
  gap codex found). **Commit mode-mismatch** (round 6): a commit whose
  input field (`file_path`/`zip_base64`/`files`) differs from what
  preview used is rejected with the same hard error as a digest mismatch,
  never silently accepted. **Symlink rejection** (round 6): a `file_path`
  pointing at a symlink is rejected outright, not silently followed to
  its target — this needs a platform-appropriate test (Unix: an actual
  symlink via `std::os::unix::fs::symlink`; confirm the Windows
  equivalent at implementation time per §3.0.5's own note there).
  `instructions_preview` is capped at `MAX_INSTRUCTIONS_PREVIEW_CHARS`
  with `instructions_truncated: true` set when a real bundle's
  instructions exceed it, and the committed bundle's actual
  `instructions` field is unaffected by the preview cap (round 5).
  **Digest mode-binding** (round 7): a `file_path` preview and a
  `zip_base64` commit of the exact same underlying archive bytes must
  produce **different** `content_digest`/`expected_content_digest`
  values (proving the mode tag is actually mixed into the hash, not just
  documented as a requirement) and the commit must be rejected. **MCP
  preview projection** (round 7): `mcp_servers[].display` never contains
  the full `config` — only bounded `name`/`command` strings — even when
  the source MCP JSON is large or has an oversized `name`/`command`
  value; the full `config` still reaches the write path correctly at
  commit time, unaffected by the preview projection. **Bounded warnings**
  (round 8): an archive with more than the warnings-array count cap worth
  of unsafe/malformed entries produces a response with `warnings_truncated:
  true`, at most the capped count of entries, and no individual warning
  string longer than its character cap — even when constructed from
  entries with deliberately oversized raw names. **`instructions_total_chars`**
  (round 8): present and equal to the true full length on every preview
  response, not just truncated ones, verified against both a short
  (non-truncated) and a long (truncated) instructions string.
  **Commit response warnings bounded too** (round 9): the same warnings-
  count-and-length archive used for preview's bounded-warnings test,
  driven through `bundle.import.commit` instead, produces an equally
  bounded `warnings`/`warnings_truncated` in the commit response — and
  the underlying Store writes (the bundle row, any skill rows) still
  reflect the full, untruncated parsed data regardless of what the
  response reports back. **Bounded skill descriptions** (round 10): a
  skill whose SKILL.md `description` frontmatter exceeds the truncation
  cap gets a shortened `description` in preview's `skills[]`, and the
  full description is still what's actually written to the `Skill` row
  at commit time. **Bounded requirement fields** (round 11): a
  requirement with an oversized `id`/`provider`/`env` string gets
  truncated versions of those three fields in preview's `requirements[]`,
  while the full values are what the existing account-resolution lookup
  and the commit write path actually use. **`bundle_name` is actually
  applied** (round 11): a commit whose `bundle_name` differs from the
  archive's own parsed name produces a `Memory` row with the
  **requested** name, not the archive's original one — the exact
  functional gap codex found (an earlier draft's write loop would have
  silently ignored the field). **Warnings capped during accumulation, not
  just at the response boundary** (round 11): an archive with thousands
  of unsafe entries must not cause `parse_bundle_import`/
  `unzip_bundle_import` themselves to allocate the full unbounded warning
  list — this needs a test at the *parser* level (not just the RPC
  response level) proving the budget parameter actually bounds
  accumulation during parsing, e.g. asserting the returned `Vec<String>`
  length/total-bytes never exceeds the budget even when called directly,
  independent of any response-boundary projection. **Bounded skill
  slugs** (round 12): the same treatment as skill descriptions, applied
  to `slug` — and the rename flow (§4.1) still works end-to-end when a
  skill's true slug is longer than the truncation cap (the commit
  `import_as` path is validated against the full-length parsed slug, not
  the truncated display value). **Commit response reuses preview's
  bounds** (round 12): an archive whose skipped skills or unresolved
  requirements would carry oversized display fields produces the same
  bounded values in `commit`'s response as `preview`'s equivalent fields
  — proving the shared-helper requirement actually holds, not just that
  each endpoint independently happens to cap its own fields the same
  way. **Bundle description/context-file content bounded** (round 12
  self-audit): a bundle whose manifest `description` exceeds the
  truncation cap produces a bounded value in preview's response, while
  the actual `Memory.description` is written from the full, untruncated
  parsed value at commit time; a `context_files[]` entry's actual
  persisted content/path is likewise unaffected by `display_path`'s
  truncation. **`name` is bounded at parse time, identically for preview
  and commit** (round 13 correction of round 12's initial approach): a
  manifest `name` exceeding the length cap produces the *same* truncated
  value from `parse_bundle_import` whether called by `preview` or by
  `commit` (re-parsing independently) — proving there's no separate
  "full" name anywhere for a display-only truncation to lose, and that a
  commit whose `bundle_name` was never edited from preview's suggestion
  still imports under a name consistent with what preview showed.
  **Context-file selection survives truncation** (round 13): a
  `context_files[]` entry whose `display_path` was truncated is still
  correctly included/excluded via `include_context_files`'s `id` value —
  proving selection doesn't silently drop truncated-path entries the way
  an earlier draft's path-based selection would have. **Skill rename
  input never auto-submits a truncated slug** (round 13): committing a
  colliding skill's row with its rename field left exactly as the UI
  presents it (no `import_as`, or an empty one) results in that skill
  being skipped, never imported under a truncated version of its own
  slug — this needs an explicit test that a *non-empty but unedited*
  scenario cannot occur given the "starts blank" UI change, i.e. that the
  wire contract has no way to represent "the default" as distinct from
  "the user typed this," by construction.
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
