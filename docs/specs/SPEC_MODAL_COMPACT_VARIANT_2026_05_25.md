# SPEC: Compact modal variant — auto-trigger for narrow lock regions

**Date:** 2026-05-25
**Author:** AgentA (Claude Opus 4.7)
**Builds on:** [`SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md`](./SPEC_UNIFIED_MODAL_SYSTEM_2026_05_21.md), [`SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md`](./SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md) (#1034, #1038)

---

## TL;DR

`<Modal scope="pane">` can mount in arbitrarily narrow panes — verified live during the browser-auth diagnosis: a focused browser pane sat at **240×1020** in a 630-wide window with three sibling panes. Every modal panel in the canonical chrome assumes ≥320px (`ConfirmModal`), ≥560px (`AgentInstallModal`), ≥430px (`AgentLaunchModal`). At 240px the panel either overflows the pane (covering neighbors despite the pane-scope lock) or relies on the unified `<Modal>`'s `size="fit"` to clamp, which produces visually broken layouts (button row truncated, title cropped, body horizontally scrolling).

A **compact variant** of the modal chrome — auto-triggered when the layer's mount node is narrower than a threshold — fixes this. No per-call-site flag needed; the modal layer detects the constraint and toggles a class the chrome SCSS responds to.

---

## 1. Trigger

`ResizeObserver` on the layer's `.modal-layer-mount` node toggles a class:

```
.modal-layer-mount.modal-layer-mount--compact
```

Threshold: **`width < 400px`**. Chosen because:

- 240px is the verified-narrow case (browser pane in three-pane layout).
- 400px gives ~30% headroom for the user dragging the divider — avoids flapping at the threshold.
- Above 400px every existing modal panel renders cleanly without horizontal scroll.

The class flips synchronously inside the ResizeObserver callback. No debouncing — `:where(...)` rules in CSS apply instantly and the modal panel reflows in the same frame.

Single threshold, not a stack (no `--ultra-compact` for 200px etc.). Adding granularity later is cheaper than collapsing it.

## 2. Visual contract

When `.modal-layer-mount--compact` is set:

| Slot | Standard | Compact |
|---|---|---|
| `.modal-panel` | `min-width: <per-panel>` (320-560px) | `min-width: 0`, `width: 100%`, `max-width: 100%` |
| `.modal-panel-header` | `padding: var(--space-3)` | `padding: var(--space-1) var(--space-2)` |
| `.modal-panel-title` | `font-size: 18px` | `font-size: 14px`, `line-height: 1.3` |
| `.modal-panel-description` | `font-size: 13px` | `font-size: 12px`, `margin-top: 2px` |
| `.modal-panel-body` | `padding: var(--space-3)` | `padding: var(--space-2)` |
| `.modal-panel-footer` | `padding: var(--space-2) var(--space-3)`, `flex-direction: row` (right-aligned) | `padding: var(--space-1) var(--space-2)`, `flex-direction: column-reverse`, button `width: 100%` |
| Buttons | natural width | `width: 100%`, smaller font |
| Backdrop | unchanged | unchanged (still covers lock region) |
| Animation keyframes | unchanged | unchanged |

The `column-reverse` footer direction keeps the primary action (typically `submit` / `green-solid`) at the BOTTOM, matching mobile-app convention where the thumb-reachable action is bottom. Cancel sits above. Reagent + UX precedent in [`SPEC_MODAL_TRANSITIONS_2026_05_18`](./SPEC_MODAL_TRANSITIONS_2026_05_18.md) doesn't apply (it's about cold→hot transitions, not layout).

## 3. Per-panel opt-in (the rare case)

The compact rules live in `frontend/app/element/modal.scss` keyed on the universal chrome classes (`.modal-panel-*`). All consumers that use the canonical chrome get the variant for free — no per-panel JS needed.

