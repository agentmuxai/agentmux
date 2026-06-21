# SPEC — Editor Markdown Live Preview Panel

**Date:** 2026-06-21
**Status:** Proposed

---

## Goal

When editing a `.md` file in the editor pane, a collapsible live-preview panel
appears in the bottom half of the editor body. The user can write in CodeMirror
(top) and watch the rendered result update in real-time (bottom) — or collapse
the panel to a slim strip when they don't need it.

This replaces the existing **full-overlay toggle** (`showRendered()` /
`editor-md-preview` / `editor-md-toggle` button / Mod-Shift-V) with a
non-destructive split: the source is always visible; the preview slides in below
it.

---

## What exists — reuse, don't reimplement

| Existing piece | Where | How it's reused |
|---|---|---|
| `<Markdown textAtom={() => liveDoc()} />` | `editor-view.tsx:851` | Exact same call, same signal — drop into the new panel |
| `liveDoc` signal | `editor-view.tsx:113` | Already seeded on tab switch + every keystroke |
| `isMarkdown()` guard | `editor-view.tsx:114` | Show panel only when true |
| `META_TREE_EXPANDED` / `persistMeta()` pattern | `editor-model.ts:39,803` | Add `META_PREVIEW_OPEN` + `META_PREVIEW_HEIGHT` with identical plumbing |
| Tree resize-handle drag | `editor-view.tsx:430–447` | Copy `handleResizeMouseDown` verbatim, swap `setTreeWidth` → `setPreviewHeight` |
| Chevron toggle button | `AgentComposerStrip.tsx:267–288` | Same `▾`/`▴` + `aria-expanded`/`aria-controls` contract |

---

## Layout

```
editor-main-column (flex-column)
├── EditorTabStrip
├── [error / loading / banner slots]
└── editor-body-wrap (flex-column, height: 100%)
    ├── editor-codemirror            flex: 1 1 auto; overflow: hidden
    ├── editor-preview-divider       4px drag handle, .md only, when expanded
    └── editor-preview-pane          flex: 0 0 <previewHeight>px; .md only
        ├── editor-preview-header    28px strip: "Preview" label + chevron button
        └── editor-preview-content   overflow-y: auto; <Markdown textAtom={...} />
```

When the panel is **collapsed**, `editor-preview-divider` is hidden and the pane
height collapses to the header height only (28px). The `editor-codemirror` flex
item fills the freed space automatically.

---

## State & persistence

Two new block meta keys (same pattern as `editor:tree_expanded` /
`editor:tree_width`):

| Constant | Block meta key | Type | Default |
|---|---|---|---|
| `META_PREVIEW_OPEN` | `"editor:preview_open"` | boolean | `true` |
| `META_PREVIEW_HEIGHT` | `"editor:preview_height"` | number (px) | `300` |

In `EditorModel`:

```ts
const META_PREVIEW_OPEN   = "editor:preview_open";
const META_PREVIEW_HEIGHT = "editor:preview_height";

const PREVIEW_HEIGHT_MIN = 80;
const PREVIEW_HEIGHT_MAX = 1200;
const PREVIEW_HEIGHT_DEFAULT = 300;

private _previewOpen   = createSignal<boolean>(true);
private _previewHeight = createSignal<number>(PREVIEW_HEIGHT_DEFAULT);

previewOpenAtom:   Accessor<boolean> = this._previewOpen[0];
previewHeightAtom: Accessor<number>  = this._previewHeight[0];

// Hydrate in the same block that reads META_TREE_EXPANDED:
if (meta?.[META_PREVIEW_OPEN] === false) this._previewOpen[1](false);
const h = meta?.[META_PREVIEW_HEIGHT];
if (typeof h === "number") this._previewHeight[1](clamp(h, PREVIEW_HEIGHT_MIN, PREVIEW_HEIGHT_MAX));

// Toggle:
async togglePreview(): Promise<void> {
    const next = !this._previewOpen[0]();
    this._previewOpen[1](next);
    await this.persistMeta({ [META_PREVIEW_OPEN]: next });
}

// Commit height (called on mouseup, like commitTreeWidth):
async commitPreviewHeight(): Promise<void> {
    await this.persistMeta({ [META_PREVIEW_HEIGHT]: this._previewHeight[0]() });
}
```

---

## View changes (`editor-view.tsx`)

### Remove

