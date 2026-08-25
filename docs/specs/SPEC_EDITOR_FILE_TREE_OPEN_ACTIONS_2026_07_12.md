# SPEC: Editor File-Tree "Open" Actions (Open to the Side / Open in New Tab)

Status: Draft
Date: 2026-07-12
Depends on: `SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md`, `SPEC_EDITOR_WIDGET_DEFAULT_UX_2026_06_14.md`
Closes gap in: `SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md` §Menu Definitions (File node),
§Phase 1

---

## Audit: current state of the editor pane's requested improvements

Two improvements were requested for the editor pane: (1) finish the file-tree
right-click menu, and (2) live markdown preview while editing. Auditing the
code against `SPEC_FILE_TREE_CONTEXT_MENU_2026_06_14.md` found:

### 1. File-tree context menu — 90% done, one specific gap

`buildContextMenuItems` (`frontend/app/view/editor/editor-view.tsx:559-630`)
already implements everything in the original spec **except** two Phase-1
items on the file-node menu:

| Spec item (file node) | Implemented? | Where |
|---|---|---|
| Open | ✅ | `editor-view.tsx:605` |
| **Open to the Side** | ❌ | — (this spec) |
| **Open in New Tab** | ❌ | — (this spec) |
| Copy Path | ✅ | `editor-view.tsx:607` |
| Copy Relative Path | ✅ | `editor-view.tsx:608-612` |
| Rename… (F2) | ✅ | `editor-view.tsx:616` |
| Delete | ✅ | `editor-view.tsx:617-628` |
| Reveal in Explorer | ✅ | `editor-view.tsx:614` |

Folder-node menu (New File…, New Folder…, Rename…, Delete, Open in Terminal,
Reveal in Explorer, Collapse Folder) and empty-space menu (New File…, New
Folder…, Refresh) are complete — no gaps there. Phases 2–3 of the original
spec (rename, create, delete flows, tab sync on rename/delete) all shipped as
specified.

The menu already uses the shared `ContextMenu` primitive
(`frontend/app/components/context-menu.tsx`) the original spec introduced —
this is the file tree's own primitive and is distinct from the codebase's
other two right-click/dropdown mechanisms (`ContextMenuModel`, a native-OS
menu used by tabs/blocks/action-widgets; `FlyoutMenu`, a click-triggered DOM
dropdown used for the hamburger menu). This spec's new items stay on
`ContextMenu` for consistency with the rest of this menu.

**This spec covers only the two missing items.**

### 2. Markdown live preview — already fully implemented, no gap found

Live preview already exists as a complete, previously-shipped feature (fixed
in PR #1743 after a regression documented in
`docs/retro/retro-editor-markdown-preview-regression-2026-06-23.md`):

- `.md` tabs default to `EditorMode = "preview"`
  (`frontend/app/view/editor/editor-model.ts:845`); `"source"` and `"split"`
  modes are also available via a toolbar and `Mod+Shift+V`
  (`editor-view.tsx:746-770`, `editor-model.ts:855-857`).
- CodeMirror's `updateListener` calls `setLiveDoc(content)` on every
  keystroke (`editor-view.tsx:325-329`) — deliberately decoupled from the
  persisted-content memo, which does not update per-keystroke
  (see comment at `editor-view.tsx:99-104`).
- The preview renders `<Markdown textAtom={() => liveDoc()} />`
  (`editor-view.tsx:888`) — the same shared `frontend/app/element/markdown.tsx`
  component used for agent chat rendering, built on a full
  `unified`/`remark-gfm`/`rehype-*` pipeline (confirmed present in
  `package.json`).

**No implementation work is needed for live markdown preview** — it already
updates on every edit in both `"preview"` (CodeMirror hidden, full preview)
and `"split"` (side-by-side) modes. If there is a specific reproduction where
this doesn't work as expected, that's a bug report against existing behavior,
not a new feature — file it separately with repro steps rather than folding
it into this spec.

---

## Problem (this spec's actual scope)

