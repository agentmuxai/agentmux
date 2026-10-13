# Media pane: a faint AgentMux logo when no file is open

**Status:** proposed
**Date:** 2026-10-12
**Author:** AgentA (Area54)
**Trigger:** Owner request: "quick fix: similar to the editor, the media pane should have a feigt agentmux logo when no files are open. write spec to file"

## 1. Goal

When a Media pane shows no file, it should look like an empty Editor pane: the
AgentMux brain mark large and faint in the middle, with the pane's own text on
top. Today the Editor has this and the Media pane shows plain grey text on an
empty background.

## 2. What the Editor does (the reference)

`EmptyEditor` in [`frontend/app/view/editor/editor-view.tsx`](../../frontend/app/view/editor/editor-view.tsx) renders
`<div class="editor-empty-logo" innerHTML={brainLogoSvg} aria-hidden="true" />`
(the SVG from `frontend/app/asset/logo-brain.svg?raw`) as the first child of
`.editor-empty`, then the shortcuts and buttons. The styles in
[`frontend/app/view/editor/editor-view.scss`](../../frontend/app/view/editor/editor-view.scss) (`.editor-empty-logo`):

- `position: absolute`, centred with `top/left: 50%` and `translate(-50%, -50%)`
- `width: min(70%, 560px)`, the SVG at `width: 100%; height: auto`
- `opacity: 0.1`, `pointer-events: none`, `z-index: 0`
- the content above it sits at `position: relative; z-index: 1`

## 3. What the Media pane shows today

There are two empty states:

| Case | Where | Shows |
|---|---|---|
| A tab with no file (the usual one: a Media pane always keeps a tab) | `MediaView`, [`frontend/app/view/media/media-view.tsx`](../../frontend/app/view/media/media-view.tsx), the `kind() === "none" && !displayPath()` branch | "Click to load media" (or "No media loaded" where the host can't pick files) and the supported-types explainer, on a full-pane click target that opens the file picker |
| A pane with no tab at all (rare) | `MediaPane` fallback, [`frontend/app/view/media/media-pane.tsx`](../../frontend/app/view/media/media-pane.tsx) (`.media-pane-empty`) | "No files open" and "Click to pick one, or drop a file here." |

Neither has the logo.

## 4. Design

### 4.1 One shared logo element

Add `frontend/app/element/EmptyPaneLogo.tsx` (and `EmptyPaneLogo.scss`): the
Editor's logo, moved, not copied.

```tsx
/** The faint AgentMux mark behind an empty pane's own text (Editor, Media). */
export function EmptyPaneLogo() {
    // eslint-disable-next-line solid/no-innerhtml
    return <div class="empty-pane-logo" innerHTML={brainLogoSvg} aria-hidden="true" />;
}
```

`.empty-pane-logo` takes the `.editor-empty-logo` rules unchanged (§2). Its
parent must be a positioned box (`position: relative` or `absolute`), and the
parent's other children sit above it at `position: relative; z-index: 1`.

### 4.2 The Editor uses it

`EmptyEditor` renders `<EmptyPaneLogo />` in place of its own div;
`.editor-empty-logo` is deleted from `editor-view.scss`. The Editor looks the
same as before.

### 4.3 The Media pane uses it, in both empty states

- **Empty tab** (`MediaView`): `<EmptyPaneLogo />` is the first child of the
  "Click to load media" box. That box is already `position: absolute; inset: 0`,
  so the logo centres on the whole content area. The two text lines get
  `position: relative; z-index: 1`.
- **No tab** (`MediaPane` fallback): the same, inside `.media-pane-empty`, which
  gets `position: relative` if its style lacks it.

Clicking anywhere still opens the file picker (the logo has
`pointer-events: none`), and a file dropped on the pane still opens.

### 4.4 Not shown

- While a file shows, or loads ("Loading…").
- On an error ("Failed to play video…"): that box offers a different file, but a
  file was asked for, so it isn't the empty state.

### 4.5 Look

The same as the Editor in both themes: the SVG's own colours at 10% opacity,
`min(70%, 560px)` wide. Narrow and short panes clip it the way the Editor does
(the parent keeps `overflow: hidden`).

## 5. Out of scope

- The empty-tab logo in [`frontend/app/tab/tabcontent.tsx`](../../frontend/app/tab/tabcontent.tsx) (a small logo above
  user@host and the version). It is a different design and stays.
- Empty states of other panes. Each can use `EmptyPaneLogo` later.
- The empty-state wording.

## 6. Tests

- **[`frontend/app/view/media/media-pane.test.tsx`](../../frontend/app/view/media/media-pane.test.tsx):** a new pane's empty tab
  renders `.empty-pane-logo`, `aria-hidden`; it is gone once a file is shown;
  it is absent on the error state; clicking the logo's area still calls the
  picker.
- **Editor:** the empty Editor still renders the logo (now `.empty-pane-logo`).
  No current test names `.editor-empty-logo`.
- **By eye:** an empty Media pane next to an empty Editor pane, dark and light
  theme, wide and narrow. The two logos should match.

## 7. Files

| File | Change |
|---|---|
| `frontend/app/element/EmptyPaneLogo.tsx`, `EmptyPaneLogo.scss` | New (§4.1) |
| [`frontend/app/view/editor/editor-view.tsx`](../../frontend/app/view/editor/editor-view.tsx), `editor-view.scss` | Use it; drop `.editor-empty-logo` (§4.2) |
| [`frontend/app/view/media/media-view.tsx`](../../frontend/app/view/media/media-view.tsx) | Logo in the empty-tab state (§4.3) |
| [`frontend/app/view/media/media-pane.tsx`](../../frontend/app/view/media/media-pane.tsx) (and its style) | Logo in the no-tab state (§4.3) |
| [`frontend/app/view/media/media-pane.test.tsx`](../../frontend/app/view/media/media-pane.test.tsx) | Tests (§6) |
