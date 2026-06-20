# Editor — Markdown rendering bugs (analysis)

**Date:** 2026-06-20  
**Scope:** `view: "editor"` pane (`frontend/app/view/editor/`)  
**Status:** Analysis only — no code changed.

Three reported issues, in priority order (Bug 3 is the real defect; 1 and 2 are
small polish).

---

## Bug 3 — Markdown loads blank on first open; toggling Source→Preview fixes it

**Severity:** High (the headline bug). Every freshly-opened `.md` renders blank
until the user clicks **✎ Source** then **👁 Preview**.

### Where
- Preview render: `editor-view.tsx:838-841`
  ```tsx
  <Show when={showRendered()}>
    <div class="editor-md-preview">
      <Markdown textAtom={() => liveDoc()} />
    </div>
  </Show>
  ```
- The preview binds to the **`liveDoc`** signal, which is seeded *imperatively*,
  not derived from the model's content.
- Seeding sites: `onMount` (`editor-view.tsx:414`), the tab-change
  `createEffect` (`:470` and `:475`), and the CodeMirror update listener (`:398`,
  and the `updateListener` that calls `setLiveDoc` on `docChanged`).

### Root cause
`liveDoc` is only ever set as a side effect of building/restoring CodeMirror or
of a CM edit. There is **no reactive path from "file content finished loading" to
"preview buffer updated."** On first open the timing loses the race:

1. First render: `loadingAtom()` is `true`, so the body `<Show>`
   (`editor-view.tsx:823-832`) renders `<EmptyEditor/>` — the
   `<div ref={containerRef}>` (`:834`) is **not mounted yet**, so `containerRef`
   is `undefined`.
2. `onMount` (`:410-416`) runs with empty `contentAtom()` → `setLiveDoc("")`.
3. RPC returns, content arrives, `loadingAtom()` flips `false`.
4. The tab-change `createEffect` (`:447`) re-runs (it tracks `activeIdAtom` +
   `loadingAtom`). **But it guards on `!containerRef` and returns early
   (`:450`)** when the container isn't mounted. Because the effect was *created*
   before the JSX `<Show>` and runs earlier in Solid's update order, on the
   `loadingAtom` flip it can execute **before** the `<Show>` re-renders and
   assigns `containerRef`. → early return → `liveDoc` never re-seeded with the
   loaded content.
5. `liveDoc` stays `""`. The preview (`showRendered()` is the default for `.md`)
   renders empty.

Clicking **Source** then **Preview** works because by then `containerRef` is
mounted; the next `setupEditor`/restore path calls `setLiveDoc(content)` with the
real text.

The blank-preview symptom is fundamentally that **`liveDoc` is an imperative
mirror of CodeMirror, but the preview consumes it before CodeMirror has been
built with real content.**

### Recommended fix
Make the preview buffer reactive to loaded content, independent of
container/CM timing. Add a dedicated effect that re-seeds `liveDoc` whenever the
model's content (re)loads:

```tsx
// Sync the preview buffer when content finishes loading for the active tab,
// regardless of whether CodeMirror has mounted yet. Fixes the first-open
// blank-preview race. CM edits still drive liveDoc via the update listener;
// contentAtom only changes on load/save, so this never clobbers live edits.
createEffect(() => {
    const c = model.contentAtom();
    if (!model.loadingAtom()) setLiveDoc(c);
});
```

Alternatives considered:
- Gate the preview `<Show>` on `!loadingAtom()` — hides the blank but doesn't fix
  the underlying missing reactivity (still blank if the effect races).
- Have `<Markdown>` read `model.contentAtom()` directly in preview mode — loses
  unsaved source edits when toggling to preview.

The dedicated-effect fix is smallest and correct: contentAtom changes only on
load/save (edits go through CM → updateListener → `setLiveDoc`), so re-seeding on
content change is safe and deterministic.

---

## Bug 1 — Markdown preview has a fixed width; should conform to the window

**Severity:** Low (cosmetic).