Right-clicking a file in the editor tree offers no way to open it beside the
current pane or in a fresh tab — both are one-click actions in every
comparable IDE, and the original file-tree context-menu spec called for both
in its very first implementation phase. Only "Open" (pinned tab in the
*current* editor pane) exists today.

---

## Goals

1. **Open to the Side**: opens the file in a new editor pane, split right of
   the current editor pane, without disturbing the current pane's tabs.
2. **Open in New Tab**: opens the file in a new editor pane inside a brand
   new, otherwise-empty app tab (not the standard agent/swarm/armory/sysinfo
   preset tab).
3. Both actions reuse existing backend/frontend primitives — no new RPCs.

## Non-Goals

- Multi-file selection ("open to the side" for N selected files at once) —
  out of scope, same as the parent spec's multi-select non-goal.
- Remembering per-file "preferred open mode" — every open via these two
  actions is a one-shot user choice, not a persisted preference.

---

## Design

Both actions are one RPC call each — `pane.open` (`agentmux-srv/src/backend/
rpc_types/block.rs:336-367`) already accepts everything needed:

```rust
pub struct CommandPaneOpenData {
    pub view: String,                          // "editor"
    pub file: Option<String>,                  // the clicked file's path
    pub tab_id: Option<String>,                 // target tab (Open in New Tab)
    pub split_direction: Option<String>,        // "right" (Open to the Side)
    pub split_reference_block_id: Option<String>, // this editor block's id
    pub focus: Option<bool>,
    // ...
}
```

This is the exact mechanism the file tree's existing **Open in Terminal**
action already uses (`editor-model.ts:804-816`, `pane.open` with
`view: "term", split_direction: "right", split_reference_block_id:
this.blockId`) — "Open to the Side" is that same call with `view: "editor",
file: path` instead of `view: "term", cwd: path`.

### Open to the Side

```typescript
// EditorViewModel (editor-model.ts), new method alongside openInTerminal():
async openToTheSide(filePath: string): Promise<void> {
    try {
        await TabRpcClient.rpcCall("pane.open", {
            view: "editor",
            file: filePath,
            split_direction: "right",
            split_reference_block_id: this.blockId,
        }, {});
    } catch {
        // pane.open might not be registered yet — fail silently, same as
        // openInTerminal's existing error handling.
    }
}
```

