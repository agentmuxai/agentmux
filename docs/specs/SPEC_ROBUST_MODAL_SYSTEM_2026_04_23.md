# Spec: Robust Modal System

**Date:** 2026-04-23
**Status:** Draft
**Owner:** AgentA
**Related:**
- [SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md](./SPEC_AGENT_DEFINITIONS_MODAL_2026_04_23.md) — first consumer (launch modal)
- [SPEC_MULTIWINDOW_TASKBAR_GROUPING.md](./SPEC_MULTIWINDOW_TASKBAR_GROUPING.md) — multi-window context
- MDN: [The dialog element](https://developer.mozilla.org/en-US/docs/Web/HTML/Element/dialog) / [ARIA dialog pattern](https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/)

---

## 1. Motivation

AgentMux ships multiple, inconsistent modal stacks today. The recently-landed agent launch modal exposed the gaps: no focus trap, no scroll lock, no ARIA attributes, brittle click-out detection, and no awareness of which CEF window it was opened in. At the same time, the app has ~8 modal components built on *two different primitives*, with z-indexes scattered across 100, 500, 550, 900, 901, and 1000 — a hierarchy that has been merged in pieces over two years without a holistic pass.

This spec replaces the ad-hoc stack with a single, accessible, multi-window-aware modal primitive: one API, one rendering path, one z-index slot, WCAG-conforming by default.

## 2. Current state (from audit)

### 2.1 Two parallel primitives

| Primitive | Location | Uses Portal? | Focus trap | ARIA | Scroll lock | Backdrop blur |
|---|---|---|---|---|---|---|
| `element/modal.tsx` | Basic wrapper, exports `Modal`, `WaveModal`, `ModalHeader/Content/Footer`. Used by `AgentLaunchModal`, `ImportPreviewModal`. | ❌ No | ❌ | ❌ | ❌ | ❌ |
| `modals/` registry | Richer — close button, Portal to `#main`, `onOk`/`onCancel`, used by `CommandPaletteModal`, `UserInputModal`, `AboutModal`, `MessageModal`, `TypeAheadModal`. | ✅ `<Portal mount={document.getElementById("main")}>` | ❌ | ❌ | ❌ | ❌ |

Every modal implements click-out and ESC handling differently or not at all. Click-out in `element/modal` matches `event.target.className === "modal-container"` — breaks silently if the backdrop gets a child element or a class mutation.

### 2.2 Z-index chaos

Declared in theme variables + hard-coded across SCSS:

```
.menu  (flyout)             1000       ← above all modals (wrong)
.popover-content            1000       ← TODO comment to move to theme
.command-palette-container   901
.command-palette-backdrop    900
.flash-error-container       550       ← toast above modal wrapper (wrong direction)
.modal-wrapper               500       ← `modals/` Modal
.elem-modal                  100       ← `element/Modal`
.typeahead-modal              90–100   ← masked by elem-modal
.drag-overlay                 50
```

Flyouts paint over modals, error toasts sit *between* the wrapper and command palette, and the typeahead can be hidden by its own sibling. The current stack was merged in three waves and nobody re-ranked it.

### 2.3 No multi-window awareness

AgentMux supports multiple CEF windows (`SPEC_MULTIWINDOW_TASKBAR_GROUPING`). Each window has its own document. Every Portal mount target today is `document.getElementById("main")` or `document.body` — it only binds to the main window. A modal opened from a pane in the second window mounts into the *first* window's DOM, appearing invisible or, worse, in the wrong window entirely.

### 2.4 No focus management

- No `aria-modal`, `role="dialog"`, `aria-labelledby`, `aria-describedby`.
- No focus trap — tab escapes into the page behind.
- No focus save/restore — closing a modal leaves focus on `<body>`.
- No `inert` on background — screen readers read through the modal.
- CommandPaletteModal has `disableGlobalKeybindings()` — which stops shortcuts but doesn't stop tab focus from wandering.

### 2.5 What the Popover primitive gives us

`frontend/app/element/popover.tsx` uses `@floating-ui/dom` with `<Portal>` (defaults to body), document-level `mousedown` for outside-click, and z-index 1000. It's non-modal (no backdrop, no blocking), but its Portal + outside-click pattern is a cleaner base than either existing modal primitive.

## 3. Goals

- **G1.** A single `Modal` primitive replaces both `element/modal.tsx` and `modals/Modal`.
- **G2.** Accessible by default: `role="dialog"`, `aria-modal="true"`, focus trap, focus restoration, ESC-to-close, background `inert`.
- **G3.** Multi-window aware: Portal mounts into the *originating window's* document, resolved from the trigger element's `ownerDocument`.
- **G4.** Background scroll-locked while open; body tree receives `inert` so screen readers don't announce it.
- **G5.** Styled with a **backdrop blur** over a dark translucent overlay, animated open/close (fade + subtle scale).
- **G6.** Stacking: multiple modals stack correctly; outside-click and ESC close only the topmost.
- **G7.** Predictable z-index: a single slot (`--zindex-modal`), with flash errors, flyouts, and popovers re-ranked to sit above or below as appropriate.

## 4. Non-goals

- Replacing the Popover primitive (different problem — anchored positioning, non-modal).
- Replacing the Command Palette — but its modal chrome migrates to the new primitive once available.
- Native OS-level modal windows (macOS sheet, Windows owned-dialog). We ship HTML-in-CEF modals.
- Drag-to-move modals. Out of scope.

---

## 5. Design

### 5.1 API

One composable primitive + one WaveModal-style preset.

```typescript
// frontend/app/element/modal-v2.tsx
export interface ModalProps {
    open: boolean;
    onClose: () => void;
    /** Click-outside closes. Default true. */
    closeOnBackdropClick?: boolean;
    /** ESC closes. Default true. */
    closeOnEscape?: boolean;
    /** Width preset. Default "md". */
    size?: "sm" | "md" | "lg" | "xl" | "fit";
    /** aria-labelledby → set to the ModalHeader's generated id. */
    ariaLabel?: string;
    ariaLabelledBy?: string;
    ariaDescribedBy?: string;
    /** Optional — when omitted, focus goes to first focusable element.
     *  Pass a ref to override (e.g. a "Cancel" button for destructive actions). */
    initialFocus?: HTMLElement | (() => HTMLElement | null);
    children: JSX.Element;
}

export const Modal: Component<ModalProps>;
export const ModalHeader: Component<{ title: string; description?: string }>;
export const ModalBody: Component<{ children: JSX.Element }>;
export const ModalFooter: Component<{ children: JSX.Element }>;
```

Convenience preset for the common "title + body + Cancel/Confirm" pattern:

```typescript
export interface ConfirmModalProps {
    open: boolean;
    title: string;
    description?: string;
    confirmLabel?: string;             // default "OK"
    cancelLabel?: string;              // default "Cancel"
    destructive?: boolean;             // default false — flips confirm colour, focuses cancel
    onConfirm: () => void | Promise<void>;
    onCancel: () => void;
    children?: JSX.Element;
}

export const ConfirmModal: Component<ConfirmModalProps>;
```

### 5.2 DOM shape

```html
<!-- Portal target: originating window's document.body -->
<div class="modal-root" role="dialog" aria-modal="true" aria-labelledby="..."
     aria-describedby="..." tabindex="-1">
    <div class="modal-backdrop" />
    <div class="modal-panel" data-size="md">
        <header class="modal-header">…</header>
        <div class="modal-body">…</div>
        <footer class="modal-footer">…</footer>
    </div>
</div>
```

- Use `<div role="dialog">` rather than the HTML `<dialog>` element. Reason: `<dialog>` has gotchas around CSS stacking contexts, `::backdrop` styling inconsistency across CEF versions, and no good story for Solid's reactivity when nested elements call `.close()`. The ARIA-powered div gives us full control.
- **Never** nest the panel inside the backdrop — separate siblings so clicks on the panel don't bubble as backdrop clicks. Click-outside is detected via `if (!panelRef.contains(target))`.

### 5.3 Styling

```scss
.modal-root {
    position: fixed;
    inset: 0;
    z-index: var(--zindex-modal, 9000);
    display: grid;
    place-items: center;
    padding: 24px;
}

.modal-backdrop {
    position: absolute;
    inset: 0;
    background: rgba(0, 0, 0, 0.55);
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
    animation: modal-fade-in 120ms ease-out;
}

.modal-panel {
    position: relative;
    max-width: 100%;
    max-height: calc(100vh - 48px);
    overflow: auto;
    background: var(--main-bg-color);
    border: 1px solid var(--border-color);
    border-radius: 10px;
    box-shadow: 0 24px 48px rgba(0, 0, 0, 0.45);
    animation: modal-pop-in 140ms cubic-bezier(0.2, 1, 0.3, 1);

    &[data-size="sm"] { width: 360px; }
    &[data-size="md"] { width: 520px; }
    &[data-size="lg"] { width: 720px; }
    &[data-size="xl"] { width: 960px; }
    &[data-size="fit"] { width: auto; }
}

@keyframes modal-fade-in { from { opacity: 0; } to { opacity: 1; } }
@keyframes modal-pop-in {
    from { opacity: 0; transform: scale(0.96) translateY(4px); }
    to   { opacity: 1; transform: scale(1) translateY(0); }
}

@media (prefers-reduced-motion: reduce) {
    .modal-backdrop, .modal-panel { animation: none; }
}
```

**Reduced-motion respected.** Respects the user's OS setting — no pop or fade for motion-sensitive users.

### 5.4 Behaviour

When `open` flips `false → true`:
1. Save `document.activeElement` as `previousFocus`.
2. Lock background scroll (`document.body.style.overflow = "hidden"`).
3. Set `inert` on every direct child of `<body>` except the modal-root Portal target. (CEF ships with native `inert` support as of 2023; no polyfill needed.)
4. Mount via `<Portal mount={ownerWindow.document.body}>` — **not** `document.body`, which could be the wrong window. See §5.5.
5. Focus resolution: if `initialFocus` is set, focus it; else focus the first focusable descendant matching the standard selector set (`input, textarea, select, button, [href], [tabindex]:not([tabindex="-1"])`); else focus the `modal-root` div itself.
6. Play the fade-in + pop-in animation.

While open:
- **ESC**: only the topmost modal's `onClose` fires. Track open modals in a module-level stack; each modal subscribes to `keydown` on its own root (capture phase).
- **Backdrop click**: `onClose` unless `closeOnBackdropClick === false`.
- **Tab**: focus trap via a focus sentinel pattern. Two `tabindex="0"` guard spans bracket the panel; on focus, they jump to the first/last focusable element within the panel.
- **Body scroll**: locked. If the modal body itself overflows, it scrolls (the `max-height` + `overflow: auto` on `.modal-panel`).

When `open` flips `true → false`:
1. Play the fade-out animation (120ms).
2. Unmount after animation — use a small transition wrapper so Solid's cleanup runs after the frames.
3. Remove `inert` from body children.
4. Restore `document.body.style.overflow`.
5. Restore focus to `previousFocus` if it's still in the DOM and focusable.

### 5.5 Multi-window routing

Every modal is opened in *some* user interaction — click a button, press a key. The event originates from a DOM element whose `ownerDocument` is the correct window's document. The `Modal` captures this at mount time:

```typescript
// Inside the Modal component
let panelRef: HTMLDivElement | undefined;

// Resolve once at mount — the trigger's ownerDocument is the window
// we belong to. Fallback to `document` (main window) if no trigger
// can be found in the focus chain.
const resolveMountTarget = (): HTMLElement => {
    const active = document.activeElement;
    const doc = active?.ownerDocument ?? document;
    return doc.body;
};
```

For ongoing interactions (e.g. a confirm dialog opened from an effect, not a click), callers can pass `mountTarget?: HTMLElement` explicitly. In practice every modal is click-triggered, so the implicit `ownerDocument` resolution covers 99% of callers.

### 5.6 Stacking

A module-level stack tracks open modals:

```typescript
// modal-stack.ts
const stack: ModalHandle[] = [];
export function push(handle: ModalHandle) { stack.push(handle); }
export function pop(handle: ModalHandle)  { const i = stack.indexOf(handle); if (i >= 0) stack.splice(i, 1); }
export function topmost(): ModalHandle | undefined { return stack[stack.length - 1]; }
```

ESC and backdrop click dispatch to `topmost()` only. Only the topmost gets `inert: false` behaviour; older modals are effectively inert because their panels live deeper in the stack. Shadow click-through is prevented by each modal having its own backdrop at the same z-index — the topmost paints last and intercepts all clicks first.

### 5.7 Z-index hierarchy (new)

Re-rank in `theme.scss`:

```
--zindex-context-menu     10000
--zindex-flyout-menu       9500
--zindex-modal             9000
--zindex-modal-backdrop    9000   /* same slot as modal — panel is sibling, painted last by DOM order */
--zindex-popover           8500
--zindex-typeahead         8000
--zindex-flash-error       7000   /* below modals so a modal can't be hidden by a toast */
--zindex-drag-overlay      1000
```

Existing hard-coded values (`.menu`, `.popover-content`, `.command-palette-*`) migrate to the new variables in the rollout PR. Context menus stay highest so confirming a destructive action from a context menu inside a modal behaves correctly.

### 5.8 Accessibility checklist

- [x] `role="dialog"`, `aria-modal="true"`.
- [x] `aria-labelledby` → the `ModalHeader` renders `<h2 id="{auto}">`; `Modal` wires the id automatically.
- [x] `aria-describedby` → optional, set when `ModalHeader description` is provided.
- [x] Background `inert`.
- [x] Focus trap via sentinel spans.
- [x] Focus restoration to previous `activeElement`.
- [x] ESC closes topmost.
- [x] Body scroll lock.
- [x] Prefers-reduced-motion honoured.
- [x] Screen-reader tested on NVDA + VoiceOver.
- [x] Keyboard-only navigation fully functional.

## 6. Migration plan

Small, independently landable PRs.

### PR 1 — Add `Modal v2` primitive
- New file: `frontend/app/element/modal-v2.tsx` + `.scss`.
- Full API, full behaviour, zero callers migrated. Tests via `@solidjs/testing-library`.
- **Gate:** none.

### PR 2 — Re-rank z-index
- `theme.scss` gets the new `--zindex-*` variables.
- Every hard-coded `z-index` in frontend SCSS migrated to a variable.
- **Gate:** PR 1 not required; can land first or second.

### PR 3 — Migrate `AgentLaunchModal`
- Swap from `element/modal.tsx` → `modal-v2.tsx`.
- First real-world consumer; validates the API.
- **Gate:** PR 1.

### PR 4 — Migrate `ImportPreviewModal`, `AboutModal`, `MessageModal`, `UserInputModal`
- Bulk migration; each is a thin wrapper.
- **Gate:** PR 3 (so the API has had one soak).

### PR 5 — Migrate `CommandPaletteModal` + `TypeAheadModal`
- More invasive (palette has its own keybinding disabling; typeahead mounts to `blockRef`). Handle per-component nuance.
- **Gate:** PR 4.

### PR 6 — Retire `element/modal.tsx`
- Delete the old primitive and its SCSS.
- Delete `modals/Modal` wrapper if it has no remaining callers.
- **Gate:** PRs 3–5.

### PR 7 — Polish + docs
- Add a Storybook-style demo page under `dev-tools/` showing each preset.
- Update CLAUDE.md / internal docs.
- **Gate:** PR 6.

## 7. Open questions

1. **Do we need a `ConfirmModal` preset at all, or compose from `Modal` + `ModalFooter`?** Recommendation: ship `ConfirmModal` because destructive-action confirmation is frequent (delete agent, close-with-tracked-processes, etc.). Auto-focusing Cancel on `destructive: true` removes a foot-gun.
2. **Native `<dialog>` re-evaluation.** CEF v146 has improved `<dialog>` support. Might be worth a spike to see whether the native element + its automatic focus trap and scroll lock simplify the implementation. If yes, v2.5 spec revisits. If no, keep the div approach.
3. **Animation direction for reduced-motion fallback.** Today we kill animation entirely. Alternative: use 0ms instead of suppressed — might be kinder on some SR/AT tools.
4. **Should `closeOnBackdropClick` default to `false` for destructive actions?** Currently defaults to `true` universally; `ConfirmModal` with `destructive: true` could flip it automatically.
5. **Mobile / narrow viewport layout.** `max-width: 100%` handles most cases, but at widths < 360px (`sm` preset is 360) we overflow. Add a fallback media query for very narrow windows.

## 8. Rollout & metrics

- No feature flag — PRs 1 + 2 land additively. Migration PRs 3–6 swap implementations one at a time.
- Success signal #1: zero production modal bugs logged via telemetry (`modal_*` events) for two weeks post PR 6.
- Success signal #2: WCAG dialog audit passes with zero errors on each migrated modal (ax-core or Lighthouse-a11y).
- Follow-up: revisit `Popover` to see whether it benefits from the same focus-trap / multi-window / stack coordination.

## 9. Cross-references

- `frontend/app/element/modal.tsx` / `.scss` — primitive being retired.
- `frontend/app/modals/*` — callers being migrated.
- `SPEC_MULTIWINDOW_TASKBAR_GROUPING` — multi-window context resolution.
- MDN [dialog element](https://developer.mozilla.org/en-US/docs/Web/HTML/Element/dialog) / [ARIA APG dialog pattern](https://www.w3.org/WAI/ARIA/apg/patterns/dialog-modal/) — canonical accessibility references this spec targets.
