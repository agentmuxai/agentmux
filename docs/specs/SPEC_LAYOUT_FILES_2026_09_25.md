# SPEC: Layout files — "Layouts → Save layout…" and the `agentmux.layout` file format

**Date:** 2026-09-25
**Status:** proposed — nothing here is built. Measured against `agentmux`
`main` @ `c0268089b`.
**Trigger:** Repo owner: *"in the hamburger we'd add a new entry 'Layouts' and a
single submenu: 'Save layout' which would let you save it as a file (you'll
need to write a file spec) — actually, we may want to simply extend or
generalize abf as the single file store for all needs?"*
**Relationship to `SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md`:**
that spec's naming ("Layout", §3) and menu placement (§5.1) stand. This spec
**replaces its Feature 2 storage decision** — a `db_layouts` table, with
export out of scope (its l.128, l.140) — with a file, and adds the file
format it never had. Its Feature 1 (restore on relaunch) shipped in PR #2560
and is unchanged here (§1.1).
**Decision needed from the owner:** §5 (ABF), §9.

---

## 0. TL;DR

- **Nothing to extend in code.** Named Layouts were never built; the menu has
  no Layouts entry; the host has no Save dialog. What does exist is the
  session-restore snapshot, which walks exactly the tree a layout file needs
  but copies block metadata verbatim — session ids, instance ids, absolute
  paths — so it is not safe to hand to a file (§1).
- **The file** is a readable JSON document, `*.agentmux-layout.json`:
  `format` + `version` header, windows → tabs → a split tree with **ratios**
  → panes → pane tabs, each a **typed view** with a per-type config written
  from an **allowlist**, never raw block meta (§3). One schema serves as a
  template (fresh launch, the default) and as a snapshot (opt-in resume
  fields).
- **ABF:** don't make the layout an ABF, and don't turn ABF into the single
  store yet. ABF is a zip whose manifest is bundle-shaped, untyped, has no
  `kind`, and whose importer always produces a bundle row and silently drops
  unknown components (§1.3). Instead the layout document carries its own
  `format`/`version` envelope, so it can later become one typed item in a
  general container — and the one strong case for that container is real: a
  layout that carries the agents it references (§5).
- **v1 is exactly what was asked**: ☰ → Layouts → **Save layout…**, a native
  Save dialog, one file written by the srv. Opening a layout is Phase 2 —
  flagged in §9 because a save-only feature isn't useful for long (§6, §7).

---

## 1. What exists today (measured)

### 1.1 The session snapshot — the tree walk to reuse

