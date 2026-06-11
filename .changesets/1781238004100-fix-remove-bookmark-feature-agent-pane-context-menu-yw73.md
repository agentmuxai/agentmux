---
type: patch
---

fix(agent): remove bookmark feature; right-click now shows standard tile context menu

**What changed:**

The bookmark feature has been removed from the agent pane:

- **Right-click on agent feed body** now shows the standard split-right / split-left /
  float / close tile context menu — matching terminal and other pane types.
  Previously `DocumentRow.handleContextMenu` intercepted every right-click on the
  feed body and replaced the tile menu with a bookmark-only menu, making the standard
  tile actions unreachable from the agent pane body.

- **Tool expansion overlay** no longer shows a Bookmark button in the action bar.

- **Node hover strip** no longer shows a bookmark icon on row hover.

- **Ctrl+B** no longer opens a bookmarks panel in the agent view.

- **'b' key** on a focused row no longer triggers a bookmark action.

**Files deleted:**
- `frontend/app/view/agent/hooks/useBookmarks.ts`
- `frontend/app/view/agent/components/BookmarksPanel.tsx`
- `frontend/app/view/agent/styles/_bookmarks.scss`

**Files modified:** `agent-view.tsx`, `agent-view.scss`, `DocumentRow.tsx`,
`NodeHoverStrip.tsx`, `ToolOverlayActions.tsx`, `ToolBlock.tsx`,
`ToolBlockOverlay.tsx`, `AgentDocumentView.tsx`, `AgentDocumentVirtualList.tsx`,
`useAgentKeyboard.ts`, `types.ts`, `gotypes.d.ts`.
