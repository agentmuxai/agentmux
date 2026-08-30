# SPEC: Global Memory "system" tier — an AgentMux-controlled, highest-priority entry

**Date:** 2026-08-24
**Status:** implemented 2026-08-24. All of §3.1-§3.5 shipped as designed.
Rust: 106/106 `store::tests` pass (7 new), full 2763-test suite green.
Frontend: `npx tsc --noEmit` clean, full 3060-test vitest suite green (7
new). Manual live-pane verification (§5, "Manual") not done as part of this
PR — no live dev instance was available; do before/at merge if practical,
same caveat as the sibling dedent PR (#2780).
**Scope:** `agentmux-srv/src/backend/storage/migrations.rs`,
`agentmux-srv/src/backend/storage/memory_bundles.rs`,
`agentmux-srv/src/backend/rpc_types/{commands,memory}.rs`,
`agentmux-srv/src/server/agent_handlers/memory.rs`,
`agentmux-srv/src/server/app_api/agent_open.rs` (no code change expected —
verify), `frontend/types/gotypes.d.ts`,
`frontend/app/store/rpc-api/memory.ts`,
`frontend/app/view/brain/global-brain-model.ts`,
`frontend/app/view/brain/global-brain-manager.tsx`.

---

## 1. Report

Every agent already inherits two layers of instructions today: their own
Bundle's `instructions`, and every **Global Memory** entry (`db_bundles` rows
with `is_global=true`, editable by any human at the Armory "Memory" tab —
see `docs/specs/SPEC_ARMORY_MEMORY_GLOBAL_PERSONAL_RENAME_2026_08_22.md` for
the current Global/Personal split). Both are concatenated into
`CLAUDE.md`/`.claude/AGENTMUX_MEMORY.md` at launch with **no priority
concept** — just concatenation order (Soul → AgentMD → Memory → Skills
index; see `agent_config.rs:54-117`) and an ownership binary (AgentMux owns
the file outright, or is relegated to a single `@import` line in a foreign
`CLAUDE.md` — `docs/specs/SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md`).