Restore-on-relaunch (PR #2560) serialises the first live window's workspace
to `Client.meta["session:last_topology"]`
(`agentmux-srv/src/server/service/session_restore.rs:42`):
`{tabs:[{name, blocks:[{meta}], rootnode, focusednodeid, magnifiednodeid}],
active_tab_index}` (`:117-128`). Block ids become `__snap_block_N__`
placeholders (`:44-50`, `:133-150`); layout node ids are kept (`:534-542`).

What makes it unsuitable as a file format, as-is:
- **`blocks.push(json!({ "meta": block.meta }))`** (`:95`) — every meta key,
  including `agentInstanceId` (a DB row id), `agent:sessionid`,
  `session:*` stats, `subagent:*`, `agent:last_failure`, absolute `cmd:cwd`,
  `file`, `media:path` (`frontend/types/srv-types.d.ts:790-937`).
- No `version` or schema field.
- One workspace only (`:384-420`, a documented limitation).
- Sizes are the tree's raw flex units, not normalised.

What makes it the right starting point: it already walks Workspace → Tab →
`LayoutState` → Block with the store's own types, and its restore half
(`restore_last_session`, `:422-601`) already rebuilds tabs from a tree plus a
block list — Phase 2's apply can share it.

The "turn restore off" setting that spec promised doesn't exist
(`frontend/app-init.ts:354` hardcodes `restoreIfAvailable: true`). Out of
scope here; noted in §8.

### 1.2 The layout model

- Hierarchy (`agentmux-srv/src/backend/obj.rs`): `Window` (pos, winsize,
  opacity, `:333-374`) → `Workspace` (`tabids`, `activetabid`, `:380-393`) →
  `Tab` (`layoutstate`, `blockids`, `:408-419`) → `LayoutState`
  (`rootnode`, `focusednodeid`, `magnifiednodeid`, `:433-450`) → `Block`
  (`meta`, `:480-493`).
- `LayoutNode` (`agentmux-common/src/layout_types.rs:71-91`): `id`,
  `flexDirection` (row|column), `size` (relative flex units within the
  parent), `children`, `data`, `extra` (unknown keys preserved).
  `LayoutNodeData` (`:32-56`): `blockId`, `blockStack` (pane tabs),
  `activeBlockId`. The frontend adds `minimized` and related flags
  (`frontend/layout/lib/types.ts:193-262`), kept through `extra`.
- The only portable layout description today is `PresetNode`
  (`frontend/app/tab/tab-presets.ts:29-38`): `{widget}` or `{split,
  children}` — no sizes (`:50-52`), and it duplicates the backend's
  `default_three_pane_tree` by hand (`:43-47`).

### 1.3 ABF

- A **zip** (`.abf`), every entry under `<root_slug>/`
  (`agentmux-srv/src/backend/bundle_export.rs:694-716`). Manifest
  `armory.json` is an untyped `json!` literal (`:626-651`): `$schema`, `name`,
  `version`, `description`, `provider`, `model`, `components`, `metadata`.
- **No `kind`/type discriminator.** Import requires `armory.json`
  (`bundle_import.rs:636-640`) and always produces a `db_bundles` row.
- Components are handled by name — `instructions`, `skills`, `mcpServers`,
  `accounts`, `memory`, `projectInstructions` (`bundle_import.rs:713-1067`);
  **any other key is dropped silently**. The exporter's `history` component
  (`app_api/bundle/components.rs:315-320`) is one such key today.
- `version` means different things on each side: the bundle's content
  version to the exporter (`bundle_export.rs:627-637`), the format version to
  the importer (`bundle_import.rs:673-689`).
- Import UI exists (`showOpenBundleDialog` → 3-step modal); **export has no
  UI** — nothing in the frontend calls `bundle.export`.
- Nothing in any ABF spec or file mentions layouts, windows, tabs or panes.

### 1.4 Menu and file I/O

- ☰ menu: `frontend/app/window/hamburger-menu.tsx:78-143`, `MenuItem`
  (`frontend/types/custom.d.ts:452-468`: `label`, `icon`, `subItems`,
  `onClick`, `divider`, … — no text input). Entries: New Tab, New Window |
  Theme, Opacity | Settings, Command Palette, Armory, Toolchain, DevTools,
  Online Docs | Exit.
- **Dialogs: open only.** The CEF host uses `rfd`:
  `show_open_file_dialog` and `show_open_bundle_dialog`
  (`agentmux-cef/src/commands/platform.rs:1007-1051`), wired through
  `agentmux-cef/src/ipc.rs:295-296` and `frontend/util/cef-api.ts:529-534`.
  No save dialog anywhere.
- Writing files today: the editor's `writeeditorfile` RPC (home dir only,
  10 MB, `server/editor_handlers.rs:424-489`) behind an in-app path box; a
  browser-style `<a download>` for session transcripts
  (`view/agent/session-actions.ts:53-67`) whose host-side download handler is
  a stub (`agentmux-cef/src/commands/stubs.rs:31,45`). Neither is a real
  "save as a file" path.

---

## 2. What comparable tools do (condensed)

Full notes with sources were gathered for this spec; the decision-relevant
points:

| Tool | Format | Geometry | Template vs snapshot | Notable |
|---|---|---|---|---|
| Zellij | KDL layout | `%` or fixed per pane | both, one format (resurrection writes the same KDL) | commands from a remote layout start suspended; scrollback opt-in |
| tmuxp / tmuxinator | YAML | tmux presets / cell strings | both (`tmuxp freeze`) | tmux cell strings break on resize |
| kitty | `.kitty-session` | cells/px for the OS window | both (`save_as_session`) | `--relocatable` paths relative to the file |
| Warp Tab Configs | TOML | split tree | save-as-config from a live tab | typed panes incl. `agent`; `{{param}}` prompts |
| VS Code | JSON | ratio tree summing to 1 | UI state not portable (SQLite) | `.code-profile`: one file, several typed parts, per-part import checkboxes |
| iTerm2 / JetBrains | arrangements / named layouts | — | snapshot | "restore as tabs" merge option; named, overwrite-in-place |
| i3 | JSON | percentages | template with placeholders | unmatched windows become placeholders |

Consensus: a readable text format; ratio-based split trees; one schema for
template and snapshot; never scrollback, transcripts or secrets by default;
commands from an untrusted file start suspended; a per-pane type
discriminator; unknown fields preserved; apply into a new window or as new
tabs, never silently replacing.

---

## 3. The file format: `agentmux.layout` v1

### 3.1 Container

