# Spec: An agent opening a file shows the Editor pane it lands in

**Status:** proposed — not implemented
**Date:** 2026-09-30
**Author:** Lark

## Trigger

An agent called `OpenEditor` twice to show the user two specs. Both calls
succeeded and returned the same block id. Nothing appeared.

The tab already had an Editor pane, opened earlier by another agent and
minimized by the user. `OpenEditor` reused it, as designed
(`SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md`, Part 2).
The two files landed as tabs inside a pane that was only a header chip. The
user found them only after expanding it by hand.

`OpenEditor`'s own description says to use it "when you want the user to see
a file". A file opened into a hidden pane defeats the point of the call.

## Behavior

When an agent's `OpenEditor` call pushes a file into an existing Editor pane,
that pane is made visible **in its tab**:

- **If minimized, it's restored** to its size before minimizing.
- **If it's a background tab of a multi-tab pane** (a pane stack), that pane
  switches to the Editor.
- **If another pane in the same layout is magnified**, that pane is
  un-magnified. A magnified pane hides every other tile.

The newly opened file is the Editor's active tab, as today (`openFile()`).

What it deliberately does **not** do, because the user may be typing
elsewhere:
- **No window-tab switch.** If the Editor's tab isn't the active one, it
  becomes visible when the user next goes there.
- **No layout focus and no caret.** Moving layout focus to the reused pane
  remains the separate, known limitation in `pane.rs:331-350`.

Also:
- **Only the agent-initiated path does this.** Files the user opens from the
  editor's own tree, drag-and-drop, and live reloads of changed files don't
  change the pane's visibility.
- **Once per batch.** Several files arriving together make the pane visible
  once.
- **An already-visible pane is untouched.** Its size, position and other tabs
  don't change.
- **The user hiding it again afterwards sticks**, until the next
  agent-initiated open.

## Design

### A shared "show this block in its pane" step

`frontend/app/util/reveal-block.ts::revealBlockLocally` already does two of
the three steps, for Swarm rows, the token popover and OS notifications:

- un-magnify another pane that would hide the target;
- `setActiveBlockInStack` for a background member of a pane stack.

It then also switches the window tab, calls `focusNode`, and gives the block
the caret. It doesn't restore a minimized pane, and `focusNode`
(`layoutFocus.ts:152`) doesn't either. So revealing a minimized pane from
Swarm or a notification today selects a pane that stays collapsed. That's the
same gap, on a second path.

Extract the focus-free part into one exported helper in `reveal-block.ts`:

```ts
/** Make blockId's pane visible within its layout, without switching window
 *  tabs, moving layout focus or the caret. */
export function showBlockInPane(model: LayoutModel, node: LayoutNode, blockId: string): void {
    if (node.minimized) model.minimizeNodeToggle(node.id); // restore (toggle's restore branch)
    const magnified = model.magnifiedNodeId;
    if (magnified && magnified !== node.id) model.magnifyNodeToggle(magnified, true, false); // commit, no focus
    setActiveBlockInStack(model, node.id, blockId);
}
```

A plain `magnifyNodeToggle` is not focus-free: `LayoutModel.treeReducer` sets
`shouldRequestFocus` for every `MagnifyNodeToggle` and calls
`focusManager.requestNodeFocus()` once it commits. So the action gains
`focused?: boolean`. With `focused: false`, the un-magnify still commits and
persists, but it doesn't request focus. `LayoutModel.magnifyNodeToggle(nodeId,
setState = true, focused = true)` passes it through. Every existing caller
keeps the default. In the reducer:

```ts
case LayoutTreeActionType.MagnifyNodeToggle:
    magnifyNodeToggle(this.treeState, action as LayoutTreeMagnifyNodeToggleAction);
    shouldRequestFocus = (action as LayoutTreeMagnifyNodeToggleAction).focused !== false;
    break;