A modal panel that has body content with hard min-widths (e.g., the install modal's xterm at 560×240) must declare its own override. Pattern:

```scss
.agent-install-modal-body {
    min-width: 560px;
    /* ... */

    // Compact: relax the xterm constraint. xterm's FitAddon will
    // recompute cols + rows from the smaller container.
    :where(.modal-layer-mount--compact) & {
        min-width: 0;
        min-height: 200px;
    }
}
```

The `:where()` wrapper keeps specificity at 0 so per-panel rules can still override without `!important`.

## 4. Implementation

### 4.1 `ModalLayer.tsx`

Inside the existing layer, instrument the mount node with a `ResizeObserver`:

```tsx
const [isCompact, setIsCompact] = createSignal(false);

const setMountRef = (el: HTMLElement | null) => {
    setMountEl(el);
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
        for (const entry of entries) {
            const w = entry.contentRect.width;
            const compact = w > 0 && w < COMPACT_THRESHOLD_PX;
            if (compact !== isCompact()) setIsCompact(compact);
        }
    });
    ro.observe(el);
    onCleanup(() => ro.disconnect());
};

// ...

<div
    class={`modal-layer-mount${isCompact() ? " modal-layer-mount--compact" : ""}`}
    style="display:contents"
    ref={setMountRef}
>
```

The `display:contents` mount node has no layout impact, but `ResizeObserver` still observes its content rect — the rect equals the union of its children's layout boxes. In practice the mount wraps the pane root, so `width` is the pane width.

`COMPACT_THRESHOLD_PX = 400` — a top-of-file `const`, not a SCSS variable, because the trigger lives in JS not CSS.

### 4.2 `modal.scss`

Append compact-variant rules to the existing canonical chrome:

```scss
:where(.modal-layer-mount--compact) {
    .modal-panel { min-width: 0; width: 100%; max-width: 100%; }
    .modal-panel-header { padding: var(--space-1) var(--space-2); }
    .modal-panel-title { font-size: 14px; line-height: 1.3; }
    .modal-panel-description { font-size: 12px; margin-top: 2px; }
    .modal-panel-body { padding: var(--space-2); }
    .modal-panel-footer {
        padding: var(--space-1) var(--space-2);
        flex-direction: column-reverse;
        gap: var(--space-1);

        .button, button { width: 100%; }
    }
}
```

The `:where()` wrapper keeps the compact rules at the same specificity as the base rules, so per-panel SCSS overrides work cleanly.

### 4.3 No JS API change

`<Modal>` doesn't get a new prop. The compact variant is a CSS-level response to a JS-detected geometry. Call sites are unaware.

## 5. Edge cases

| Scenario | Behavior |
|---|---|
| Pane resized from 500px → 350px while modal is open | Mount node ResizeObserver fires, class toggles, CSS reflows mid-flight. No JS state change in the panel. |
| Pane resized 380px → 420px → 380px (flap near threshold) | No hysteresis built in for v1. The `< 400` check fires both directions. Acceptable because the visual delta is small. If flap becomes annoying in practice, add a 20px hysteresis band (`<400` to enter, `>420` to exit). |
| Mount node not yet attached to DOM (initial mount) | `ResizeObserver` only fires once observed; before that, `isCompact()` is `false` → standard variant. First measurement lands ~one frame after mount. Acceptable. |
| Multi-window (two CEF windows) | Each window has its own ResizeObserver and class state. No cross-window leak. |
| Compact mount that hosts a window-scope modal | Window-scope modals don't mount into a pane root; they Portal to `document.body` which has no `--compact` class. So window modals never go compact, even when the surrounding pane is narrow. Correct — window modals own the whole window. |

## 6. Test plan

### 6.1 Manual

- Browser pane at 240px → open auth modal → verify panel fits, footer buttons stack, title legible.
- Browser pane at 600px → open auth modal → verify standard layout (no compact class).
- Drag divider from 500 → 350 while modal open → verify smooth reflow (no jitter).
- Agent pane at narrow width → open launch modal → verify identity / memory dropdowns + the OAuth panel all fit (or fail predictably with horizontal scroll INSIDE the body, never the whole panel).

### 6.2 Component test

`frontend/app/element/ModalLayer.test.tsx` (new file) renders `<ModalLayer scope="pane">` with a wrapping div of variable width, asserts `.modal-layer-mount--compact` toggles correctly. Use `ResizeObserver` mock; advance dimensions; check class.

## 7. Out of scope

- Granular thresholds (ultra-compact at 200px, comfortable at 600px). Add later if user feedback warrants.
- Mobile / touch optimization. AgentMux is desktop-only.
- Reduced-motion + compact intersection. Already handled — compact only changes layout, not animations; existing `prefers-reduced-motion` rules continue to suppress entrance/exit keyframes.
- Per-panel custom thresholds. Hardcoded global 400px is sufficient for current use cases.

## 8. Delivery

Single PR:
- `frontend/app/element/ModalLayer.tsx` — add ResizeObserver + class toggle.
- `frontend/app/element/modal.scss` — append `:where(.modal-layer-mount--compact)` ruleset.
- `frontend/app/view/agent/components/AgentInstallModal.scss` — relax `.agent-install-modal-body` `min-width` in compact mode (xterm-specific).
- `frontend/app/element/ModalLayer.test.tsx` — new test.
- Spec ships with the PR (`feedback_no_doc_only_prs`).

Reagent + codex review. Manual smoke on the verified 240px browser pane scenario.