- UTF-8 JSON, one document. Extension **`.agentmux-layout.json`** — the
  double extension keeps it filterable in dialogs and highlighted as JSON in
  any editor.
- Top-level envelope:

```json
{
  "format": "agentmux.layout",
  "version": 1,
  "name": "Review setup",
  "saved_at": "2026-09-25T18:04:11Z",
  "saved_by": { "app": "agentmux", "app_version": "0.57.3", "instance": "gr2q7gf5lh6pzfdnurnkvputhm" },
  "scope": "window",
  "windows": [ { "...": "§3.2" } ]
}
```

- `saved_by.instance` is this install's WAN instance id
  (`SPEC_WAN_JEKT_VERIFICATION_2026_09_24.md` §2.2) — public, not a secret —
  used only by §3.5 to tell "saved here" from "came from elsewhere". Omitted
  when the install has no `wan.db`.

- `format` is the discriminator; a reader refuses a document whose `format`
  it doesn't know. `version` is the **format** version (never the content's —
  the mistake ABF made, §1.3).
- A reader refuses a higher `version` with a clear message ("saved by a newer
  AgentMux"), migrates lower ones on load, and ignores and **preserves**
  unknown fields (every object carries its unknown keys through a
  load/save round trip, as `LayoutNode.extra` already does).
- A JSON Schema ships at `schema/agentmux-layout.v1.schema.json` and the
  `$schema` key may reference it; it is informative, never required.

### 3.2 Windows, tabs, tree

```json
{
  "windows": [{
    "size_hint": { "width": 0.8, "height": 0.85 },
    "active_tab": 0,
    "tabs": [{
      "name": "main",
      "focused": "p1",
      "magnified": null,
      "root": {
        "split": "row",
        "children": [
          { "ratio": 0.6, "node": { "pane": "p1" } },
          { "ratio": 0.4, "node": {
              "split": "column",
              "children": [
                { "ratio": 0.5, "node": { "pane": "p2" } },
                { "ratio": 0.5, "node": { "pane": "p3" } }
              ]
          } }
        ]
      },
      "panes": {
        "p1": { "views": [ { "type": "agent", "...": "§3.3" } ], "active": 0 },
        "p2": { "views": [ { "type": "term" } ], "active": 0, "minimized": false },
        "p3": { "views": [ { "type": "sysinfo" } ], "active": 0 }
      }
    }]
  }]
}
```

- A split is `{split: "row"|"column", children: [{ratio, node}]}`; a leaf is
  `{pane: "<id>"}`. Ratios within one split sum to 1 (writers normalise;
  readers renormalise and tolerate drift). `row`/`column` are the tree's own
  `flexDirection` values, so export is a direct map: `ratio = size / Σ size`.
- Pane ids (`p1`…) are **local to the file** — minted fresh on save, never an
  OID or node UUID. `panes` is a map so the tree stays readable.
- A pane holds its pane tabs (`views`, from `blockStack`) and which is active;
  `minimized` and the other frontend flags map from `LayoutNode.extra` when
  present and are optional.
- `size_hint` is optional, in **fractions of the work area**; applied clamped
  to a visible monitor. Never pixels or cells, never position — a layout
  opened on another monitor setup must not land off-screen.

### 3.3 Views — typed, from an allowlist

Each view is `{type, title?, config?, resume?}`. **`config` is written from a
per-type allowlist, never by copying block meta** (§1.1):

| `type` | `config` fields written | Never written |
|---|---|---|
| `agent` | `agent`: the definition's slug + display name; `provider`; optional `model` | `agentInstanceId`, `agent:sessionid`, `session:*`, `subagent:*`, `agent:last_failure`, CLI paths, account/identity links |
| `term` | `cwd` (home-relative `~/…` when under home), optional `command` | env, `term:*` runtime state, scrollback |
| `browser` | `url` with query and fragment **stripped** | cookies, history, storage |
| `editor` | `files` (home-relative), active file | unsaved buffers |
| `media` | `path` (home-relative) | — |
| `sysinfo`, `swarm`, `settings`, `armory`, other singletons | the view's own section/tab selector if it has one | — |
| unknown to the writer | nothing — the view is written as `{type}` only | — |

- `resume` (**opt-in at save time, off by default**): for `agent`,
  `{session_id}`; for `term`, nothing in v1. A resume reference is
  machine-local and useless elsewhere; the file says so rather than
  pretending otherwise.
- The allowlist lives in one Rust table next to the exporter
  (`layout_file.rs`, §6) with a test per view type: add a view type ⇒ add a
  row, or it is saved as `{type}` alone.

### 3.4 What a layout file never contains

Tokens, keys, credentials or env values; agent signing keys, jekt keys,
MCP server credentials; transcripts, scrollback, terminal output; cookies or
browser storage; any OID, block id, node UUID or instance id; query strings.
The save path additionally runs the existing credential/destructive keyword
list (`is_sensitive_message`, `backend/reactive/sanitize.rs`) over every
`command` and warns before writing a match.

### 3.5 Applying a file (Phase 2 — shaped now so v1 files stay valid)

- Default: open as a **new window**; alternative: **add as tabs** to the
  current window. Never replace what's open without an explicit, confirmed
  choice.
- Fresh launch by default; `resume` honoured only if the session still
  exists on this machine.
- **Trust:** a file not written by this install (no matching
  `saved_by.instance`, or opened from outside the layouts folder) shows a
  preview first — panes, agents, and every `command` — and its commands start
  suspended until confirmed (Zellij's remote-layout rule; this repo's §5.3
  "confirmation-gated agent relaunch").
- Missing agent definition, missing `cwd` or file, or an unknown `type`
  becomes a **placeholder pane** that shows what was expected and keeps the
  raw node, so re-saving doesn't lose it.
- Fresh OIDs for everything; the file's pane ids never reach the store.

---

## 4. Where files live

- **Default folder:** `~/.agentmux/shared/layouts/` — account-wide and
  channel-independent (`DataPaths::shared_dir`), so a layout saved in one
  build opens in every other, like bookmarks
  (`backend/bookmarks_store.rs:42-51`).
- The Save dialog opens there with `<name>.agentmux-layout.json` prefilled,
  but the user may save anywhere — the point of a file is that it can be
  shared.
- Phase 3's named list in the menu reads the default folder; files saved
  elsewhere are opened with "Open layout…".

---

## 5. ABF: extend it, generalise it, or not?

### 5.1 Options

| | A. Layout is its own JSON file (recommended for v1) | B. Layout is an ABF | C. Generalise ABF into "the AgentMux file" now |
|---|---|---|---|
| Readable / diffable / hand-editable | yes | no (zip) | no (zip) |
| Fits ABF's shape | n/a | no: manifest is bundle-shaped, untyped; import always makes a bundle row | needs a `kind` field, an importer that dispatches on it, and the `version` ambiguity fixed |
| Existing importers | unaffected | silently ignore a `layout` component today (§1.3) — an old build "imports" a layout and does nothing | same, until every importer checks `kind` |
| Can carry the agents a layout references | no | yes | yes |
| Work before "Save layout" ships | format + writer | format + writer + ABF changes | ABF redesign first |

### 5.2 Recommendation

**A now, with the door open to a real container later.** The layout document
is self-describing (`format`, `version`), so a future container can hold it
verbatim as one typed item. The case for that container is genuine and
specific: **a layout that brings its agents with it** — open a colleague's
layout and get their reviewer agent, skills and MCP servers, not a
placeholder. That is Phase 4 (§6), and it is the moment to generalise ABF
properly:
- add a top-level `kind` (`bundle` | `layout` | `workspace`) and make every
  importer refuse a kind it doesn't handle, instead of silently dropping it;
- split `version` into the format version and the content version;
- a `workspace` ABF = one `layout.agentmux-layout.json` plus
  `agents/<slug>/…` bundles, with an import preview and per-item checkboxes
  (the VS Code `.code-profile` pattern).

Doing C first would put a zip, a manifest redesign and an importer rewrite in
front of a one-menu-item feature, and make the layout unreadable to the
people it's meant to be shared with.

---

## 6. Phases

| Phase | Ships | Notes |
|---|---|---|
| **1** | ☰ → **Layouts** → **Save layout…** (between Opacity and the Settings divider, per the 08-13 spec §5.1). Native Save dialog. `layout.save` RPC writes the file. | exactly the request |
| 2 | Layouts → **Open layout…**: preview, trust, new window / add as tabs, placeholders, suspended commands | reuses `restore_last_session`'s rebuild |
| 3 | Named layouts from the default folder listed in the submenu; "Save changes to <name>"; recent list; optional "open at startup" | the 08-13 spec's §5.1 list, file-backed |
| 4 | Decision-gated: ABF `kind` + `workspace` bundles (layout + referenced agents) | §5.2 |
| — | Converge the session snapshot and `PresetNode` onto this format | removes two ad-hoc formats; separate PRs |

### 6.1 Phase 1 in detail

- **srv — `agentmux-srv/src/backend/layout_file.rs`** (new):
  - `export_layout(store, scope: Window{id} | Tab{id}) -> LayoutDoc` —
    walks Window → Workspace → Tabs → `LayoutState` → Blocks the way
    `session_restore.rs:84-128` does, normalises sizes to ratios, mints file
    pane ids, and writes each view through the §3.3 allowlist.
  - `LayoutDoc` and friends are typed `serde` structs with `extra` maps (the
    `LayoutNode` pattern), not a `json!` literal (ABF's manifest problem).
  - RPC **`layout.save`** `{scope, window_id | tab_id, name, path,
    include_resume}` → writes atomically (temp file + rename), returns the
    path and any §3.4 warnings. The path comes from the host's Save dialog;
    the srv refuses anything that isn't an absolute path ending in
    `.agentmux-layout.json`.
- **CEF host — `show_save_layout_dialog(default_name)`** in
  `agentmux-cef/src/commands/platform.rs`, `rfd::FileDialog::new()
  .set_directory(<shared>/layouts).set_file_name(…).add_filter("AgentMux
  layout", &["agentmux-layout.json"]).save_file()`, wired like
  `show_open_bundle_dialog` (`ipc.rs`, `cef-api.ts`, `custom.d.ts`). The
  first save dialog in the app — others (bundle export, which has no UI
  today) can reuse it.
- **Frontend — `hamburger-menu.tsx`:** `{ label: "Layouts", icon: "grip",
  subItems: [{ label: "Save layout…", onClick }] }`. `onClick`: host Save
  dialog (default name = the window's active tab name) → `layout.save` for
  the current window → a toast with the path, or the warnings.
- **Scope in v1:** the current window (all its tabs). A tab-only save is a
  one-field change (`scope: "tab"`) and can follow.

### 6.2 Tests (Phase 1)

- Export of a real store fixture round-trips through the schema; ratios sum
  to 1 per split; pane tabs and the active one survive.
- **Allowlist:** a block whose meta holds every "never written" key of §3.3 —
  and a token-shaped value in `cmd` env — produces a file with none of them;
  one test per view type; an unknown view type is written as `{type}` only.
- No OID, block id or node UUID appears anywhere in the output.
- Paths under home are written `~/…`; query strings are stripped.
- Unknown fields on load are preserved on save; a `version: 2` document is
  refused with the "newer AgentMux" message.
- `layout.save` refuses a relative path or another extension; writes
  atomically; a keyword match in a command returns a warning.
- Frontend: the menu renders Layouts → Save layout…; cancelling the dialog
  writes nothing.

---

## 7. Why save-only is acceptable for Phase 1 — and not for long

A saved file is already useful to read, diff and share, and it is the
fixture Phase 2 needs. But the natural next click after "Save layout…" is
"Open layout…", and the 08-13 spec's users asked for *reloadable* layouts.
§9 asks whether Phase 2 should ship in the same release.

## 8. Out of scope

- The restore-on-relaunch off switch (08-13 spec §4) — separate.
- Multi-window save ("all windows") — the envelope already allows several
  `windows`; the UI for it comes with Phase 3.
- Syncing layouts across machines through the cloud.
- Updating the 08-13 spec's status line (it still says Feature 1 isn't built)
  is done in the PR that adds this spec.

## 9. Open questions for the owner

1. **ABF (§5):** own JSON file now, ABF `kind`/`workspace` container later
   (recommended) — or generalise ABF first?
2. **Menu:** exactly one item ("Save layout…") as asked, or ship "Open
   layout…" (Phase 2) in the same release?
3. **Scope:** current window (recommended) or current tab by default?
4. **Default folder:** `~/.agentmux/shared/layouts/` (cross-channel,
   recommended) or per channel?
5. **Resume:** keep agent `session_id` references opt-in and off by default
   (recommended), or on by default for layouts saved and opened on the same
   machine?

## 10. Key files

- new `agentmux-srv/src/backend/layout_file.rs` — `LayoutDoc`, export,
  allowlist, `layout.save`
- `agentmux-srv/src/server/service/session_restore.rs` — tree walk to share;
  Phase 2 rebuild
- `agentmux-common/src/layout_types.rs` — `LayoutNode`/`LayoutNodeData`
- `agentmux-cef/src/commands/platform.rs`, `ipc.rs` — the Save dialog
- `frontend/util/cef-api.ts`, `frontend/types/custom.d.ts` — host API typing
- `frontend/app/window/hamburger-menu.tsx` — the menu entry
- new `schema/agentmux-layout.v1.schema.json`
- ABF (Phase 4 only): `backend/bundle_export.rs`, `bundle_import.rs`,
  `server/app_api/bundle/`
