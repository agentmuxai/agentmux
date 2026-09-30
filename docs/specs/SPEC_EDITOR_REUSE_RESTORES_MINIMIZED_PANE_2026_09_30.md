# Spec: An agent opening a file restores a minimized Editor pane

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
a file". A file opened into a minimized pane defeats the point of the call.

## Behavior

When an agent's `OpenEditor` call pushes a file into an existing Editor pane
that is minimized, the pane is restored. It returns to its size before
minimizing, with the newly opened file as its active tab.

- **Only the agent-initiated path restores.** Files the user opens from the
  editor's own tree, drag-and-drop, and live reloads of changed files don't
  change the pane's minimized state.
- **An expanded pane is untouched.** Nothing about its size, position or
  other tabs changes.
- **Only that pane.** Other panes' minimized or expanded state doesn't
  change.
- **Once per batch.** Several files arriving together (queued while the pane
  wasn't mounted) restore the pane once.
- **No layout focus.** Moving layout focus to the reused pane remains the
  known, separate limitation documented in `pane.rs` (see below). The
  restored pane shows the file; it doesn't take the focus ring.

The user minimizing the pane again afterwards sticks, until the next
agent-initiated open.

## Design

### Where

In `EditorViewModel`'s `META_PENDING_OPEN_FILES` drain
(`frontend/app/view/editor/editor-model.ts`, the `createEffect` around
`:266-305`). That effect is the one and only consumer of reuse opens: files
pushed by `maybe_reuse_editor_pane`
(`agentmux-srv/src/server/app_api/pane.rs:351-409`).

After the effect schedules `openFile()` for the paths it processed, and only
when that list is non-empty:

1. Resolve the `LayoutModel` that owns this block's tab.
2. Find this block's leaf: `layoutModel.getNodeByBlockId(this.blockId)`.
3. If the leaf has `minimized: true`, call
   `layoutModel.minimizeNodeToggle(leaf.id)`. That takes the restore branch
   of `layoutMinimize.ts::minimizeNodeToggle`, which clears the flag, runs
   `updateTree`, and persists.

### Why the frontend, not the backend

The obvious alternative is to clear the flag server-side in
`maybe_reuse_editor_pane`, next to the meta write. It wouldn't work, for the
same reason the same function doesn't set layout focus (`pane.rs:331-350`).
The frontend's `onBackendUpdate` (`layoutPersistence.ts`) doesn't apply
backend layout-tree changes to an already-mounted `LayoutModel`, outside its
two narrow triggers. A backend edit would compile, pass a reducer test, and
have no visible effect. The drain effect already runs in the right place, at
the right time, for both the already-mounted and not-yet-mounted cases.

### Not-yet-mounted panes

If the reused pane isn't mounted when the file is queued (for example, its
tab isn't active), the drain runs when the pane mounts. The restore happens
then, the first time the user can see the pane.

### Guard

`minimizeNodeToggle` refuses to *minimize* the last expanded pane. Restore
has no such guard and needs none. Calling it only when the leaf is minimized
keeps an expanded pane from being minimized by mistake, since the function
is a toggle.

## Tests

- **`editor-model` drain restores:**
  - Given a minimized leaf for the editor's block, a meta update adding
    `editor:pending_open_files` calls the restore once, and the leaf is no
    longer minimized.
  - Two paths in one update: one restore.
  - The leaf is not minimized: `minimizeNodeToggle` is never called.
  - An empty or absent pending list: no layout call.
- **Other open paths leave it alone:** `openFile` from the tree, a drop, and
  a file-changed reload on a minimized pane leave it minimized.
- **Mount order:** the pane mounts with a pending file already queued and
  minimized persisted in the tree. It is restored after mount.

## Out of scope, noted from the same incident

- **The pane title doesn't follow.** A reused pane keeps the title of
  whoever created it. Here the header read "Reports per account — spec"
  while showing the agent's two specs. `OpenEditor`'s `title` argument is
  also ignored on the reuse path (`maybe_reuse_editor_pane` takes only
  `file`). Either the header should follow the active file tab, or reuse
  should apply the requested title. That's a separate small spec.
- **Layout focus on reuse:** see `pane.rs:331-350`. Unchanged.

## Files

- `frontend/app/view/editor/editor-model.ts`: the drain effect.
- `frontend/app/view/editor/editor-model.test.ts` (or the existing drain
  test file): new cases.
- No backend change.