- `mdSourceTabs` signal and all `setMdSourceTabs` / `mdSourceTabs()` calls
- `showRendered()` computed
- `toggleMdMode()` function
- The `<Show when={showRendered()}>…<div class="editor-md-preview">…</Show>` overlay
- The `<Show when={isMarkdown()}>…<button class="editor-md-toggle">…</Show>` toggle button
- The `Mod-Shift-V` keydown handler body (key stays, action changes — see below)

### Add

Resize handler (verbatim copy of `handleResizeMouseDown`, renaming tree→preview):

```tsx
const handlePreviewResizeMouseDown = (e: MouseEvent) => {
    e.preventDefault();
    const startY = e.clientY;
    const startH = model.previewHeightAtom();
    const onMove = (ev: MouseEvent) => {
        const delta = startY - ev.clientY; // drag up → taller preview
        model.setPreviewHeight(startH + delta);
    };
    const onUp = () => {
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        void model.commitPreviewHeight();
    };
    document.body.style.cursor = "row-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
};
```

Mod-Shift-V now calls `model.togglePreview()` instead of `toggleMdMode()`.

In the JSX, replace the old overlay + toggle button with:

```tsx
<div class="editor-body-wrap">
    <div class="editor-codemirror" ref={setContainerRef} />

    <Show when={isMarkdown()}>
        <Show when={model.previewOpenAtom()}>
            <div
                class="editor-preview-divider"
                onMouseDown={handlePreviewResizeMouseDown}
                title="Drag to resize preview"
            />
        </Show>
        <div
            class="editor-preview-pane"
            classList={{ "editor-preview-pane--collapsed": !model.previewOpenAtom() }}
            style={model.previewOpenAtom()
                ? { height: `${model.previewHeightAtom()}px` }
                : undefined}
            id="editor-preview-panel"
        >
            <div class="editor-preview-header">
                <span class="editor-preview-header-label">Preview</span>
                <button
                    type="button"
                    class="editor-preview-chevron"
                    aria-expanded={model.previewOpenAtom()}
                    aria-controls="editor-preview-panel"
                    aria-label={model.previewOpenAtom() ? "Collapse preview" : "Expand preview (Ctrl+Shift+V)"}
                    onClick={() => void model.togglePreview()}
                >
                    {model.previewOpenAtom() ? "▴" : "▾"}
                </button>
            </div>
            <Show when={model.previewOpenAtom()}>
                <div class="editor-preview-content">
                    <Markdown textAtom={() => liveDoc()} />
                </div>
            </Show>
        </div>
    </Show>
</div>
```

---

## CSS additions (`editor-view.scss`)

```scss
// Preview pane
.editor-body-wrap {
    display: flex;
    flex-direction: column;
    flex: 1 1 0;
    min-height: 0;
}

.editor-codemirror { flex: 1 1 auto; min-height: 0; overflow: hidden; }

.editor-preview-divider {
    flex: 0 0 4px;
    background: var(--border-color);
    cursor: row-resize;
    &:hover { background: var(--accent-color); }
}

.editor-preview-pane {
    flex: 0 0 auto;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border-color);
    min-height: 28px; // header-only when collapsed

    &--collapsed { height: 28px !important; }
}

.editor-preview-header {
    flex: 0 0 28px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 8px;
    background: var(--panel-bg-color);
    border-bottom: 1px solid var(--border-color);
    cursor: pointer;
    user-select: none;
}

.editor-preview-header-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--main-text-color);
    opacity: 0.7;
    text-transform: uppercase;
    letter-spacing: 0.05em;
}

.editor-preview-chevron {
    background: none;
    border: none;
    color: var(--main-text-color);
    opacity: 0.7;
    cursor: pointer;
    padding: 2px 4px;
    font-size: 10px;
    &:hover { opacity: 1; }
}

.editor-preview-content {
    flex: 1 1 auto;
    overflow-y: auto;
    padding: 16px 20px;
}
```

---

## Shortcut reference (updated empty-editor hint)

```ts
{ keys: [MOD, "⇧", "V"], label: "Toggle .md preview" }  // unchanged label, new behaviour
```

The hint text is already correct; no change needed.

---

## Out of scope

- Side-by-side split (preview right of editor) — can be a follow-on. The bottom
  panel pattern ships faster and covers the primary use case (phones / narrow
  laptops where side-by-side is cramped).
- Per-file collapsed state — panel state is per-pane (block meta), not per-tab.
  All tabs in the same editor pane share the same open/height preference.
- Scratch buffer behaviour — the panel renders fine on scratch buffers; no special
  case needed (unlike the old overlay which skipped `isScratch`).
