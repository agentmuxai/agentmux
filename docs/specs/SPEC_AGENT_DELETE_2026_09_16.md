# SPEC: Agent Row Actions Menu — Delete + Duplicate (My Agents picker)

**Status:** proposed — backend delete-cleanup implemented and manually
verified via `task dev`; UI relocated per repo-owner direction, not yet
rebuilt at the new location. Scope for the new location finalized
2026-09-16: Delete, Duplicate (history-forking, unchanged from existing
fork flow), Rename, and View History — repo-owner-confirmed via direct
Q&A, §8.1/§8.2 resolved.
**Date:** 2026-09-16
**Author:** Agent2
**Related:** `agentmux-srv/src/backend/storage/agents.rs` (`AgentDefinition`,
`agent_def_delete`), `agentmux-srv/src/backend/rpc_types/commands.rs`
(`COMMAND_DELETE_AGENT`), `agentmux-srv/src/server/agent_handlers/core.rs`
(the `deleteagent` RPC handler), `agentmux-srv/src/server/agent_handlers/template.rs`
(the `forkagentdefinition`/`forkagentdefinitionsuggest` handlers this spec's
Duplicate action reuses), `agentmux-srv/src/backend/storage/managed.rs`
(the `PRAGMA foreign_keys` caveat §5.1 depends on), `agentmux-srv/src/backend/
storage/migrations.rs` (schema — cascade-declared and loose-reference tables,
§5.1), `frontend/app/store/rpc-api/agent.ts` (`DeleteAgentDefinitionCommand`,
`ForkAgentDefinitionCommand`, `ForkAgentDefinitionSuggestCommand`,
`RenameAgentDefinitionTitleCommand`), `frontend/app/view/agent/components/
MyAgentsList.tsx` (the picker's "My Agents" row list this spec's menu attaches
to — already has an inline-expand precedent, the fork prompt, §1),
`frontend/app/view/agent/components/AgentPicker.tsx` (`handleFork`, the
existing duplicate-and-launch flow this spec's Duplicate action reuses),
`frontend/app/element/confirm-modal.tsx` (`ConfirmModal`, the confirm-dialog
component Delete reuses), `frontend/app/view/agent/components/AgentCard.tsx`
(documents the "hide is templates-only, deleteagent is the real removal path"
split this spec completes; also the Templates-tier card this spec's menu does
**not** extend to — My Agents only).

---

## 0. Origin, and why this spec changed shape twice

The repo owner asked for the ability to delete an agent. This went through
three UI locations in one session before settling — recorded here so a
future reader doesn't re-propose the first two:

1. **First draft: a red × in the live Agent pane header**
   (`AgentViewModel.endIconButtons`, next to the "Stash" backpack icon).
   Implemented, manually verified working via `task dev`. **Superseded** —
   repo owner: "we dont want it in the agent pane header."
2. **Second draft: a new "Manage" tab inside the Stash modal**
   (`AgentStashModal.tsx`'s existing Accounts/Memory/MCP/Skills/Startup/
   Registration tab set). Design-only, not implemented (superseded mid-build,
   before any Stash-modal file was touched). **Superseded** — repo owner
   redirected before this was built: "we want the red X button on the 'my
   agents' in the startup screen of the agent pane" instead.
3. **Current direction (this revision): a per-row actions menu on the
   picker's "My Agents" list** (`MyAgentsList.tsx` — the "startup screen,"
   i.e. what a pane shows before an agent is launched into it). A
   chevron-down affordance on each row, highlighted on hover, which expands
   inline to reveal actions — **Delete** and **Duplicate** confirmed by the
   repo owner, plus room for more (§4.3 brainstorms candidates; §8 asks
   which to actually build).

**What did NOT change across all three drafts:** the backend work. Direct
code research (not guessed) found that **the delete path already existed
end-to-end before this spec — `agent_def_delete` / the `deleteagent` RPC /
`DeleteAgentDefinitionCommand` — but nothing in the frontend called it.**
`agents.rs`'s own doc comment on `AgentDefinition.user_hidden` states the
intended split plainly: user-owned rows' "removal path is `deleteagent`, not
hide" — confirming this was always meant to be a real, user-facing action,
just never connected to anything. Implementing draft 1 surfaced a real,
independent bug in that dormant backend path (§5.1 — six declared FK
cascades are inert in production; some now-orphaned tables hold live secret
material) and fixed it with a dependent-table purge + a passing test. That
fix is UI-location-independent and stays as-is across all three drafts —
only the button's home keeps moving.

