# Report: pane sections as widget-bar entries, and moving the Vault into Warden

**Date:** 2026-10-21
**Author:** Naki-2@narko
**Code read at:** `origin/main` 76ebb3343
**Status:** active. The operator asked for all of it on 2026-10-21. §4 is fixed in #5279. §3.4 items 1, 2, 4 and 5's groundwork ship with this report (the sections contract). §5 (the Vault in Warden), §3.4 item 3 (pin a section) and §6 (docs) follow in their own PRs.

## 1. The questions

The operator asked three things on 2026-10-21:

1. Kanban is a section of Warden, but it also has its own widget-bar entry. So can a widget be made from any sub-pane (section)? Is that consistent across every pane and the widget API?
2. Move the Vault into Warden.
3. Document this better, in agentmux-docs and the README.

## 2. Short answers

- **Yes, mostly by accident of design.** A widget-bar entry is just a pane definition: a `view` plus block meta. Every sectioned pane keeps its selected section in its own meta key. So an entry that sets that key opens the pane on that section. `defwidget@kanban` is exactly this: `{ "view": "warden", "warden:section": "kanban" }`. It isn't a separate widget. It's Warden, opened on Kanban.
- **It isn't enforced or uniform.** The pane-tab contract (`PaneTabManifest`) doesn't know what a section is. Each of the five sectioned panes picks its own meta key, its own default and its own validation, and three of the five are written by hand. Nothing lists a pane's sections, so neither `widgets.json`, the SDK nor the UI can discover or check them. A wrong section id quietly falls back to the pane's default section. The only way to make a section entry is to hand-write meta.
- **Through the widget API:** a sandboxed widget with the `panes` permission can open any built-in pane on any section, through `am.panes.open(view, meta)`. It does this the same way, with raw meta. A widget can't *add* a section to a built-in pane, and has no idea of sections itself: each of its `contributes.panes` is a separate view.
- **Vault in Warden is feasible.** Migration `m0037` already moved Remotes into Connectors the same way, so there's a precedent. There are four security behaviours tied to "the Vault is a pane" that must be re-keyed to "the Vault section is showing" (§5.3).
- **Docs:** agentmux-docs has no page that mentions the Vault or Kanban at all. Its Warden page lists five sections, one of which (Supervisor) no longer exists. The README's widget table has no Kanban or Vault entry.

## 3. How sections and widget-bar entries work today

### 3.1 A widget-bar entry is a pane definition

`crates/srv/src/config/widgets.json` maps `defwidget@<name>` to a `blockdef.meta`. Clicking the entry creates a block with that meta. The user's own `widgets.json`, next to `settings.json`, is merged over the built-ins: a new key adds an entry and a built-in key replaces it (`crates/srv/src/backend/user_widgets.rs`, `merge_widgets`). It is hot-reloaded. So a user can already add an entry like this one, which opens the Vault on Cards:

```json
{
  "defwidget@vault-cards": {
    "label": "Cards",
    "icon": "credit-card",
    "blockdef": { "meta": { "view": "vault", "vault:section": "cards" } }
  }
}
```

### 3.2 The five sectioned panes

| Pane | Meta key | Sections | Default | Implementation | Built-in section entry points |
|---|---|---|---|---|---|
| Warden | `warden:section` | kanban, host, lan, internet, audit, history | host | hand-written (`warden-model.ts`, `warden-view.tsx`) | `defwidget@kanban` (pinned), palette `open:kanban`, `openWarden(section)`, `kanban-held.tsx` (`kanban:open`) |
| Vault | `vault:section` | personal, cards, api-keys, logins, notes, kept, activity | (model) | hand-written (`vault-model.ts`, `vault-view.tsx`) | `openVault(section, requestId)` from the request prompts |
| Connectors | `connectors:section` | accounts, mcp, remotes | accounts | generic `SectionPane` (`section-pane/`) | palette `open:remotes`, `openConnectors(section)` |
| Memory | `memory:section` | global, personal, skills, bundles | global | generic `SectionPane` | `openMemory(section)` |
| Settings | `settings:section` | appearance, window, browser, terminal, sounds, notifications, recording, devices, widgets, advanced | (model) | hand-written (`settings-model.ts`) | browser profile menu → browser, missing-widget → widgets |

