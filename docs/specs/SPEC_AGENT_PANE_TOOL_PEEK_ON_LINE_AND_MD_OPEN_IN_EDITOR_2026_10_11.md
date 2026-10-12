# Agent pane: a tool call's hover peek only on its line, and "Open in editor" on Markdown file previews

**Status:** implemented (#4680).
**Date:** 2026-10-11 | **Version:** main at `cc94d15e0`.
**Requested by:** repo owner: "in the agent pane, when hovering over tool calls (like bash) we want to display the popover only hovering over the actual tool call line. the expandable result/preview should not show the hover. this is so we can see it but also select it. the hover over the tool preview blocks that. Additionally, on the top right of md file previews, add a button to the top right which opens it in an editor pane (with collapsed tree)." Then: "if an editor is open and not minimized in the same window tab, then open the doc in another document tab in the same editor pane tab. If the editor is not on the same window tab, or is minimized or not shown, open a new editor pane", and "a button for all tools (md previews and normal, so that md's have 2 buttons, and normal have 1) … an extend/shorten button, that makes the preview 3x the height (or revert back to original preview height)."
**Author:** Loap.
**Related:** `SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25.md` §1.3 (which widened the tool peek to the whole row), [`frontend/app/view/agent/hooks/useNodePeek.ts`](../../frontend/app/view/agent/hooks/useNodePeek.ts), [`frontend/app/view/agent/components/PeekOverlay.tsx`](../../frontend/app/view/agent/components/PeekOverlay.tsx).

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

A Read or Write of a `.md`/`.mdx` file renders its content as Markdown. `FilePreview` in [`frontend/app/view/agent/components/tool-renderers/builtins.tsx`](../../frontend/app/view/agent/components/tool-renderers/builtins.tsx) (L65–97, `isMarkdownPath` at L55) does it, wrapped in `.agent-tool-read-md` / `.agent-tool-write-md`. Above it, both renderers draw a header row, `.agent-tool-file-path-row`, holding the path (and Read's line range, Write's byte count): `renderRead` L159–164, `renderWrite` L228–236.

Nothing in the agent pane opens a file in an editor today. Markdown links go to `openLink`, which opens an external browser. A PDF Read already has buttons in that position ("Open", "Show in folder", `.agent-tool-read-open`, `builtins.tsx` L169–184), so the pattern exists.

### 2.2 Change

**The button.** On a Markdown file preview (a Read or Write that `FilePreview` renders as Markdown: `markdownPreviewPath()` in `builtins.tsx`), an icon button ("Open in editor", the editor's `file-lines` icon) sits in the preview's top-right actions, beside §3's height toggle. Edit's diff gets no button: it shows a change, not the file.

**Where it opens.** In the editor the agent's tab is showing, if there is one; otherwise in a new editor pane:

- **An editor shown in the agent's own window tab** (the active member of its pane stack, not minimized, and not hidden because another pane is magnified), on the same host: the file opens there as another document tab.
- **Otherwise** (no editor in this tab, or only one that is minimized, behind another pane tab, or hidden by a magnified pane): a new editor pane split right of the agent pane, with its file tree collapsed.

```ts
TabRpcClient.rpcCall("pane.open", {
    view: "editor",
    file: absolutePath,
    split_direction: "right",
    split_reference_block_id: agentBlockId,
    reuse_editor_pane: true,
}, {});
```

- srv already had the reuse half: `reuse_editor_pane` (the agents' `OpenEditor` uses it) finds an editor in the caller's tab and queues the file into it (`editor:pending_open_files`), which the editor opens as a new document tab (`maybe_reuse_editor_pane`, `pane.rs`). What it lacked was "shown": `find_editor_block` took the first editor block in the tab. It now considers only the blocks the tab's layout shows (`shown_block_ids` in `app_api/mod.rs`: each leaf's active member, skipping effectively minimized nodes, and every leaf but the magnified one while one is magnified). That applies to `OpenEditor`'s reuse too: a file pushed into an editor the user can't see would look like nothing happened.
- No `tree_expanded` in the request: srv skips reuse whenever one is given (a reused pane can't take a new tree state). A new pane still gets a collapsed tree: srv defaults `editor:tree_expanded` to false for a Markdown file (`build_pane_meta`), now for `.mdx` as well as `.md`.
- An editor that has just been created and not yet placed by the frontend isn't in the layout yet, so a second click within that moment opens a second pane. Accepted.
- After opening, the button shows the editor it got (`showBlockWithoutFocus`, `reveal-block.ts`: un-magnify, restore, switch its stack to it, without moving focus). Without it, a new pane beside a magnified agent pane stayed hidden behind it, and repeated clicks stacked up hidden editors (Codex on #4680).

**What the button needs from the pane.** Tool renderers get only `node` and a small `ToolRenderContext` (`registry.ts` L30). The button needs the agent pane's block id (for the split) and its working directory (for a relative path). `AgentPaneProviders` (`agent-media.tsx`) already passes the working directory down the render path as a context, deliberately not as props through the virtualized list. It now also provides both through `agent-open-in-editor.tsx`. An agent runs either on this host or in a container (`agent_type == "container"`); there are no SSH agents. A container agent's paths are inside its container, so the context is only provided for an agent known to run on this host: the pane's own `agentMode` meta (`host`/`container`) when set, else a loaded definition whose `agent_type` isn't `container`. While neither is known (definitions still loading, or the definition deleted), there is no button. Outside an agent pane (no context), the button is not shown either.

**Paths.** Read's `file_path` and Write's `file_path` are absolute in practice. A relative one resolves against the pane's working directory, which `AgentPaneProviders` already computes (`cmd:cwd`, else the definition's `working_directory`).

**Failure.** If `pane.open` fails (the file was deleted or moved, or the host is unreachable), show the error the same way the PDF "Open" button does today.

### 2.3 Tests

- `open-in-editor.test.tsx`: a Read and a Write of a `.md` file show the button in the preview's actions, and a `.ts` Read doesn't. Clicking it calls `pane.open` with `view: "editor"`, the path (a relative one under the pane's working directory), `split_direction: "right"`, the pane's block id and `reuse_editor_pane: true`. Without the pane context, or for a container agent, there is no button.
- `pane_open_reducer_tests.rs`: `shown_block_ids` (shown, minimized, behind another pane tab, magnified elsewhere); the existing reuse tests now place the editor in the tab's layout first, as the frontend would.
- Live check in a dev build: with no editor in the tab, the button opens one to the right, tree collapsed. With one shown, the file opens there as a new document tab. With it minimized, a new pane opens.

---

## 3. A taller preview, on every tool

A tool's preview is capped at a third of half the window height (`$transcript-preview-max-height`, `.agent-tool-panel`), and longer output scrolls inside it. Every tool's preview now has a toggle in its top-right actions: "Taller preview" (`expand` icon) sets `.agent-tool-panel--tall`, three times the cap, and "Shorter preview" (`compress`) sets it back. A Markdown file preview therefore has two buttons (Open in editor and the toggle), every other preview one.

- The actions (`.agent-tool-panel-actions`) sit absolutely positioned at the panel's top-right, on the panel's own shade, clear of the log's scrollbar. They're quiet (reduced opacity) until the pointer is on the preview, like a user message's pin button. The panel became `position: relative` for them; nothing else in it was absolutely positioned.
- The state is the tool block's own (`ToolBlock`, a signal): not saved, and back to normal if the block is unmounted (scrolled far out of the virtualized list). The class is only set while the panel is shown, so a collapsed tool's `--hidden` height of 0 still wins.

Tests: every tool's preview has the toggle and a Markdown one has both buttons; the toggle sets and clears `--tall` and swaps its label (`open-in-editor.test.tsx`).

## 4. Open questions

1. **Container agents (decided).** Hidden: the paths are inside the container, and the editor has no way into it.
2. **Other previews.** The same button could go on any file preview (a `.ts` Read, too), and on a Markdown link to a local `.md` file in the agent's own text. The owner asked for Markdown previews; these would be follow-ups.