## 1. What exists today

| Concept | Mechanism | Notes |
|---|---|---|
| **Delete agent definition** | `Store::agent_def_delete` (`agents.rs:1252+`, extended by this spec — see §5.1) → purges every dependent table, `project_instructions_forget(id)`, `DELETE FROM db_agents`, mirrors into the global registry (`reg.hard_delete(id)`, `registry_def_retire(id)`). RPC: `COMMAND_DELETE_AGENT = "deleteagent"` (`rpc_types/commands.rs:180`), handled in `agent_handlers/core.rs:292-310`, broadcasts `"agents:changed"` after. Frontend stub: `DeleteAgentDefinitionCommand` (`rpc-api/agent.ts:111-112`). | **Implemented and tested** (backend + test), independent of which UI calls it. Not currently called from anywhere in the UI — draft 1's call site was reverted along with the header button. |
| **Duplicate an agent (this spec's "Duplicate" action)** | Already exists, built for a different trigger: `MyAgentsList.tsx`'s "already open in another pane → Open new session" fork prompt (`handleOpenNewSession`/`handleForkStart`) calls `ForkAgentDefinitionSuggestCommand` (name suggestion) then `AgentPicker.tsx`'s `handleFork` (421-460+), which calls `ForkAgentDefinitionCommand({source_id, branch_label})` → a genuinely new, independent `AgentDefinition` row with **its own bundle** (`agent_handlers/mod.rs`'s own test name: `fork_agent_definition_provisions_its_own_bundle_distinct_from_source`), then launches it. | **No new backend RPC needed for Duplicate** — see §4.3. One real semantic wrinkle: `handleFork` passes `forkSession: true` to `launchAgentDefinition`, which carries the **conversation history** forward into the clone (Claude: `--fork-session`), not just the agent's config. Whether "Duplicate" from this new menu should keep that behavior or start the clone blank is open — §8.2. |
| **Rename an agent** | `RenameAgentDefinitionTitleCommand` (`agent-view.tsx`'s `handleTabRenameConfirm`, used today for a fork/stack tab's double-click-to-rename). | Not currently reachable from `MyAgentsList.tsx` rows at all — candidate action, §4.3. |
| **Delete agent instance** | `DeleteAgentInstanceCommand` (`rpc-api/agent.ts:276-281`), a separate, narrower-looking command. | Exact semantics vs. the definition-level delete above need confirming before implementation — open question §8.4. Not assumed equivalent. |
| **Hide template** | `agent_def_set_hidden` (`agents.rs:902-930`), wired from `AgentPicker.tsx`'s template-only right-click menu (`handleTemplateContextMenu`, 833-862). | Explicitly barred from user-owned agents per `agents.rs:99-102`'s own comment. This spec's menu is on **My Agents rows**, a completely different list from the Templates section `AgentCard.tsx`/`handleTemplateContextMenu` serve — no overlap, no reuse. |
| **Close pane (×)** | Header × → `pane-actions.ts` → `LayoutModel.closeNode` → `ObjectService.DeleteBlock` → `sagas::delete_block::run`. `blockframe.tsx:251-256` (`closeDecl`, `icon: "xmark-large"`). | **Not this feature.** Closes the *pane/block*, not an agent row. Unrelated now that the menu has moved off the pane header, kept here only so the icon choice in §4.1 still avoids colliding with it visually elsewhere in the app. |
| **My Agents row structure** | `MyAgentsList.tsx:528-743` — each row is a `<li class="agent-recent-sessions-row">` containing ONE big `<button class="agent-recent-sessions-entry">` wrapping the icon + all row text (clicking anywhere on it reattaches/launches), plus a conditionally-rendered inline `<div class="agent-fork-prompt">` sibling (the existing "already open elsewhere" fork prompt) that expands/collapses per-row via a `Map<string, ForkState>` signal (`forkStates`). | **This is the reusable precedent for §4.2's inline-expand menu** — same `<li>`-scoped conditional-sibling pattern, same "keyed by `definition_id`" state-map idiom. The existing entry `<button>` already contains all row content, so a NEW interactive chevron control must be a **sibling** element in the `<li>`, not nested inside that button (nested `<button>`s are invalid HTML and the outer button's own click handler would fire first anyway). |