### Where
`editor-view.scss:585-589`:
```scss
.editor-md-preview {
    .markdown {
        max-width: 860px;   // ← fixed reading measure
        margin: 0 auto;     // ← centers the narrow column
        font-size: calc(var(--markdown-font-size, 14px) * var(--editor-zoom, 1));
    }
}
```

### Root cause
The rendered markdown is intentionally constrained to an 860px centered reading
column (`.editor-md-preview` container fills the pane via `position:absolute;
inset:0` at `:572-574`, but its `.markdown` child is capped). The user wants it
to fill the pane width instead.

### Recommended fix
Remove the cap (or make it opt-in). Drop `max-width: 860px` and `margin: 0 auto`
so `.markdown` flows to the container width; keep the container's
`padding` (`:583`) for breathing room. If a reading measure is still wanted as a
default, gate it behind a setting or a wider `max-width: 100%`. The
`.editor-md-preview` already scrolls (`overflow:auto`, `:576`) so width-fill is
safe.

Note: this `.markdown` rule is editor-scoped (`.editor-md-preview .markdown`), so
changing it does **not** affect the shared `Markdown` component elsewhere
(`frontend/app/element/markdown.tsx`).

---

## Bug 2 — In-editor file tab should show the full path on hover

**Severity:** Low. **Target confirmed by user: the in-editor file tab strip**
(`editor-tab-strip.tsx`), not the pane header.

### Findings (corrected after deeper investigation)
The tab **already sets a native `title`** with the full path
(`editor-tab-strip.tsx:106`):
```tsx
title={props.tab.isPreview ? `${props.tab.filePath} (preview …)` : props.tab.filePath}
```
And the content is correct: `tab.filePath` is the **full absolute path** —
`canonicalizePath` (`editor-pane-state-store.ts:221-234`) only normalizes
slashes / drive-letter casing / trailing slash; it does not strip the path to a
basename or `~`. So the *data* is right.

**The real problem is the mechanism.** This app does not rely on the native
`title` tooltip for hover affordances — native `title` has a ~1s OS delay and is
inconsistent in the CEF/Chromium embedding. The codebase instead uses **custom
instant tooltips**:
- a pure-CSS `data-tip` pattern (`editor-view.scss:71-94`, used by the file-tree
  toolbar), explicitly *"visible the same frame mouseenter fires"*;
- a shared `Tooltip` component (`@/app/element/tooltip`, used by e.g.
  `action-widgets.tsx`).

So hovering an editor tab feels like "nothing shows" — the native `title` is
there but slow/unreliable, and it isn't the affordance users see elsewhere.

### Recommended fix
Replace the native `title` on `.editor-tab` with the app's tooltip mechanism so
the full path shows instantly and consistently. Two options:

1. **Shared `Tooltip` component (preferred).** Wrap the tab (or its label) in
   `<Tooltip>` from `@/app/element/tooltip`. It uses floating-ui positioning, so
   a long absolute path is placed and constrained sensibly rather than clipping
   at the window edge. Best for long paths.
2. **Scoped `data-tip` CSS** (matches the file-tree pattern). Add
   `data-tip={props.tab.filePath}` and an `.editor-tab[data-tip]:hover::after`
   rule mirroring `editor-view.scss:74-94`. Lighter, but the file-tree rule uses
   `white-space: nowrap`; a long absolute path would overflow, so this variant
   needs `max-width` + wrapping (`white-space: normal; word-break: break-all`).

Recommendation: option 1 (shared `Tooltip`) — consistent with the widget bar,
handles long paths, instant. Keep the preview-tab suffix ("(preview — double-click
to pin)") in the tooltip text. The native `title` can be dropped once the
component tooltip is in place (avoid having both fire).

---

## Suggested sequencing

1. **Bug 3** first — it's the actual defect and self-contained
   (`editor-view.tsx`, one effect).
2. **Bug 1** — one SCSS edit.
3. **Bug 2** — swap the native `title` on `.editor-tab` for the shared
   `Tooltip` component (`editor-tab-strip.tsx:106`). Data is already correct
   (full absolute path); only the tooltip mechanism changes.

All three are low-risk, frontend-only, no backend/schema involvement. Could ship
as one small PR.