```

For a minimized Editor that is a background member of a pane stack, the order
matters. `node` is the stack's shared pane node, so the restore expands the
pane. Then `setActiveBlockInStack` makes the Editor its active member.
Restoring alone would leave `activeBlockId` on the other member, and the file
would stay hidden.

`revealBlockLocally` calls it in place of its current un-magnify and stack
lines, then keeps its own `focusNode` and caret steps. Reveal therefore gains
the minimized restore.

### Where the Editor calls it

In `EditorViewModel`'s `META_PENDING_OPEN_FILES` drain
(`frontend/app/view/editor/editor-model.ts`, the `createEffect` around
`:266-305`). That effect is the only consumer of reuse opens, the files
pushed by `maybe_reuse_editor_pane` (`agentmux-srv/src/server/app_api/pane.rs:351-409`).
After it schedules `openFile()` for a non-empty batch:

1. Find the tab holding this block with `reveal-block.ts`'s existing
   `findTabHolding(blockId, hint?)`. It is already on main, where
   `revealBlockLocally` uses it (`SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md`
   §4.1), and it is unchanged here. The drain has only a block id, and no
   layout API maps a block to its tab. It scans only
   `workspace()`, which is **this window's** workspace:
   `[...pinnedtabids, ...tabids]`, the hint first if given. It reads each
   `Tab` object (loading it if it isn't cached) and returns the first tab
   whose `blockids` include the block, or null. A block id belongs to one tab,
   so there's no tie to break. Other windows' tabs are never scanned. A
   renderer only has layout models for its own window, and
   `getLayoutModelForTabById` is keyed by the tab id found here. So the lookup
   can't reach another window's layout. The Editor running the drain is
   mounted in this window, so its tab is always found.
   Then get that tab's layout with `getLayoutModelForTabById`. The pane running
   the drain is mounted, so its tab's layout exists. Do **not** call
   `setActiveTab`.
2. `node = model.getNodeByBlockId(this.blockId)`. For a stack member this
   returns the shared pane node, which is what `showBlockInPane` expects.
3. `showBlockInPane(model, node, this.blockId)`.

Steps 1–3 are one exported helper, `showBlockWithoutFocus(blockId)`. It
returns false, touching nothing, when the block isn't in a built layout in this
window.

The Editor is a keep-alive view (`pane-leaf-chrome.tsx:333-439`), so the
drain can run while it's a hidden stack member or minimized. That is the
case step 3's stack switch exists for.

### Why the frontend, not the backend

The obvious alternative is to change the layout server-side in
`maybe_reuse_editor_pane`. It wouldn't work, for the same reason that
function doesn't set layout focus (`pane.rs:331-350`). `onBackendUpdate`
(`layoutPersistence.ts`) doesn't apply backend layout-tree changes to an
already-mounted `LayoutModel`, outside its two narrow triggers. The drain
effect already runs in the right place at the right time.

### Guards

- `minimizeNodeToggle` is a toggle: call it only when the leaf is minimized,
  or it would minimize a visible pane. Its last-expanded-pane guard applies
  only to minimizing, so restoring is always allowed.
- `magnifyNodeToggle` on the magnified pane un-magnifies it. Skip it when the
  target itself is the magnified pane. Pass `focused: false` (see above), or
  the un-magnify moves the user's focus.
- `setActiveBlockInStack` is already a no-op for a single-block pane or the
  already-active member.

## Tests

- **`showBlockInPane`:**
  - Minimized leaf: restored.
  - Visible leaf: `minimizeNodeToggle` never called.
  - Another pane magnified: un-magnified.
  - The target is the magnified pane: left magnified.
  - Background member of a stack: becomes the active block.
  - None of the above: no change.
  - No focus or caret change in any case.
- **`LayoutModel`:** `magnifyNodeToggle(id, true, false)` commits and
  persists the un-magnify without calling `requestNodeFocus`. The default
  still calls it.
- **Editor drain:**
  - A pending batch on a minimized Editor calls the helper once, and the pane
    is visible afterwards.
  - Two files in one batch: one call.
  - An Editor that's a background member of a stack: the stack switches to
    it.
  - A minimized Editor behind a magnified sibling: the sibling is
    un-magnified and the Editor restored.
  - An empty or absent batch: no layout call.
  - The window tab is not switched.
- **Other open paths leave it alone:** `openFile` from the tree, a drop, and
  a file-changed reload on a minimized Editor leave it minimized.
- **`revealBlockLocally`:** revealing a minimized pane restores it, and
  still focuses it and gives the caret as before.

## Out of scope, noted from the same incident

- **The pane title doesn't follow.** A reused pane keeps the title of
  whoever created it. Here the header read "Reports per account — spec"
  while showing the agent's two specs. `OpenEditor`'s `title` argument is
  also ignored on the reuse path (`maybe_reuse_editor_pane` takes only
  `file`). Either the header should follow the active file tab, or reuse
  should apply the requested title. That's a separate small spec.
- **Layout focus on reuse:** see `pane.rs:331-350`. Unchanged.
- **Switching window tabs** for an agent's file-open: deliberately not done
  (see Behavior).

## Files

- `frontend/app/util/reveal-block.ts`: `showBlockInPane`,
  `showBlockWithoutFocus`; `revealBlockLocally` uses the first.
- `frontend/layout/lib/types.ts`, `layoutModel.ts`, `layoutMagnify.ts`: the
  `focused` flag on `MagnifyNodeToggle`.
- `frontend/app/view/editor/editor-model.ts`: the drain calls it.
- Tests next to each (`reveal-block.test.ts`, the editor drain's test file).
- No backend change.