What they share is a convention, not a contract. The selected section lives in block meta under `<view>:section`, so it survives a remount and can be deep-linked. `SectionPane` (`section-pane/section-pane.tsx`) is the generic form of that convention. Its comment says it is "the shape of the Connectors and Memory panes". Warden, the Vault and Settings re-implement the same thing. Warden's model even says it "mirrors section-pane.tsx's sectionAtom".

### 3.3 Where it falls short

1. **No contract.** `PaneTabManifest` (`frontend/app/block/pane-tab-registry.ts`) has no `sections` field. Nothing outside a pane's own module knows its key, ids or labels.
2. **Nothing checks an entry.** `widgets.json` accepts any meta. An entry with `"warden:section": "kanbn"` opens Warden on Host with no error (`isWardenSection` falls back to `host`). The other panes behave the same way.
3. **No way to make one in the UI.** Kanban's entry was added by hand in `widgets.json` (SPEC_WARDEN_KANBAN_2026_10_20.md §6.2, D1). There's no "Pin this section to the bar" action. A user has to know the meta key and edit JSON.
4. **A section entry isn't a standalone pane.** Opening Kanban opens a whole Warden: all six sections mount, and Host, LAN and Audit start their 5-second polls. The user can switch it to Host, and its title follows the selected section. `openOrFocusPaneByView`, used by `open:kanban` and `openWarden`, focuses an existing Warden in the tab and switches *its* section instead of adding a pane. That's fine for a shortcut, but it's not what "a widget" suggests.
5. **Not in the SDK.** `v1.d.ts` types `am.panes.open(view, meta?, split?)` with free-form meta. The README and `widget-api.md` don't list the built-in views or their sections.
6. **Widgets can't extend a built-in pane.** A widget's `contributes.panes` each get their own `ext:<id>/<name>` view. `defaultMeta` keys are confined to `widget:`. There's no extension point for "add a section to Warden", and none is proposed here.

### 3.4 Recommendation: make sections part of the pane contract

