# SPEC: Render `.md` content as markdown in the Write tool overlay

**Date:** 2026-06-23
**Status:** Planned (implemented — see note below)
**Author:** clamk

> **2026-08-07 audit note:** Implemented (`isMarkdown` detection in
> `renderRead`/`renderWrite`, `ToolOverlayLog.tsx`). Status field was never
> updated. See `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Extends:** `SPEC_WRITE_TOOL_CONTENT_VIEW_2026_06_19.md` (implemented in PR #1601)
**Related:** `frontend/app/view/agent/components/ToolOverlayLog.tsx` (`renderWrite`),
             `frontend/app/element/markdown.tsx` (`Markdown`)

---

## 1. Problem

The Write tool overlay currently shows the written content through `HighlightedCode`
for all file types. For `.md` files this means the user sees syntax-highlighted
markdown *source* — raw `#`, `**`, `[links]` — instead of rendered markdown.

The correct experience for `.md` is the same as what the editor pane now shows (PR
#1655): formatted prose, rendered headings, code fences highlighted, links
clickable. The content is already available at `node.params.content` (always
present) — only the renderer choice is wrong.

The fix was not part of `SPEC_WRITE_TOOL_CONTENT_VIEW_2026_06_19.md`, which
deliberately reused the `renderRead` pattern (`HighlightedCode` + `detectLanguage`)
for all types and did not carve out a markdown exception. That decision was correct
for the initial landing; this spec adds the exception.

---

## 2. Current behavior (post PR #1601)

`renderWrite` in `ToolOverlayLog.tsx`:

```
.md file written → detectLanguage("foo.md", ...) → "markdown"
                 → HighlightedCode(code=content, lang="markdown")
                 → Shiki syntax-highlights markdown source (backticks, asterisks, hashes coloured)
```

Not wrong, but visually noisy and unhelpful — the agent just wrote a document the
user wants to *read*, not audit the markdown syntax of.

---

## 3. Design

### 3.1 Branch on extension inside `renderWrite`

Add a single `isMarkdown` check before the `HighlightedCode` branch:

```typescript
const isMarkdown = filePath.endsWith(".md") || filePath.endsWith(".mdx");
```

When `isMarkdown` is true, render the capped content through the `Markdown`
component instead of `HighlightedCode`.

### 3.2 Capping stays the same

Apply `capText(content, MAX_TOOL_OUTPUT_LINES, "head")` regardless of render path.
Markdown documents that are very long (e.g. a generated CHANGELOG written by the
agent) should not bloat the conversation DOM. The cap threshold
(`MAX_TOOL_OUTPUT_LINES`) is the same as for code files.

`OutputHiddenMarker` is shown below the rendered markdown when lines were truncated,
same as today.

### 3.3 Updated `renderWrite`

```typescript
import { Markdown } from "@/app/element/markdown";   // add to imports

function renderWrite(node: ToolNode): JSX.Element {
    const filePath = (node.params as any).file_path ?? "";
    const content: string | undefined = (node.params as any).content;
    const bytes: number | undefined = (node.result as any)?.bytesWritten;
    const capped = content ? capText(content, MAX_TOOL_OUTPUT_LINES, "head") : null;
    const isMarkdown = filePath.endsWith(".md") || filePath.endsWith(".mdx");

    return (
        <div class="agent-tool-write">
            <div class="agent-tool-file-path-row">
                <span class="agent-tool-file-path">{filePath}</span>
                <Show when={bytes != null}>
                    <span class="agent-tool-write-bytes">{formatBytes(bytes!)}</span>
                </Show>
            </div>
            <Show when={capped} fallback={<div class="agent-tool-write-info">No content written.</div>}>
                <Show
                    when={isMarkdown}
                    fallback={
                        <HighlightedCode
                            code={capped!.text}
                            lang={detectLanguage(filePath, capped!.text.split("\n")[0])}
                            class="agent-tool-write-content"
                        />
                    }
                >
                    <div class="agent-tool-write-content agent-tool-write-md">
                        <Markdown text={capped!.text} />
                    </div>
                </Show>
                <Show when={capped!.hiddenLines > 0}>
                    <OutputHiddenMarker hidden={capped!.hiddenLines} noun="line" from="head" />
                </Show>
            </Show>
        </div>
    );
}
```

### 3.4 Styling

Add `.agent-tool-write-md` to the existing SCSS for the Write tool
(`frontend/app/view/agent/styles/_document-nodes.scss`):

```scss
.agent-tool-write-md {
    padding: 8px 12px;
    overflow-x: auto;

    // Inherit the conversation markdown prose styles.
    // This reuses .agent-markdown (from MarkdownBlock) which already sizes
    // headings, code fences, lists, and links correctly for the agent pane.
    @extend .agent-markdown;
}
```

If `.agent-markdown` is not directly extendable (it may be scoped to a parent),
the fallback is to pull in the same font-size/line-height/prose rules inline.
Confirm at implementation time by checking `_document-nodes.scss` and
`agent-view.scss` for where `.agent-markdown` is defined.

---

## 4. Why not the tool-renderer registry?

The registry (`SPEC_TOOL_RESULT_RENDERER_REGISTRY_2026_06_17.md`) would let a
separate module register a higher-priority Write renderer keyed on
`file_path.endsWith(".md")`. That's a good long-term pattern if Write gets many
per-type variants (e.g. a dedicated JSON viewer, an SVG preview).

For now there is exactly one variant (markdown), and the existing `renderWrite` is
already modular (it's a registered renderer itself at priority 0). A one-line
`isMarkdown` branch inside it is less indirection for the same result. If a second
per-type variant materialises, extract to the registry then.

---

## 5. Files changed

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/ToolOverlayLog.tsx` | Add `Markdown` import; add `isMarkdown` branch inside `renderWrite` |
| `frontend/app/view/agent/styles/_document-nodes.scss` | Add `.agent-tool-write-md` prose wrapper styles |

No new components. `Markdown` is already used in the editor pane and `MarkdownBlock`
pulls it transitively — this adds one direct import to `ToolOverlayLog.tsx`.

---

## 6. Scope

**In scope:**
- Write tool overlay only
- `.md` and `.mdx` extensions

**Explicitly out of scope (follow-up if desired):**
- Read tool: the same extension check would apply identically, but the Write case
  is more common (agents write docs more than they read them in UX-critical spots)
  and is the user-reported pain point. Read is a separate PR.
- Streaming Write (if Write ever streams its content live): the `Markdown` component
  accepts a `textAtom` accessor for reactive updates; swap `text={capped!.text}` for
  `textAtom={() => capped!.text}` when content arrives incrementally.

---

## 7. Acceptance criteria

- Write tool overlay for a `.md` file shows rendered markdown (headings, bold, code
  fences highlighted, links) — not syntax-coloured markdown source
- Write tool overlay for any non-`.md` file (`.ts`, `.json`, `.py`, …) is unchanged
- Content is still capped at `MAX_TOOL_OUTPUT_LINES`; `OutputHiddenMarker` shown
  when truncated
- `npx tsc -p tsconfig.json --noEmit` clean
- No regression on Read, Bash, Edit, or Grep tool overlays