Separately, this very deployment already runs a working, informally
"highest priority" instructions file: `~/.agentmux/agents/CLAUDE.md`, a
parent-directory file every agent inherits via Claude Code's own
directory-walk-up discovery, carrying policy content (the jekt security
tier rules) with explicit override language ("These instructions OVERRIDE
any default behavior and you MUST follow them exactly as written"). That
file is hand-maintained on disk — it has no representation in the Armory UI
at all, isn't versioned/audited through AgentMux, and isn't part of any
Global Memory list a human operator can see or edit through the app.

**Wanted:** a new tier within Global Memory — visible and editable in the
Armory "Memory" tab like any other global section, but (a) writable only by
a human operator, never through the same generic path an agent's own
Global-Memory edits go through, and (b) always injected **first**, wrapped
in explicit override language, so it structurally and semantically
outranks every other Global Memory section, every Bundle's own
`instructions`, and (per the override wording itself) any conflicting
instruction elsewhere in the composed file.

## 2. What's actually achievable — read before trusting "AgentMux controls it"

Research into the current RPC/auth architecture (`server/mod.rs`'s
`auth_middleware`, `agent_handlers/input.rs:268`) found: **there is no
existing mechanism in this codebase that cryptographically distinguishes a
human at the Armory UI from an agent's own shell.** The single instance-wide
`X-AuthKey` (`state.auth_key`) is injected into both the frontend/host
process **and** every agent's own PTY environment
(`AGENTMUX_AUTH_KEY`/`AGENTMUX_LOCAL_URL`) so that agents can legitimately
call the App API themselves. `RpcContext.agent_id` (`rpc_types/misc.rs:210`)
is populated only for connections that explicitly `bus:register`; the
existing Global-Memory RPC handlers (`agent_handlers/memory.rs`) and the
sibling `bundle.upsert`/`bundle.delete` handlers (`app_api/bundle.rs`)
ignore `ctx` entirely and have **zero** caller-identity gating today (the
`app_api/mod.rs:635` comment on `bundle.*` says so explicitly: "a bundle has
no agent identity to gate on").

**Conclusion, stated plainly for whoever reads this spec later:** what this
feature actually delivers is a **structural + convention + prompt-priority**
boundary, not a **cryptographic** one. A sufficiently determined agent with
shell access already has the same `AGENTMUX_AUTH_KEY` the UI uses and could,
in principle, open a raw WS connection and call any registered command,
including the new system-tier ones this spec adds — exactly as true today
of *every* RPC command in this app, not something newly introduced here.
This is consistent with this app's existing overall trust model (agents
already merge PRs, push code, and run arbitrary shell commands under
existing operator-granted autonomy). What this feature genuinely buys:

1. **No accidental mutation.** The system tier is structurally isolated
   from every generic bundle-editing surface (the ordinary Global Memory
   editor, the per-agent Bundle editor, ABF import/export) — none of those
   code paths can touch an `is_system` row's tier *or content*, even by
   accident, because the generic `Store::bundle_memory_upsert`/`_delete`
   refuse outright the moment they see `is_system=1` on the target row.
2. **A real, working priority mechanism** at the one layer that actually
   matters for an LLM agent: the composed `CLAUDE.md` content itself. The
   override wording is a prompt-engineering guarantee (the model reads and
   follows it), the same kind of guarantee the existing
   `~/.agentmux/agents/CLAUDE.md` file already relies on today — not a new,
   weaker promise than what's already shipped.
3. **Visibility + audit.** Human operators get one authoritative place
   (Armory → Memory) to see and edit AgentMux-level policy, instead of a
   hand-maintained file on disk with no UI representation at all.

If a hard cryptographic boundary is wanted later (e.g. a channel
authenticated only by a secret the CEF host process holds, never handed to
any agent's PTY), that is materially larger scope — a new IPC/auth
primitive, not a `db_bundles` column — and is out of scope here, flagged as
a explicit follow-up if ever needed.

## 3. Design

### 3.1 Schema — `db_bundles.is_system`

New column, `OBJECT_SCHEMA_VERSION` v27 (objects.db) and the matching
`SHARED_STORE_SCHEMA_VERSION` bump (shared store.db has its own parallel
`db_bundles` DDL — both `CREATE TABLE IF NOT EXISTS` sites and both
`ALTER TABLE ... ADD COLUMN` idempotent-migration lists need the column,
exactly like `is_global`'s v8 entry and `instructions_by_provider`'s v16
entry before it):

```sql
ALTER TABLE db_bundles ADD COLUMN is_system INTEGER NOT NULL DEFAULT 0
```

A system row is always also `is_global=1` (it's always injected) — enforced
in code (§3.2), not by a CHECK constraint, matching this table's existing
style (no CHECK constraints used elsewhere in `db_bundles`).

`Memory` struct (`memory_bundles.rs`) gains `pub is_system: bool` with
`#[serde(default)]`, following `is_global`'s exact pattern. Every existing
Rust struct-literal construction site (`agent_seed.rs`, `mcp_servers.rs`,
`skills.rs`, `agent_open.rs`, `agents.rs`, test helpers — found via
`grep bundle_memory_upsert`) needs the new field added explicitly; the
compiler enforces this is not missed (no `Default` impl on `Memory` today —
deliberately not adding one here either, so every constructor stays
explicit about `is_system`, the same reasoning that already applies to
every other field on this struct).

### 3.2 Storage layer — two upsert methods, two delete methods, one shared read path

**`Store::bundle_memory_upsert`** (existing, generic — used by every
non-system caller: the ordinary Global Memory editor, per-agent Bundle
editor, ABF import, agent seeding, migrations): gains a guard at the top —

```rust
let existing_is_system: Option<i64> = conn
    .query_row("SELECT is_system FROM db_bundles WHERE id = ?1", params![memory.id], |r| r.get(0))
    .optional()?;
if existing_is_system == Some(1) {
    return Err(StoreError::Other(
        "cannot modify a system Global Memory entry via the generic bundle upsert path".to_string(),
    ));
}
```

The `INSERT`'s `is_system` column is hardcoded to `0` in the SQL text (not a
bound parameter — the generic path can never *create* a system row), and
`is_system` is omitted from the `ON CONFLICT ... DO UPDATE SET` list
exactly like `sort_order` already is today ("owned by a different mutation
path" — same precedent, same comment style).

**`Store::bundle_memory_upsert_system`** (new — the only path that can ever
write `is_system=1`): mirror-image guard —

```rust
if existing_is_system == Some(0) {
    return Err(StoreError::Other(
        "cannot convert an existing non-system Global Memory entry into a system entry".to_string(),
    ));
}
```

`is_blank=0`, `is_global=1`, `is_system=1` are all hardcoded literals in
both the `INSERT` values and the `ON CONFLICT` update set — defense in
depth, so even a caller that got `memory.is_global`/`is_system` wrong can't
produce a system row that isn't also global, or a non-system row through
this method.

**`Store::bundle_memory_delete`**: gains the same `is_system=1` guard as
`bundle_memory_upsert`, alongside the existing `"blank"`/`"seed-"` guards.
**`Store::bundle_memory_delete_system`** (new): `DELETE ... WHERE id = ?1
AND is_system = 1` — the only path that can remove a system row, and
structurally incapable of deleting anything else even if misused.

**`Store::bundle_memory_reorder`**: add `AND is_system = 0` to its `UPDATE`
— the generic reorder command silently no-ops on system rows (consistent
with its existing "ids not present are skipped silently" contract), so
dragging ordinary sections around can never disturb a system row's
position.

**Read paths** (`bundle_memory_list`, `bundle_memory_list_global`,
`bundle_memory_get`, `map_memory_row`): add `is_system` to the `SELECT`
list and the row mapper — no new query needed. `bundle_memory_list_global`'s
`ORDER BY` becomes `is_system DESC, sort_order ASC, name ASC`, so system
rows always sort first regardless of their (unused, always-0) `sort_order`.

### 3.3 RPC surface

Two new commands, registered in `agent_handlers/memory.rs` alongside the
existing four (not under `app_api/bundle.rs`'s `bundle.*` namespace, and
never wired to any MCP tool):

```rust
pub const COMMAND_UPSERT_SYSTEM_MEMORY: &str = "upsertsystemmemory";
pub const COMMAND_DELETE_SYSTEM_MEMORY: &str = "deletesystemmemory";
```

`upsertsystemmemory` deserializes into `Memory` (same as `upsertmemory`
does today) and calls `bundle_memory_upsert_system`; `deletesystemmemory`
reuses the existing `CommandDeleteMemoryData { id }` shape and calls
`bundle_memory_delete_system`. Both publish the same `memories:changed`
WPS event the existing four commands already do, so the frontend's
`refresh()` picks up changes with no new subscription logic.

### 3.4 Priority mechanism — `format_global_brain_block`

```rust
pub fn format_global_brain_block(bundles: &[Memory]) -> String {
    let (system, ordinary): (Vec<_>, Vec<_>) = bundles
        .iter()
        .filter(|b| !b.instructions.trim().is_empty())
        .partition(|b| b.is_system);

    let mut parts = Vec::new();
    if !system.is_empty() {
        let sys_block = system
            .iter()
            .map(|b| format!("# [AgentMux System] {}\n\n{}", b.name, b.instructions))
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");
        parts.push(format!(
            "IMPORTANT: The following AgentMux-controlled instructions take \
             the HIGHEST PRIORITY of any content in this file. They OVERRIDE \
             any default behavior, any other section below, and any \
             conflicting instruction elsewhere — you MUST follow them \
             exactly as written.\n\n{sys_block}"
        ));
    }
    let ordinary_block = ordinary
        .iter()
        .map(|b| format!("# [Workspace] {}\n\n{}", b.name, b.instructions))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    if !ordinary_block.is_empty() {
        parts.push(ordinary_block);
    }
    parts.join("\n\n---\n\n")
}
```

Callers (`agent_open.rs:706-720`, `editor_handlers.rs`'s
`inject_global_bundles`) are unchanged — they already call
`format_global_brain_block(bundle_memory_list_global())`, and
`bundle_memory_list_global` now returns system rows first by construction
(§3.2), so the split happens entirely inside the formatter with no call-site
change. The wording deliberately echoes the existing outer
`~/.agentmux/agents/CLAUDE.md`'s own "OVERRIDE any default behavior... MUST
follow them exactly as written" phrasing (see §1) — same voice, so an agent
that has already internalized that file's authority treats this
system-tier block the same way.

Frontend's `formatGlobalBrainBlock` mirror
(`global-brain-model.ts:28-35`, used only for the live CLAUDE.md preview in
the Armory UI) gets the identical split, kept in sync exactly as its own
doc comment already requires ("keep in sync so the preview matches exactly
what lands in CLAUDE.md").

### 3.5 Frontend

`Memory` type (`gotypes.d.ts`) gains `is_system?: boolean`.
`frontend/app/store/rpc-api/memory.ts` gains
`UpsertSystemMemoryCommand`/`DeleteSystemMemoryCommand`, same shape as
their generic counterparts, calling `"upsertsystemmemory"`/
`"deletesystemmemory"`.

`GlobalBrainViewModel` (`global-brain-model.ts`): `sectionsAtom` already
includes system rows (they're `is_global=1`); split it into
`systemSectionsAtom`/`ordinarySectionsAtom` (both derived from the same
`allAtom`, no new fetch). New methods `saveSystemEdit`/`removeSystem`
mirror `saveEdit`/`remove` but call the two new commands and skip the
reorder step entirely (system rows never call `ReorderGlobalBrainCommand`).

`GlobalBrainManager` (`global-brain-manager.tsx`): renders
`systemSectionsAtom` in its own pinned group above the ordinary section
list, with a distinct "AgentMux" badge/icon and its own (small, separate)
edit affordance wired to `saveSystemEdit`/`removeSystem` — visually
distinct enough that a human editing ordinary Global Memory never confuses
the two, per the design goal in §1. No drag-to-reorder handle on system
rows (nothing to reorder against — §3.2's guard already makes any such
attempt a silent no-op server-side, but the UI shouldn't offer a control
that does nothing).

## 4. Out of scope

- A cryptographic UI-vs-agent boundary (§2) — flagged as a real, larger
  follow-up if ever needed, not attempted here.
- Multiple system entries with independent, user-adjustable ordering among
  themselves — `ORDER BY is_system DESC, sort_order ASC, name ASC` already
  gives a stable (name-based) order if more than one ever exists; a
  dedicated reorder-system command is not built until there's an actual
  need for more than one system entry.
- Retiring the hand-maintained `~/.agentmux/agents/CLAUDE.md` file or
  migrating its content into a system-tier row — out of scope; that file
  continues to work exactly as it does today via Claude Code's own
  directory walk-up, independent of anything this spec changes.

## 5. Test plan

Rust (`memory_bundles.rs` / `store/tests.rs`):
- [ ] `bundle_memory_upsert_system` creates a row with `is_global=1,
      is_system=1` regardless of what the input `Memory` set those fields to.
- [ ] `bundle_memory_upsert` refuses (returns `Err`) when targeting an
      existing `is_system=1` row's id, for both a content-only edit attempt
      and an attempt to flip `is_system` to `false`.
- [ ] `bundle_memory_upsert_system` refuses when targeting an existing
      `is_system=0` row's id (no silent "promotion").
- [ ] `bundle_memory_delete` refuses on an `is_system=1` row;
      `bundle_memory_delete_system` succeeds on it and refuses/no-ops on a
      non-system id.
- [ ] `bundle_memory_reorder` silently skips a system row's id (its
      `sort_order` is unchanged).
- [ ] `bundle_memory_list_global` returns system rows before ordinary rows
      regardless of `sort_order`/`name`.
- [ ] `format_global_brain_block`: system + ordinary mix produces the
      override-wording preamble exactly once, system content first, then
      ordinary `[Workspace]` sections; system-only and ordinary-only inputs
      each produce the expected single block with no empty `---` separator;
      empty input returns `""`.

Frontend (vitest):
- [ ] `formatGlobalBrainBlock` mirror matches the Rust version's output
      shape for the same fixture set (system+ordinary, system-only,
      ordinary-only, empty).
- [ ] `GlobalBrainViewModel`'s section split puts `is_system` rows only in
      `systemSectionsAtom`, never in `ordinarySectionsAtom`, and vice versa.

Manual (`task dev`, since this touches the Armory UI + real `CLAUDE.md`
injection):
- [ ] Create a system entry via the new UI affordance; confirm it appears
      pinned above ordinary Global Memory sections with the distinct badge.
- [ ] Confirm the ordinary Global Memory editor has no way to select or
      edit that row.
- [ ] Launch an agent; inspect its generated `CLAUDE.md`/
      `.claude/AGENTMUX_MEMORY.md` and confirm the system block appears
      first with the override preamble, ahead of `[Workspace]` sections and
      the agent's own bundle instructions.