1. Add an optional `sections` capability to `PaneTabManifest`: `{ key, default, items: [{ id, label, icon }] }`. Warden, the Vault, Settings, Connectors and Memory declare it. Move the three hand-written panes onto `SectionPaneModel` (or onto a shared helper that reads and validates the key), so the five can't drift apart.
2. In `widgets.json`, allow `"section": "<id>"` beside `blockdef`. srv and the frontend resolve it to `<view>:section` from the manifest, and an unknown id is reported in Settings ▸ Widgets instead of failing silently. Raw meta keeps working.
3. Add **Pin this section to the bar**. *Built:* the pane header's menu offers "Pin “LAN” to bar" for the section a pane shows (and its own section, for Warden's Vault). It pins an entry that already opens that section (Kanban's built-in one), or has srv add one to the user's `widgets.json` (`widgets.sectionentry`, key `section@<view>-<section>`) and pins that; a pinned section offers "Unpin". `defwidget@kanban` is the built-in example of the same thing.
4. In the SDK, export the built-in views and their section ids as a typed union, so `am.panes.open("warden", { "warden:section": "kanban" })` type-checks.
5. *Built (task #173):* Warden mounts each section the first time it is shown, and Host, LAN and Audit poll only while shown (`pollWhileVisible`). Optionally, mount a section-opened pane lazily: mount only the selected section until the user switches. Warden already does this for History and Kanban through a `visible` prop. This makes a pinned section cheaper than a whole Warden.

## 4. The widget API's `panes` permission (fixed in #5279)

`am.panes.open(view, meta, split)` is checked in `frontend/app/block/widget-bridge.ts`. For any view other than the widget's own, it needs the `panes` permission. The meta is then passed through unchanged: `sandboxed-widget-host.tsx` `openPane` creates `{ meta: { ...meta, view } }`. Nothing in that path, in `widget-bridge.ts` or in `wcore/block.rs` filters meta keys.

The concern: meta can carry a controller and a command. `defwidget@agent` uses `controller: "cmd"`, and `cmd:runonstart` defaults to `true` (`blockcontroller/shell/controller.rs`, `should_run_on_start`). If a sandboxed widget with `panes` can open `{ view: "term", controller: "cmd", cmd: "<anything>" }` and have it run, then `panes` is in effect "run commands on this computer". The install prompt describes it only as "Open other kinds of panes". The existing test (`widget-bridge.test.ts:171`) checks only that `panes` gates the view, not what the meta may say.

**Fixed in #5279**, found by reading the code, not reproduced at runtime. `paneMetaForWidget` decides what the new pane gets:
- A pane of the widget's own gets the widget's meta under its namespace, with `meta.set`'s key and size checks.
- Another view gets only a section its manifest declares (§3.4) or, for `browser`, an http(s) `url`. A controller, a command, a connection and every other key are dropped, so `panes` opens panes and runs nothing.
- The SDK typings, its README and the widget API spec say so.

## 5. Moving the Vault into Warden

### 5.1 The decision being changed

On 2026-10-19 the operator decided that "the Vault stays its own pane, and that Warden also shows its oversight half" (SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.6; reasoning in §5.1). The operator has now chosen to move the Vault into Warden. The spec needs an amendment to §5.1, §5.2 and §5.6 recording this, with the date.

§5.1's main argument for a pane of its own was "it can stay closed and locked until the person opens it". That argument is weaker than it reads. The Vault has no session unlock: every Reveal and Copy is confirmed with Windows Hello on its own (§5.3), so there's nothing to keep locked. What a separate pane does give is the set of behaviours in §5.3 below, which are tied to the pane. Each has a direct equivalent for a section.

### 5.2 Shape

- A seventh Warden section, `vault`, labelled "Vault", icon `lock`. Proposed position: after Kanban, since both are things the operator acts on, ahead of the monitoring sections.
- The Vault's own seven sections become a second-level tab row inside the Warden section. `vault:section` and `vault:request` keep their meaning, so a request prompt still opens the right form.
- The Vault's **Activity** stays in the Vault section. Warden's Audit links to it, which is the "linked both ways" of the old §5.6, now inside one pane.
- `defwidget@vault` keeps its key, so existing pins survive, and becomes `{ "view": "warden", "warden:section": "vault" }`, the same pattern as Kanban. The palette's `open:vault` and `openVault(section, requestId)` route through `openWarden("vault")` and add the Vault meta.
- Warden's tab title already follows the section, so it reads "Vault".

### 5.3 What must be re-keyed from "pane" to "section"

| Behaviour today | Where | In Warden |
|---|---|---|
| The "Vault on screen" note, which makes `UI window` refuse to capture any AgentMux window | `vault-model.ts:30` `shown` = pane visibility; `vault-view.tsx:229` | `shown` must be pane visible **and** `warden:section === "vault"`. Warden keeps every section mounted, so keyed on the pane alone it would block window captures whenever any Warden is on screen. |
| `data-agent-hidden="vault"`, which blacks out the Vault in agents' screenshots | `vault-view.tsx:262` | Stays on the Vault section's root. Check that the screenshot clip still finds it inside Warden's hidden-section wrapper. |
| Reveal hides after 15 s, on Hide, on window blur and on dialog close | `vault-reveal.tsx` | Also hide when the Warden section changes. Otherwise a revealed value stays in the DOM of a hidden section until the timer ends. |
| Entries load when the pane mounts | `vault-store.ts` | Load only when the section is first shown, using Warden's `visible` prop pattern (History, Kanban). Opening Warden on Host shouldn't fetch the Vault's entry list. |

### 5.4 Migration

Follow the Remotes → Connectors move:
- A migration like `m0037_retire_old_pane_names.rs` rewrites saved blocks from `view: "vault"` to `view: "warden"` with `warden:section: "vault"`, keeping `vault:section`.
- The layout-file reader (`layout_file.rs`, the `"remotes" => ("connectors", Some("remotes"))` arm) maps `vault` the same way.
- Register the `vault` view as an alias that opens Warden for one release, so a layout saved by an older build still opens.

### 5.5 Text that names "the Vault pane"

The Vault MCP tool's descriptions (`crates/mcp/src/tool_schemas.rs`), the operator-config seed (`crates/srv/operator-config-seed.json`, "kept in their Vault pane"), `window_capture.rs`, the CEF `browser_api` messages and `vault/storage` comments all say "Vault pane". They become "Warden's Vault section", or just "the Vault", which still reads right. Update the Vault spec and Kanban spec cross-references in the same PR.

### 5.6 Tests

- Port the vault-view tests into the Warden section.
- Add warden-view tests for the seventh section and for lazy loading.
- Re-run the screenshot blackout and `UI window` refusal tests with the Vault inside Warden, on the Vault section and on another section. The capture must be allowed on another section.
- Add migration tests for saved `view: "vault"` blocks.

## 6. Documentation plan

### 6.1 README (`agentmux/README.md`, §Widgets)

- Add **Kanban** to the "on the widget bar by default" table (`table-columns`, between Swarm and Memory, `display:order` 2.5). Say that it opens Warden on its Kanban section.
- Add **Vault** to the "More" table now (`lock`, `vault`). After the move, describe it as a Warden section reached from the same entry.
- Update Warden's row: "Kanban, Host, LAN, Internet, Audit and History (and the Vault)", not "Host / LAN / Internet, plus a jekt audit trail".
- Add a short paragraph, **Sections and shortcuts**: some entries open a section of a larger pane. The paragraph shows the `widgets.json` example from §3.1 for pinning any section yourself, and links to the docs table in §6.2.

### 6.2 agentmux-docs

| Page | Change |
|---|---|
| `warden.md` | "The five sections" → six (seven with the Vault): add Kanban and History, remove Supervisor (no longer a section in `warden-model.ts`). Correct "There is no … command-palette entry": `open:kanban` exists. Mention the pinned Kanban entry. |
| new `kanban.md` | What a card is, the columns, Needs you, how agents use the `Tasks` tool, lanes by repository, the `tasks:read` widget permission (from SPEC_WARDEN_KANBAN_2026_10_20.md). |
| new `vault.md` | What it holds, the sections, Reveal and Copy with Windows Hello, how agents fill fields without seeing values, grants and Activity, `.vault` export and import, where it's stored (shared by every AgentMux on the computer) and what happens on upgrade. After the move, it's reached from Warden. |
| `security/` | A Vault page beside `identity-credential-storage.md`: the storage key, per-entry encryption, the capture protections in §5.3. |
| `widgets.md` | A **Sections** subsection under "Widgets from widgets.json": the pattern, the example, and a table of every pane's section key and ids (§3.2). |
| `widget-api.md` | `am.panes.open` with a section. List the built-in views. State exactly what meta a widget may pass, once §4 is settled. |
| `pane-types.md` | Add Kanban and Vault to the widget list and the "more" list. |
| `glossary.md` | Section, Kanban, Vault. |

agentmux-docs runs a docs-vs-code audit (`specs/AUDIT_DOCS_VS_CODE_*`). The Warden page drift suggests the next audit should also check `widgets.json` against `pane-types.md` and the README tables.

## 7. Suggested order

1. Verify §4 in `task dev`; if it's real, fix it first, in its own PR.
2. Write the docs for what exists today (§6, without the move), so Kanban and the Vault are documented now.
3. Build the section contract (§3.4, items 1 and 2) and move Warden, the Vault and Settings onto it.
4. Move the Vault into Warden (§5), with the spec amendment and the migration. Update docs to match.
5. Add "Pin this section to the bar" and the SDK types (§3.4, items 3 and 4).