No backend changes — `pane::build_pane_meta` (`agentmux-srv/src/server/
app_api/pane.rs:208+`) already handles `view: "editor"` + `file` (it's how
every `pane.open`-driven editor pane gets seeded, e.g. the existing "Open in
Terminal" sibling action and widget-bar editor launches).

### Open in New Tab

There is no single RPC that creates a tab *and* opens a specific pane into it
with no preset — `WorkspaceService.CreateTab` (used by `createTab()` in
`frontend/app/store/global.ts:745-776`) is the primitive for creating a tab,
but `createTab()` itself always layers on `applyTabPreset(tabId,
DEFAULT_TAB_PRESET)` (agent + swarm + armory + sysinfo, as of
`SPEC_DEFAULT_WIDGETS_REORDER_2026_08_25.md`), which is wrong for this
action — the user asked for *this file*, not a fresh default workspace.

**Revised during implementation** (the design below superseded the original
draft, which called `pane.open` against the freshly created `tab_id` — that
looked correct on paper and even matched the existing `openInTerminal` /
`openToTheSide` pattern, but failed live testing: the backend created the
block and updated the layout with zero errors every time, yet the block
never rendered. Root cause: `pane.open`'s layout mutation goes through the
*backend's* reducer-driven layout-queue + WaveObj broadcast, and a
brand-new tab's *client-side* `layoutModel` — even once `waitForLayoutModel`
confirms the object exists — isn't yet subscribed to receive that specific
tab's `layout:update` broadcast. `openToTheSide` never hit this because it
targets the *current*, long-subscribed tab. The fix: build the block through
the same client-side path `applyTabPreset` itself uses —
`ObjectService.CreateBlock` + `layoutModel.treeReducer(...)` — instead of
the backend RPC.)

```typescript
// tab-presets.ts: export the two helpers applyTabPreset already uses
// internally (waitForLayoutModel, createBlockOnModel) so other CreateTab
// callers that need a NON-default block set can reuse them.

// editor-model.ts:
async openInNewTab(filePath: string): Promise<void> {
    const ws = workspace(); // from @/store/global
    if (!ws) return;
    try {
        const tabId = await WorkspaceService.CreateTab(ws.oid, "", true, false);
        const layoutModel = await waitForLayoutModel(tabId);
        if (!layoutModel) return; // tab never propagated
        const isMarkdown = filePath.toLowerCase().endsWith(".md");
        await createBlockOnModel(
            tabId,
            layoutModel,
            {
                meta: {
                    view: "editor",
                    file: filePath,
                    // Mirrors build_pane_meta's markdown defaults
                    // (agentmux-srv/src/server/app_api/pane.rs:208+) —
                    // this path bypasses that function entirely, so its
                    // per-view defaults have to be replicated here.
                    ...(isMarkdown ? { "editor:tree_expanded": false, "editor:source_hidden": true } : {}),
                },
            },
            null, // splitTargetId — none, this is the tab's only block
            null,
        );
        await setActiveTab(tabId); // see note below
    } catch {
        // Fail silently, consistent with the file tree's other pane.open callers.
    }
}
```

**Also discovered live, unrelated to the render bug**: `WorkspaceService.
CreateTab`'s `activate: bool` argument (3rd positional arg, `true` in both
`createTab()` and the call above) is accepted by the `workspace.CreateTab`
RPC handler but never forwarded into `Command::CreateTab` — the reducer
only ever auto-activates a workspace's very *first* tab
(`create_tab_second_tab_does_not_steal_active` in `reducer.rs`). Every
`CreateTab` call for a workspace's 2nd+ tab silently fails to switch focus,
`activate` argument notwithstanding. This affects the existing hamburger/
titlebar "New Tab" action too — **out of scope for this spec**, worked
around here with an explicit `setActiveTab(tabId)` call after the block
exists, but worth a standalone bug report.

### Menu wiring

In `buildContextMenuItems` (`editor-view.tsx:604-606`), insert both items
after "Open", before the "Copy Path" separator — matching the original
spec's menu ordering exactly:

```typescript
return [
    { type: "action", label: "Open", onSelect: () => void model.openFile(path) },
    { type: "action", label: "Open to the Side", onSelect: () => void model.openToTheSide(path) },
    { type: "action", label: "Open in New Tab", onSelect: () => void model.openInNewTab(path) },
    { type: "separator" },
    // ...unchanged
];
```

---

## Testing

Verified live via CDP against a running `task dev` instance (right-click →
click, not just unit-level):

- "Open to the Side" on a file → a new editor pane appears split right of
  the current one (2 distinct `editor-view` blocks under 2 distinct
  `data-blockid`s, confirmed both belong to the same app tab), showing the
  file; the original pane's tabs are untouched.
- "Open in New Tab" on a file → a new, empty app tab is created, becomes
  the active tab, and contains exactly one editor pane with the file open —
  confirmed by checking the rendered markdown preview text, not just DOM
  presence (early testing had a false-positive "it worked" read from
  `data-blockid` existing in the DOM while the tab was actually still
  blank — checking rendered file *content* is what caught the render bug
  documented above).
- Both actions on a file in a nested folder resolve the same absolute path
  the tree already has (no relative-path resolution needed — the tree's
  `path` values are already absolute, same as every other file-node action).
- `pane.open` unavailable (very early startup race) → `openToTheSide` fails
  silently, matching `openInTerminal`'s existing behavior. `openInNewTab`
  no longer depends on `pane.open` at all (see above) — its equivalent
  early-exit is `waitForLayoutModel` timing out and returning `null`.

## Effort estimate (actual, not original estimate)

The original "half a day, reuse pane.open for both" estimate held for
`openToTheSide` but not for `openInNewTab`, which needed a different
implementation strategy after live testing caught the client-side
layout-model gap. Total: about a day including the debugging that found the
two issues documented above (the render gap and the pre-existing
`activate`-argument bug).