No existing spec documents this feature — genuinely new ground on top of an
already-built, previously-unreachable backend endpoint (delete) and an
already-built, differently-triggered one (fork/duplicate).

## 2. Goals

1. Each row in the picker's **My Agents** list (`MyAgentsList.tsx`) gets a
   chevron-down affordance, visually highlighted on hover, that does **not**
   trigger the row's own reattach/launch click.
2. Clicking the chevron **expands an inline panel** (not a floating
   popover — matches the existing fork-prompt precedent in the same file,
   and the repo owner's own wording, "it expands with options") listing row
   actions.
3. **Four confirmed actions for this pass (repo-owner decision, 2026-09-16):**
   - **Delete** — opens a destructive `ConfirmModal` naming the agent, then
     calls the existing `deleteagent` RPC (backend already hardened, §5.1).
   - **Duplicate** — reuses the existing fork-and-launch flow (§1) as-is,
     **forking conversation history forward** (`forkSession: true`, the
     flow's existing default) — no new backend RPC, no behavior change from
     what `handleFork` already does today. §8.2 (was open, now resolved).
   - **Rename** — reuses the existing `RenameAgentDefinitionTitleCommand`
     (§1), with an inline input in the expanded panel.
   - **View History** — reuses the existing `openOrFocusHistoryTab` (§1) to
     open a read-only history tab without relaunching the agent.
4. The menu design stays extensible for later additions — §4.3 records
   other candidate actions considered and explicitly deferred (Copy Agent
   ID, Hide/Archive, Export as Bundle).
5. Deleting an agent still leaves no dangling pane pointing at it (§4.5,
   carried over from the header-button draft — the concern is identical
   regardless of which UI triggers the delete).

## 3. Non-goals

- **The Agent pane header button (draft 1).** Reverted. Do not re-add
  `AgentViewModel.endIconButtons`' delete entry — this spec's UI lives in
  the picker now, not the live pane.
- **A "Manage" tab inside `AgentStashModal` (draft 2).** Never built past
  the design stage; abandoned before any Stash-modal file was touched. Do
  not resurrect without a fresh repo-owner ask — two redirects in one
  session is a strong signal this is the wrong home for the affordance.
- **The Templates section of the picker.** `AgentCard.tsx` / templates
  already have their own, separate right-click menu (Hide). This spec's
  menu is scoped to **My Agents** rows (`MyAgentsList.tsx`) only — a
  template is not a user-owned agent and Delete/Duplicate don't apply to it
  the same way (Hide already exists; "duplicate a template" is really
  "launch from template," an existing, different flow).
- **Bulk/multi-select actions.** Per-row menu only, one agent at a time. A
  fleet-tier bulk delete (mirroring `FleetBulkStop`) is out of scope.
- **Soft delete / trash / undo.** Delete is still a hard, immediate
  operation once confirmed. No recovery mechanism.
- **Enabling `PRAGMA foreign_keys = ON` globally.** §5.1 found the six
  `ON DELETE CASCADE` FKs declared in the schema are inert in production
  (`managed.rs:367-368`, pragma only ever set in test fixtures). Turning it
  on process-wide would affect every other delete path in the app, not just
  this one — `agent_def_delete` instead cleans up its own dependents
  explicitly (§5.1), which is already implemented.

## 4. Design

### 4.1 The chevron affordance

A new sibling control per row, **not** nested inside the existing
`<button class="agent-recent-sessions-entry">` (§1's HTML-validity note).
Rendered as a second `<button>` inside the same `<li class="agent-recent-sessions-row">`,
positioned via CSS to sit at the row's right edge (flex/absolute, matching
how `AgentCard.tsx`'s `agent-card-new-session-btn` already overlays its own
card without being nested inside that card's click target):

```tsx
<li class="agent-recent-sessions-row">
    <button class="agent-recent-sessions-entry" onClick={() => handleRowClick(row)}>
        {/* existing icon + body content, unchanged */}
    </button>
    <button
        type="button"
        class="agent-recent-sessions-menu-toggle"
        classList={{ "is-open": menuOpen() }}
        aria-label={`Actions for ${row.instance_name || row.definition_name}`}
        aria-expanded={menuOpen()}
        onClick={(e) => {
            e.stopPropagation(); // don't also trigger the row's own reattach
            toggleMenu(row.definition_id);
        }}
    >
        <i class="fa-sharp fa-solid fa-chevron-down" aria-hidden="true" />
    </button>
    <Show when={menuOpen()}>
        {/* §4.2 */}
    </Show>
</li>
```

`e.stopPropagation()` is required (same reasoning `AgentCard.tsx`'s
`handleNewClick` already documents for its own "+ New" button) — without it,
a click on the chevron would bubble to... nothing in this case since the
chevron isn't nested inside the entry button, but it sits visually on top of
it, and a browser could still route the click to the entry button first
depending on stacking/positioning; `stopPropagation` removes any ambiguity.

**Hover:** `.agent-recent-sessions-menu-toggle:hover` gets a background/tint
— matches the repo owner's "when hover it highlights." A `.is-open` class
(or the native `aria-expanded` attribute selector) keeps the same
highlighted look while the panel is expanded, so the control doesn't look
"unpressed" while its own panel is showing.

**Icon:** `chevron-down` (rotates to `chevron-up`, or just flips via CSS
`transform: rotate(180deg)` when `.is-open`, cheaper than swapping icon
names) — this is a disclosure affordance, not itself a destructive action,
so no red tint here (unlike draft 1's button, which WAS the destructive
action and needed to read as dangerous on sight). Red is reserved for the
Delete option inside the expanded panel (§4.2).

### 4.2 The expanded panel

Inline, not a floating popover — same `<li>`-scoped conditional-sibling
pattern the existing fork prompt (`agent-fork-prompt`, `MyAgentsList.tsx:642-738`)
already uses, including its state-map idiom:

```ts
const [menuStates, setMenuStates] = createSignal<Map<string, boolean>>(new Map());
const isMenuOpen = (definitionId: string): boolean => menuStates().get(definitionId) ?? false;
const toggleMenu = (definitionId: string): void => {
    setMenuStates((prev) => {
        const next = new Map(prev);
        next.set(definitionId, !(prev.get(definitionId) ?? false));
        return next;
    });
};
```

Panel contents (v1, per §2.3 — all four repo-owner-confirmed actions):

```tsx
<div class="agent-row-menu" data-testid="agent-row-menu">
    <button class="agent-row-menu-item" onClick={() => handleRenameStart(row)}>
        <i class="fa-sharp fa-solid fa-pen" aria-hidden="true" /> Rename
    </button>
    <button class="agent-row-menu-item" onClick={() => handleDuplicate(row)}>
        <i class="fa-sharp fa-solid fa-clone" aria-hidden="true" /> Duplicate
    </button>
    <button class="agent-row-menu-item" onClick={() => handleViewHistory(row)}>
        <i class="fa-sharp fa-solid fa-clock-rotate-left" aria-hidden="true" /> View History
    </button>
    <button
        class="agent-row-menu-item agent-row-menu-item--danger"
        onClick={() => setDeleteConfirmRow(row)}
    >
        <i class="fa-sharp fa-solid fa-trash-can" aria-hidden="true" /> Delete
    </button>
</div>
```

Delete is deliberately placed last (a small, standard convention for
"the destructive one goes at the end, visually separated") — worth a
`.agent-row-menu-item--danger` top border/spacing in the stylesheet to set
it apart from the three non-destructive actions above it.

Closing behavior: click elsewhere collapses it (same as the existing fork
prompt's implicit collapse-on-cancel pattern) — a click on the chevron
itself already toggles closed via `toggleMenu`; a click on either menu item
should also close the panel immediately (optimistic collapse) rather than
wait for the action's async result, since Delete opens ANOTHER modal
(§4.4) on top and Duplicate's own loading state is better shown on the row
itself (a spinner, matching `AgentCard.tsx`'s existing `props.launching`
pattern) than inside a still-open menu panel.

### 4.3 Delete and Duplicate, and other candidate actions

**Delete (confirmed):** `setDeleteConfirmRow(row)` opens a `ConfirmModal`
(destructive) — same shape as draft 1's already-built-and-discarded version,
just retargeted:

```tsx
<ConfirmModal
    open={deleteConfirmRow() !== null}
    title={`Delete ${deleteConfirmRow()?.instance_name || deleteConfirmRow()?.definition_name}?`}
    description="This permanently deletes the agent and its credentials, skills, and activity history. Its bundle and any manually-saved transcripts are not affected. This cannot be undone."
    confirmLabel="Delete"
    destructive
    onConfirm={confirmDelete}
    onCancel={() => setDeleteConfirmRow(null)}
/>
```

`confirmDelete` calls `RpcApi.DeleteAgentDefinitionCommand(TabRpcClient, { id: row.definition_id })`
— the same already-hardened backend path from draft 1 (§5.1), unchanged.
On success: close the modal, and (§4.5) sweep for any open pane bound to
this `definition_id` and close it. `MyAgentsList.tsx` already refetches on
`"agents:changed"` (`waveEventSubscribe`, line 322-326), so the row
disappearing from the list is already handled — no new subscription needed.

**Duplicate (confirmed, 2026-09-16: keep history-forking behavior as-is):**
reuses `AgentPicker.tsx`'s existing `handleFork` (§1) unchanged — the
picker already owns this function; the new menu just needs a second call
site for it (today it's only reachable via the fork-prompt's "Open new
session" button). Simplest wiring: hoist `handleFork` (or a thin wrapper
around it) so `MyAgentsList.tsx` can invoke it directly, the same way
`onFork` is already threaded down as a prop for the existing fork prompt.
**Naming:** reuse `ForkAgentDefinitionSuggestCommand` for a suggested label
the same way `handleOpenNewSession` already does, rather than silently
reusing the source agent's exact name for the clone. **Resolved:**
`forkSession: true` stays as-is — the clone continues the source's
conversation, matching `handleFork`'s existing behavior exactly. No new
backend logic, no parameter change.

**Rename (confirmed, 2026-09-16):** reuses `RenameAgentDefinitionTitleCommand`
(§1). `handleRenameStart(row)` opens an inline input in the expanded panel
— reuse the existing fork-prompt's naming sub-pattern (`agent-fork-naming`,
already in this file: text input + Start/Cancel buttons, Enter/Escape
handling) rather than inventing new input-row markup. On submit, call
`RenameAgentDefinitionTitleCommand({ id: row.definition_id, title })`; the
row updates via the same `"agents:changed"` refetch delete already relies
on (§4.3's Delete paragraph) — no separate optimistic update needed.

**View History (confirmed, 2026-09-16):** `handleViewHistory(row)` calls
the existing `openOrFocusHistoryTab` (`open-history-tab.ts`) with
`row.definition_id` — opens a read-only history tab for the agent without
launching/reattaching a live pane. No new backend work; this is purely
wiring an existing function to a new call site, same as Rename and
Duplicate.

**Other candidate actions (brainstormed, explicitly deferred — not part of
this pass):**

| Action | Feasibility | Notes |
|---|---|---|
| **Copy Agent ID** | Trivial — `row.definition_id` to clipboard (`clipboardWriteText`, already imported in `agent-view.tsx`). | Useful for MCP/scripting contexts that address agents by id. Deferred, not requested for this pass (§8.1 answer: only Rename + View History added). |
| **Hide / Archive** | **Not recommended.** | `agents.rs:99-102`'s own doc comment explicitly designed "Hide" as templates-only and delete as the user-agent removal path — resurrecting a hide concept for user agents contradicts that design intentionally, not accidentally. Only pursue this with an explicit repo-owner ask that acknowledges overriding that decision. |
| **Export as Bundle / "Save as Template"** | **Bigger, no existing primitive.** | Nothing today clones a user agent back into a shared template row — would need real backend design, not a thin UI wrapper like the other rows in this table. Flagged as a stretch idea only. |

### 4.4 Reused from draft 1 unchanged: backend hardening

See §5 (renumbered from draft 1's §4.4/§4.5, content unchanged) —
`agent_def_delete`'s dependent-table purge and the open-pane sweep are
UI-location-independent and already implemented/tested.

## 5. Backend — implemented, UI-location-independent

*(Carried over verbatim from the header-button draft; this work does not
change regardless of which UI calls `deleteagent`.)*

### 5.1 `agent_def_delete` did not clean up its dependents — now fixed

> **Correction (2026-09-16, later the same day):** this section's premise —
> that the declared cascades are inert — is **wrong**, and so is the
> `managed.rs:367-368` comment it cites. `Store::configure_and_migrate`
> (`store.rs:279`) sets `PRAGMA foreign_keys=ON` on the production
> connection, so the six cascades below do fire. The explicit purge is still
> correct and still load-bearing, for the tables that carry no FK at all
> (credentials, both signing-key tables) — and for a reason this section
> missed entirely, see the second correction below. Keeping one complete
> list rather than splitting it by FK-or-not is deliberate: a reader
> shouldn't have to know which half a new table lands in to tell whether it
> was handled. `managed.rs`'s stale comment is untouched here (it is about
> a different, genuinely cross-database case) but should be corrected.

> **Correction 2 (2026-09-16): the purge was aimed at the wrong database for
> four of these tables.** `db_agent_identity_links`, `db_agent_credentials`,
> `db_agent_native_memory` and `db_agent_native_memory_versions` are created
> by **both** `run_object_schema` and `run_identity_store_schema`, and per
> `SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md` the identity store holds the
> live rows — `listrecentsessions` itself reads links from
> `identity_store.agent_identity_list_all()`, not the object store's
> same-named table. `agent_def_delete` runs against the object store only,
> so the credentials and account links this section promised to remove
> survived the delete; the test passed because it seeded the object store's
> legacy copies. Fixed: `purge_agent_dependents` now skips tables absent
> from whichever connection it runs on (so one list serves both schemas),
> `Store::agent_dependents_purge` exposes it, and the `deleteagent` handler
> — the only layer that holds both stores — calls it on
> `state.identity_store` after `agent_def_delete`. Covered by
> `agent_dependents_purge_works_against_the_identity_store_schema`.

Confirmed directly: six tables declare
`ON DELETE CASCADE FOREIGN KEY (agent_id) REFERENCES db_agents(id)`
(`db_agent_content`, `db_agent_skills`, `db_agent_history`,
`db_agent_identity_links`, `db_agent_skills_ref`, `db_agent_mcp_ref` —
`migrations.rs:511-776`), but `managed.rs:367-368` states outright that
`PRAGMA foreign_keys` is only ever set `ON` in test fixtures — never on a
production connection. ~~**These six cascades are decorative in the shipped
app.**~~ (See the correction above — they are not.) Six more tables
reference `agent_id` with **no FK at all**:
`db_agent_credentials` (838-846), `db_agent_native_memory` /
`db_agent_native_memory_versions` (858-901), `db_agent_jekt_keys` (939-943 —
the per-agent HMAC jekt signing key), `db_agent_lan_keys` (958-963 — the
per-agent Ed25519 LAN signing key), `db_conversation_trust_grants`
(1024-1030, both `agent_id` and `granted_peer_agent_id`), and
`db_agent_activity_summaries` (1037-1041, keyed by `definition_id`). Leaving
credentials and signing keys behind after "deleting" an agent is a real
security-hygiene problem, not just clutter.

**Implemented:** `agent_def_delete` now explicitly deletes from all twelve
tables above, the same way it already explicitly called
`project_instructions_forget(id)` for the one dependent it already knew
about. Verified by a real test (`store/tests.rs`,
`agent_def_delete_purges_dependent_tables_not_just_the_agent_row`) that
inserts a row in every table, calls `agent_def_delete`, and directly
queries each table to confirm zero remaining rows.

**Explicitly NOT deleted, by design:**
- `db_bundles` — the agent's dedicated ABF bundle (`AgentDefinition.memory_id`,
  `agents.rs:153-164`) survives independently, by design.
- Any manually-exported/archived session artifacts (`SessionExportCommand`)
  — already-detached copies.

**`db_work_queue.target_agent` claims are NOT touched, and cannot be from
inside `agent_def_delete`.** Confirmed during implementation: `db_work_queue`
is created by `run_identity_store_schema` (`migrations.rs:1639+`), a
physically separate SQLite database (`~/.agentmux/shared/identity-store.db`)
from the one `db_agents` and its siblings live in (`run_object_schema`,
`Store::agent_def_delete`'s own `self.conn`). An earlier implementation
draft wrongly assumed a same-connection `UPDATE db_work_queue ...` inside
`agent_def_delete` — that would have failed with "no such table" against a
real database. See open question §8.5 for what a real fix looks like.

### 5.1a The delete that half-worked: the row survived in the instance registry

*(Added 2026-09-16 after the repo owner reported "delete isn't completely
working — the icon on the card disappears, but the card stays.")*

**Symptom, precisely.** The picker's "My Agents" rows do not come from
`db_agents`. `listrecentsessions` (`agent_handlers/session.rs:117+`) builds
them from `Registry::list_active()` — the host-global *instance* registry at
`~/.agentmux/shared/agents/registry/` — overlaid with local SQLite rows, then
resolves each row's display fields from `agent_def_list()`. When an instance
record outlives its definition, the row still renders, with
`provider: ""` (so `DualProviderLogo` draws nothing — the icon "disappears")
and `definition_name: "(missing definition)"`, while `instance_name` keeps
the card's title populated. The card stays.

**Two independent causes, both fixed:**

1. **The mirror was keyed on the wrong id.** `agent_def_delete` called
   `reg.hard_delete(id)` with a *definition* id, but `Registry` is keyed by
   **instance id** (`<instance_id>.json`). Those coincide only for records
   this tree has been re-keyed to, and `m0026_registry_agent_id_rekey` — the
   pass that re-keys them — reads `db_agent_instances` and returns a no-op
   once that table is dropped (schema v32). On the repo owner's machine 30
   of 44 registry records were still launch-keyed, i.e. one surviving ghost
   per agent. Fixed by `Registry::hard_delete_for_agent`, which matches a
   record by its file key **or** its own `definition_id`.
2. **The mirror was skipped entirely for cross-channel agents.** The sweep
   sat inside `if rows > 0`, i.e. "the local `db_agents` DELETE matched." An
   agent this channel only ever saw through the global overlay deletes with
   `rows == 0` — which is most rows a fresh channel shows (the repo owner's
   dev channel listed 20 rows against 2 local `db_agents` rows). Fixed by
   gating on `rows > 0 || global_retired` instead.

`instance_set_hidden` ("Forget agent") had cause 1 as well, which is exactly
the "the forgotten agent reappears" failure `m0026`'s own doc comment
predicted; it now uses `retire_for_agent`/`unretire_for_agent`.
`instance_delete` — the same deletion under a second name — shared neither
the dependent purge nor the registry sweep; both now come from the shared
`purge_agent_dependents` / `Store::purge_agent_side_effects`.

**Records already orphaned on disk** can't be reached by any of the above:
their definitions are already gone. `backend::registry_reconcile::
prune_tombstoned_instance_records`, run once per srv startup from
`bootstrap.rs` beside the existing `backfill_session_ids` pass, drops active
instance records whose definition is **tombstoned** in the global definition
store. Deliberately narrow: "absent from the active definition tree" alone
is not evidence of deletion (a definition may live only in some channel's
local SQLite), so a `retired/` tombstone is required.

### 5.2 Open pane handling

A block's association to an agent lives in block meta
(`agentId`/`agentName`), not a DB row — deleting `db_agents` does not, by
itself, touch any currently-open pane showing that agent. Now that Delete
is triggered from the picker (which may or may not have that agent's pane
open elsewhere at the time), the frontend must sweep for and close any pane
currently bound to the deleted `definition_id` after a successful delete,
via the existing pane-close path (`ObjectService.DeleteBlock`) — not just
"the pane the button was in," since the picker itself has no single
"originating pane" the way draft 1's header button did. Exact enumeration
mechanism (block-meta scan across open panes vs. a server-pushed list of
affected block ids) is implementation detail; "leave a stale pane open
referencing a deleted agent" is not an acceptable end state.

**Shipped state, and the gap that remains (2026-09-16, codex P2 round 2).**
`confirmDelete` sweeps every pane it can see via
`getOpenBlockIdsForDefinition`, and reports any `DeleteBlock` that fails
rather than swallowing it — the agent is already gone by then, so a pane
that won't close is a live pane attached to a deleted agent and only the
user can act on it.

**It does NOT reach another window.** `agent-pane-state-store`'s `slots` map
is module-local to one renderer, so a pane showing this agent in a second
window (or a floating-pane window) survives the delete with its agent
process still running. Closing those needs a backend-global block query or
a cross-window broadcast, which is real work rather than a wider filter at
the call site — so it is **deliberately deferred and recorded here**, not
quietly implied to be handled. This is the one part of §5.2's "not an
acceptable end state" that is still unmet; the single-window case, which is
the overwhelmingly common one, is closed.

## 6. Data flow summary

```
[My Agents row] → chevron click → inline panel expands
    ├── Duplicate → AgentPicker.handleFork(row, suggestedLabel)
    │       → ForkAgentDefinitionSuggestCommand (name) → ForkAgentDefinitionCommand
    │       → launchAgentDefinition(forkedDef, ...) → new pane opens
    └── Delete → ConfirmModal (destructive) → [confirmed]
            → DeleteAgentDefinitionCommand(row.definition_id)
            → RPC "deleteagent" → agent_handlers/core.rs
            → Store::agent_def_delete(id):
                 - DELETE across all twelve dependent tables (§5.1)
                 - project_instructions_forget(id)      [pre-existing]
                 - DELETE FROM db_agents WHERE id=?1    [pre-existing]
                 - reg.hard_delete(id) / registry_def_retire(id)  [pre-existing]
            → broadcast "agents:changed"
            → MyAgentsList refetches (already subscribed) — row disappears
            → frontend: close any open pane(s) bound to this definition_id (§5.2)
```

## 7. Phased implementation plan

1. ~~Backend cleanup (§5.1).~~ **Done** — implemented and tested, survives
   the UI relocation unchanged.
2. **Chevron + inline panel (§4.1, §4.2).** Add the sibling button, hover
   state, and expand/collapse state map to `MyAgentsList.tsx`.
3. **Delete wiring (§4.3).** `ConfirmModal` + `DeleteAgentDefinitionCommand`,
   same shape as the (reverted) draft-1 version, retargeted to a row instead
   of the pane header.
4. **Duplicate wiring (§4.3).** Hoist/share `AgentPicker.handleFork` so
   `MyAgentsList.tsx` can call it from the new menu item, unchanged
   (`forkSession: true` stays as-is per §8.2's resolution).
5. **Rename wiring (§4.3).** Inline input reusing the fork-prompt naming
   sub-pattern, calling `RenameAgentDefinitionTitleCommand`.
6. **View History wiring (§4.3).** Call `openOrFocusHistoryTab` with the
   row's `definition_id`.
7. **Open-pane sweep (§5.2).** Close any pane(s) bound to a just-deleted
   `definition_id`, verified manually against a real running instance.
8. **`db_work_queue` handling** — resolved per open question §8.3 before
   this ships, not deferred silently.

## 8. Open questions for the repo owner

1. ~~Which of §4.3's candidate actions should ship alongside Delete/
   Duplicate?~~ **Resolved 2026-09-16** (direct Q&A): Rename and View
   History, yes; Copy Agent ID and Hide/Archive, no (not for this pass).
2. ~~Duplicate's history semantics~~ **Resolved 2026-09-16:** keep
   `handleFork`'s existing behavior — forks conversation history forward
   (`forkSession: true`), unchanged.
3. **`db_work_queue.target_agent` claims for a deleted agent** — release
   back to the queue, cancel outright, or leave for WorkQueue's own expiry
   logic? Needs a cross-store fix at the RPC-handler layer regardless (§5.1)
   — not implementable as a one-line addition to `agent_def_delete` itself.
   **Still open, and now known to be worse than "clutter"** (codex P2,
   2026-09-16): `work_queue_claim` only hands a targeted row to a claimant
   with the matching id (`work_queue.rs:208`), so an OPEN row targeted at a
   deleted agent is permanently unclaimable, and a CLAIMED one reaps back
   into that same stuck state. The `deleteagent` handler now reaches the
   identity store (where `db_work_queue` lives), so the mechanical blocker
   is gone — what's missing is the repo owner's answer on WHICH of the three
   behaviors is wanted, since each is a different promise to whoever
   enqueued the work. Deliberately not decided unilaterally.
4. Delete's confirm-modal copy (§4.3) is a draft — review the exact wording
   about what is and isn't deleted (bundle survives, credentials/keys don't)
   before shipping, since it's the only warning a user gets before an
   irreversible action. Still open.
5. **`DeleteAgentInstanceCommand` vs. `DeleteAgentDefinitionCommand`** —
   confirm the menu's Delete action should call the definition-level delete
   (this spec's assumption throughout), and clarify what the separate
   instance-level command is actually for. Still open.
