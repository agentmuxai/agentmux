# SPEC: Remove Bookmark Feature — 2026-06-11

## Motivation

The bookmark feature was an early experiment for marking messages in the agent
pane feed. It is being removed because:

1. **Wrong context menu in the agent pane.** Right-clicking anywhere in the
   agent pane body shows a "Bookmark this message" / "Remove bookmark" menu.
   Other panes (terminal, editor, browser) bubble the right-click up to the
   block-frame, which shows the standard split-right / split-left / float / close
   context menu. The bookmark context menu in DocumentRow intercepts all
   right-clicks on the feed and hides those standard tile actions from the user.

2. **Bookmark button in tool-expansion overlay.** The ToolOverlayActions bar
   shows a "Bookmark" button as the first action in the tool overlay footer.
   This clutters the overlay and has no replacement; the removal leaves only
   the remaining overlay actions (copy, etc.).

3. **Feature is not used enough to justify its surface area.** The
   `agent:bookmarks` block-meta key, the BookmarksPanel, the NodeHoverStrip
   icon, the keyboard shortcut (Ctrl+B), and the per-row visual indicator all
   add complexity with no corresponding user value at this stage.

## Scope

Remove the entire bookmark feature from the frontend. No backend changes are
required (block meta is a free-form map; the `agent:bookmarks` key simply stops
being written and the type annotation in `gotypes.d.ts` is removed).

---

## Files to Delete (complete removal)

| File | Description |
|------|-------------|
| `frontend/app/view/agent/hooks/useBookmarks.ts` | Hook managing bookmark CRUD and persistence |
| `frontend/app/view/agent/components/BookmarksPanel.tsx` | Collapsible panel listing bookmarks |
| `frontend/app/view/agent/styles/_bookmarks.scss` | All bookmark-related CSS |

---

## Files to Modify

### 1. `frontend/app/view/agent/types.ts`

Remove the `Bookmark` interface (approx. lines 450–464):
```ts
// DELETE:
export interface Bookmark { ... }
```

### 2. `frontend/types/gotypes.d.ts`

Remove the `agent:bookmarks` meta key declaration:
```ts
// DELETE line:
"agent:bookmarks"?: unknown[];
```

### 3. `frontend/app/view/agent/agent-view.tsx`

- Remove `useBookmarks` import and call site (~lines 661–666).
- Remove `useAgentKeyboard` `onToggleBookmarks` callback and Ctrl+B binding
  (~lines 676–687).
- Remove `<Show when={bookmarks.visible()}>…<BookmarksPanel …/></Show>` from
  JSX (~lines 784–791).
- Remove `bookmarkedNodeIds` and `onBookmark` props passed to
  `AgentDocumentView` (~lines 841–842).
- Remove any import of `BookmarksPanel`, `useBookmarks`, `useScrollToNode`
  (if `useScrollToNode` is no longer needed — verify other consumers first).

> **Note on `useScrollToNode`:** If `scroll.jumpTo` was only wired through
> bookmarks, verify whether anything else uses `useScrollToNode` before
> removing it. If it is unused after the bookmark removal, delete the hook
> and its call site too.

### 4. `frontend/app/view/agent/agent-view.scss`

Remove the bookmark stylesheet import:
```scss
// DELETE:
@use "styles/bookmarks";
```

### 5. `frontend/app/view/agent/virtualization/DocumentRow.tsx`

- **Remove right-click context menu** (`handleContextMenu`, ~lines 125–142).
  This is the critical change: without this handler, right-clicks on the agent
  feed body bubble up to the tile/block-frame and reach the standard
  split-right / split-left / float / close context menu, matching terminal
  and other pane behaviour.
- Remove `isBookmarked` derived signal (~line 72–75).
- Remove `"agent-node-bookmarked"` CSS class application from the row div
  (~line 149).
- Remove 'b' keyboard shortcut handler (~lines 115–117).
- Remove `onBookmark` from the props interface and all downstream passing to
  `NodeHoverStrip` and `DocumentNodeBody`.
- Remove `bookmarkedNodeIds` from props and any `.has()` checks.

### 6. `frontend/app/view/agent/components/NodeHoverStrip.tsx`

Remove the bookmark button (~lines 89–96):
```tsx
// DELETE:
<Show when={props.onBookmark != null}>
    <HoverStripButton
        icon="🔖"
        label={props.isBookmarked ? "Remove bookmark" : "Bookmark"}
        active={props.isBookmarked === true}
        onClick={props.onBookmark}
    />
</Show>
```
Remove `isBookmarked` and `onBookmark` from `NodeHoverStripProps`.

### 7. `frontend/app/view/agent/components/ToolOverlayActions.tsx`

Remove the bookmark button from the tool overlay footer action bar (~lines
62–70):
```tsx
// DELETE:
<Show when={props.onBookmark != null}>
    <OverlayActionButton
        label={props.isBookmarked ? "Bookmarked" : "Bookmark"}
        icon="🔖"
        onClick={props.onBookmark}
    />
</Show>
```
Remove `isBookmarked` and `onBookmark` from `ToolOverlayActionsProps`.

### 8. `frontend/app/view/agent/components/ToolBlock.tsx`

Remove `isBookmarked` and `onBookmark` from the props interface and from any
prop-passing to `ToolOverlayActions` (~lines 47–54).

### 9. `frontend/app/view/agent/components/AgentDocumentView.tsx` (and VirtualList)

Remove `bookmarkedNodeIds` and `onBookmark` from the props chain:
- `AgentDocumentView` props interface
- `AgentDocumentVirtualList` props interface
- All call sites that pass these props down through the tree

---

## Context Menu: What Replaces the Bookmark Menu

The bookmark context menu is the **only** `onContextMenu` handler on the
document row / feed body. Removing it means right-clicks propagate normally to
the block-frame, which already has its own `onContextMenu` handler (`blockframe.tsx`,
`data-role="block-header"`) that shows the standard tile context menu
(split-right, split-left, split-up, split-down, float, close).

**No new code is needed.** The standard tile actions become available on
right-click automatically once the intercepting bookmark handler is removed.
Verify this on:

- Right-click anywhere on agent feed body → should show split/float/close menu
- Right-click on pane header → already works, unchanged

---

## Keyboard Shortcut

The Ctrl+B shortcut (toggle bookmarks panel) is removed along with the
`onToggleBookmarks` callback in `useAgentKeyboard`. If `useAgentKeyboard` has
no remaining callbacks after this, evaluate whether the hook itself should be
simplified or removed.

---

## Testing Checklist

- [ ] Right-click on agent pane body shows split-right / split-left / float /
  close — same as terminal
- [ ] Right-click on pane header still works (unchanged)
- [ ] Tool expansion overlay no longer shows a Bookmark button
- [ ] Node hover strip no longer shows a bookmark icon
- [ ] No `agent-node-bookmarked` left-border indicator visible on any row
- [ ] Ctrl+B no longer opens any panel in the agent view
- [ ] 'b' key on a focused row does nothing bookmark-related
- [ ] No TypeScript errors (`tsc --noEmit`)
- [ ] No `@use "styles/bookmarks"` remaining in any `.scss` import
- [ ] `agent:bookmarks` does not appear in any source file (grep check)

---

## Non-Goals

- No migration or cleanup of existing block metadata. Old blocks may have an
  `agent:bookmarks` key in their metadata object; this is harmless — the key
  is simply ignored once nothing reads it.
- No backend changes.
- No `useScrollToNode` removal unless confirmed unused after bookmark wiring
  is removed (it may have other callers).
