# Agent pane: a tool call's hover peek only on its line, and "Open in editor" on Markdown file previews

**Status:** spec, not implemented.
**Date:** 2026-10-11 | **Version:** main at `cc94d15e0`.
**Requested by:** repo owner: "in the agent pane, when hovering over tool calls (like bash) we want to display the popover only hovering over the actual tool call line. the expandable result/preview should not show the hover. this is so we can see it but also select it. the hover over the tool preview blocks that. Additionally, on the top right of md file previews, add a button to the top right which opens it in an editor pane (with collapsed tree)."
**Author:** Loap.
**Related:** `SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25.md` §1.3 (which widened the tool peek to the whole row), `frontend/app/view/agent/hooks/useNodePeek.ts`, `frontend/app/view/agent/components/PeekOverlay.tsx`.

---

## 1. The tool peek opens over the output you are trying to read

### 1.1 Today

`ToolBlock.tsx` puts the peek's `onMouseEnter`/`onMouseLeave` and its anchor (`ref={setPeekRowEl}`) on the whole `.agent-tool-block`. That element holds the one-line summary (`.agent-tool-summary`) and, when the tool is expanded, its panel (`.agent-tool-panel`: the command's output, a file preview, a diff).

The 2026-08-25 spec moved the handlers there on purpose, so that hovering anywhere on the row, not just the tool name, shows the peek. Back then a tool was usually collapsed, so "the row" meant one line. Expanded, the same handlers fire over the whole output. The peek (a Portal-rendered panel that tracks the mouse's Y position, `SPEC_PEEK_OVERLAY_MOUSE_Y_TRACKING_2026_09_03.md`) then sits on top of the text you are reading or selecting. It also opens for an expanded tool, where the peek's content (time, tokens) is the least useful.

`PersistentShellBlock.tsx` (a background shell's row) has the same wiring: the handlers are on `.agent-shell-block`, which holds its output when expanded.

### 1.2 Change

The peek belongs to the **tool call's line**, collapsed or expanded:

1. **`ToolBlock.tsx`:** move `handlePeekEnter`/`handlePeekLeave` and `ref={setPeekRowEl}` from `.agent-tool-block` to `.agent-tool-summary`. The block keeps its own `onMouseEnter`/`onMouseLeave` for `userHolding` only. That is the hold that keeps a pinned, expanded tool open while the pointer is over it (§1.3 of the 2026-08-25 spec), and it still needs the whole block.
2. **`PersistentShellBlock.tsx`:** the same, from `.agent-shell-block` to `.agent-shell-summary`.
3. Moving the anchor too, not only the handlers, keeps the peek's position and its "pointer moved onto the panel" bridge (`peek-bridge.ts`, `peekBridgeRow`) tied to the line. If only the handlers moved, the panel would still be placed against the whole block and could cover the output it should leave alone.

Result: hover the line, get the peek. Move down into the output and the peek closes (or never opens). The output can be read and selected with nothing over it.

Unchanged:
- The peek still opens anywhere on the line, not only on the tool name. That keeps the point of the 2026-08-25 change.
- The 50 ms delay, the "don't open mid-drag" rule (`useNodePeek`), and the peek's content.
- Other node kinds (assistant text, user messages, thinking, notices). They have no expandable output under a summary line, so their whole row is still "the line". If text selection in long assistant messages turns out to have the same problem, that is a separate change.

### 1.3 Tests

- `ToolBlock.test.tsx`: hovering `.agent-tool-summary` shows `.agent-node-peek-overlay` (after the delay). Hovering `.agent-tool-panel` of an expanded tool does not. Moving from the summary into the panel closes it. The existing tests that hover the block need to hover the summary instead.
- A test that `userHolding` still engages when the pointer is over an expanded, pinned tool's panel (the block's own handlers).
- The same pair for `PersistentShellBlock`.
- Live check in a dev build: expand a Bash tool with a few lines of output, drag-select inside the output (no peek appears), hover the line (the peek appears and follows the pointer's Y position).

---

## 2. "Open in editor" on a Markdown file preview

### 2.1 Today

A Read or Write of a `.md`/`.mdx` file renders its content as Markdown. `FilePreview` in `frontend/app/view/agent/components/tool-renderers/builtins.tsx` (L65–97, `isMarkdownPath` at L55) does it, wrapped in `.agent-tool-read-md` / `.agent-tool-write-md`. Above it, both renderers draw a header row, `.agent-tool-file-path-row`, holding the path (and Read's line range, Write's byte count): `renderRead` L159–164, `renderWrite` L228–236.

Nothing in the agent pane opens a file in an editor today. Markdown links go to `openLink`, which opens an external browser. A PDF Read already has buttons in that position ("Open", "Show in folder", `.agent-tool-read-open`, `builtins.tsx` L169–184), so the pattern exists.

### 2.2 Change

**The button.** On a Markdown file preview (Read or Write of a path `isMarkdownPath` accepts), add an icon button at the right end of `.agent-tool-file-path-row` (`margin-left: auto`). It uses the `file-pen` icon (or the editor's own icon), with the label and tooltip "Open in editor". Its style follows `.agent-tool-read-open`. Edit's diff gets no button: it shows a change, not the file.

**What it opens.** An editor pane on that file, split to the right of the agent pane, with the file tree collapsed:

```ts
TabRpcClient.rpcCall("pane.open", {
    view: "editor",
    file: absolutePath,
    split_direction: "right",
    split_reference_block_id: agentBlockId,
    tree_expanded: false,
    ...(connection ? { connection } : {}),
}, {});
```

- `pane.open` is what the editor's own "Open to the side" uses (`editor-model.ts` L1258–1269), and `openInPane` in `frontend/app/view/files/files-open.ts` (L48–68) wraps it for the Files pane.
- srv already collapses the tree for a `.md` file when `tree_expanded` is absent (`crates/srv/src/server/app_api/pane.rs` `build_pane_meta`, L229–251). This spec passes `tree_expanded: false` anyway, so the button means the same thing for `.mdx` (which srv's check, `.md` only, misses).
- Clicking twice opens two editor panes. Reusing an open editor pane on the same file (srv's `reuse_editor_pane`, `pane.rs` L1082–1095) is skipped whenever `tree_expanded` is passed. Focusing an editor that already shows the file, instead of opening another, is a possible follow-up, not part of this change.

**What the button needs from the pane.** Tool renderers get only `node` and a small `ToolRenderContext` (`registry.ts` L30). The button needs the agent pane's block id (for the split) and its `connection` (an SSH agent's file lives on that host). Add both to the pane's render-path contexts in `agent-media.tsx` (`AgentPaneProviders`, which already passes the working directory down the same way, deliberately not as props through the virtualized list). Outside an agent pane (no context), the button is not shown.

**Paths.** Read's `file_path` and Write's `file_path` are absolute in practice. A relative one resolves against the pane's working directory, which `AgentPaneProviders` already computes (`cmd:cwd`, else the definition's `working_directory`).

**Failure.** If `pane.open` fails (the file was deleted or moved, or the host is unreachable), show the error the same way the PDF "Open" button does today.

### 2.3 Tests

- `builtins` renderer tests: a Read and a Write of `notes.md` render the button, and a `.ts` Read doesn't. Clicking it calls `pane.open` with `view: "editor"`, the path, `split_direction: "right"`, the pane's block id, `tree_expanded: false`, and, for an SSH agent, its `connection`. Without the pane context there is no button.
- Live check in a dev build: an agent Reads a `.md` file, expand the tool, click the button. An editor opens to the right on that file with the tree collapsed.

---

## 3. Open questions

1. **Container agents.** A container agent's paths are inside its container. Should the button hide there, or open through the container's own file access, if the editor has one? Proposed: hide it unless the pane's connection is local or SSH.
2. **Other previews.** The same button could go on any file preview (a `.ts` Read, too), and on a Markdown link to a local `.md` file in the agent's own text. The owner asked for Markdown previews; these would be follow-ups.
