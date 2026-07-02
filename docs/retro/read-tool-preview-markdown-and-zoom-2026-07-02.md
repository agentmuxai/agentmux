# Retro: Read-tool preview — markdown rendering + zoom targeting

**Date:** 2026-07-02
**Author:** Agent2
**Area:** `frontend/app/view/agent/` — agent-pane tool overlay (the `Read` tool's file preview)
**Status:** Analysis + intended fix (implementation follow-up)

---

## 1. What we set out to refine

Two rough edges in the agent pane's **Read-tool preview** (the file-path header + file-content preview shown when the agent reads a file):

1. **Markdown files render as raw code, not formatted markdown.** A `.md` file previewed via `Read` shows syntax-highlighted source, not a rendered markdown view.
2. **Zoom targets the wrong element.** Ctrl+scroll over the preview is supposed to resize the *preview text*. Instead, the **file-path header** resizes and the **preview body stays fixed**. Expected behavior:
   - Hovering the **actual preview** → zoom the preview content.
   - Hovering **anywhere else, including the filename above the preview** → zoom the whole agent pane (the normal per-pane zoom).

---

## 2. What we found (root cause)

### 2.1 The preview render path

- `Read` results are rendered by **`renderRead()`** in
  `frontend/app/view/agent/components/ToolOverlayLog.tsx:270`, registered as the
  `Read` renderer (`registerToolRenderer({ … match: byKind("Read"), render: renderRead })`, line 383).
- `renderRead` returns:
  ```
  <div class="agent-tool-read">
    <div class="agent-tool-file-path">{filePath}</div>     // header
    <HighlightedCode class="agent-tool-read-content" … />   // body (Shiki-highlighted <pre>)
  </div>
  ```
- Both are inside **`.agent-tool-overlay-log`** (`ToolOverlayLog.tsx:167`), the element the preview zoom scales.

### 2.2 Bug #1 — markdown never rendered

`renderRead` **unconditionally** uses `<HighlightedCode>` — there is **no `.md`/`.mdx` branch**. The `Write` tool renderer (`renderWrite`, same file, ~line 307) already does the right thing:

```
const isMarkdown = filePath.endsWith(".md") || filePath.endsWith(".mdx");
<Show when={isMarkdown} fallback={<HighlightedCode … />}>
    <div class="agent-tool-write-content agent-tool-write-md"><Markdown text={…} /></div>
</Show>
```

**Why it happened:** the two renderers were written/evolved separately; the markdown branch was added to `renderWrite` but never back-ported to `renderRead`. They drifted.

### 2.3 Bug #2 — the wrong element scales on zoom

There are **two independent zoom systems**, and they collide on the preview:

| Zoom | Where | What it scales |
|------|-------|----------------|
| **Preview zoom** (ephemeral) | `ToolBlock.tsx:79–102` — Ctrl+wheel listener on the whole `.agent-tool-panel` (`panelRef`); drives `previewFontScale` | applied as inline **`font-size: <scale>%`** on `.agent-tool-overlay-log` (`ToolOverlayLog.tsx:170`) |
| **Pane zoom** (persisted) | `agent-view.tsx` — `term:zoom` block meta → CSS **`zoom`** on `.agent-view` | the whole pane |

The preview zoom sets a **percentage** `font-size` on the container. Cascade outcome:

- `.agent-tool-file-path` has **no explicit `font-size`** → it **inherits** the container `%` → **it scales.** ← the thing the user sees moving.
- `.agent-highlighted-code` (the preview body `<pre>`) is pinned to **`font-size: 12px`** (`styles/_document-nodes.scss:471`, absolute px) → the container `%` **cannot cascade into it** → **the code text does not scale.** ← the thing the user *expects* to move.

So the preview zoom resizes exactly the wrong element (the header) and leaves the body untouched. Precisely the reported symptom: *"the only thing that changes is the size of the filepath."*

### 2.4 Bug #2b — no hover-target routing

The Ctrl+wheel handler is attached to the **entire** `.agent-tool-panel` and unconditionally `preventDefault()` + `stopPropagation()` for any Ctrl+wheel inside it (`ToolBlock.tsx:91–98`). There is **no check for what's under the pointer**, so:

- Ctrl+wheel over the **filename** triggers the *preview* zoom (wrong — should be pane zoom).
- Ctrl+wheel anywhere in the panel is swallowed, so it can never fall through to the pane zoom.

---

## 3. The intended fix

### Fix #1 — render markdown in the Read preview
Mirror `renderWrite`: in `renderRead`, add
`const isMarkdown = filePath.endsWith(".md") || filePath.endsWith(".mdx")`
and render `<Markdown text={…}>` for markdown, `<HighlightedCode>` otherwise. Reuse the existing `<Markdown>` component and the `.agent-tool-*-md` styling so Read matches Write.

### Fix #2 — make the preview zoom scale the preview body, not the header
Two parts:

- **Body scales:** the preview content must respond to `previewFontScale`. Either (a) change `.agent-highlighted-code` (and the markdown container) from `font-size: 12px` to a **relative unit** (e.g. `1em`/a `%` off a container base) so the container `%` cascades, or (b) apply the scale directly to the content element rather than the outer log. The markdown preview must scale the same way.
- **Header excluded from preview zoom:** the `.agent-tool-file-path` must **not** be inside the preview-zoom `font-size` scope. Move it out of the scaled container (or give it a fixed size that ignores the preview `%`) so it scales **only** with the pane zoom.

### Fix #3 — route the zoom by hover target
The Ctrl+wheel handler should only claim the event when the pointer is over the **actual preview content**:
- Over the preview body → handle it (preview zoom), `preventDefault`/`stopPropagation` as today.
- Over the filename header or anywhere else → **do nothing**, let the event bubble to the normal pane-zoom path (`term:zoom`).

Implementation note: attach the listener to the **content element** (not the whole panel), or gate on `e.target.closest('.agent-tool-read-content, .agent-tool-write-content')` before claiming the event.

---

## 4. Lessons

1. **Renderer drift.** `renderRead` and `renderWrite` are near-duplicates that diverged — the markdown branch existed in one and not the other. A shared helper (`renderFilePreview(filePath, text, mode)`) for both would prevent this class of drift.
2. **Percentage zoom + absolute child font-size don't compose.** Setting `font-size: %` on a container silently no-ops for any descendant that pins an absolute `px` font-size. Zoom that must reach code blocks has to either scale a relative unit or target the code element directly. The `12px` on `.agent-highlighted-code` quietly defeated the whole preview-zoom feature for the body.
3. **Panel-wide event capture hides intent.** A Ctrl+wheel handler on the whole panel that always `preventDefault`s can't express "zoom the preview *here*, the pane *there*." Hover/target-scoped handling is required for the two-zoom model to feel right.
4. **Two zoom systems need an explicit boundary.** Preview zoom (ephemeral `font-size`) and pane zoom (persisted CSS `zoom`) coexist; the header sitting inside the preview-zoom scope while conceptually belonging to the pane is the core mismatch.

---

## 5. Follow-up

Implement Fixes #1–#3 (frontend-only, hot-reloadable), then verify:
- A `.md` Read preview renders as formatted markdown.
- Ctrl+wheel over the preview body resizes the body (code and markdown); the filename does not change with it.
- Ctrl+wheel over the filename (or elsewhere) zooms the whole pane.

**References**
- `frontend/app/view/agent/components/ToolOverlayLog.tsx` — `renderRead` (270), `renderWrite` (~307), fontScale application (170)
- `frontend/app/view/agent/components/ToolBlock.tsx` — preview-zoom Ctrl+wheel handler (79–102), panel/overlay wiring (325–347)
- `frontend/app/view/agent/components/ToolBlockOverlay.tsx` — passes `previewFontScale` → `ToolOverlayLog`
- `frontend/app/view/agent/components/HighlightedCode.tsx` — `<pre class="agent-highlighted-code">`
- `frontend/app/view/agent/styles/_document-nodes.scss:471` — `.agent-highlighted-code { font-size: 12px }` (the cascade-blocker)
- `frontend/app/view/agent/agent-view.tsx` — pane zoom (`term:zoom` → CSS `zoom`)
