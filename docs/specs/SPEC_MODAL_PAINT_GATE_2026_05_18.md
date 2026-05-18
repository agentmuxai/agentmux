# SPEC: Modal Paint Gate

**Status:** Draft
**Date:** 2026-05-18
**Author:** AgentA
**Related:**
- [`SPEC_MODAL_TRANSITIONS_2026_05_18.md`](./SPEC_MODAL_TRANSITIONS_2026_05_18.md) — entrance / replace animations (this spec composes on top)
- [`SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`](./SPEC_AGENT_INSTALL_STAGE_2026_05_17.md) — the modal that surfaced the issue

---

## 0. TL;DR

When a modal mounts, its entrance animation (fade-in + pop-in) fires *in parallel* with the component's `onMount` work — xterm.js constructor, `terminal.open(ref)`, `fitAddon.fit()`, ResizeObserver settling. For the install modal in particular, the user sees a half-mounted terminal mid-animation: a 0×0 box that resolves to its real size after the animation completes, sometimes with a visible reflow.

Fix: introduce a **paint gate** in `TabModalLayer`. The modal renders with `opacity: 0` + `pointer-events: none` first, lets `onMount` run, waits two animation frames (one for layout, one for paint), then adds a `data-ready` attribute that flips opacity back on and triggers the entrance keyframes. No additional library; no SolidJS `<Suspense>` needed. Reduced-motion still respected.

> **Why `opacity: 0` and not `visibility: hidden`?** The initial draft called for `visibility: hidden`, but browsers skip the `autofocus` HTML attribute on elements inside a `visibility: hidden` subtree, regressing the launch modal's name-input focus (codex P2 on PR #900). `opacity: 0` keeps layout flowing AND allows focus acquisition.

---

## 1. Problem

### 1.1 What the user reports (2026-05-18)

> "when modals load, make sure you only load the content once everything is painted .. is that already set, or is it missing?"

Observed: install modal entrance animates while the xterm container hasn't sized yet. The "Click Install now to begin" placeholder line either lands too late or sits on a wrong-sized buffer. Less visible but present in the launch modal too — form fields fade in while autofocus/layout settle.

### 1.2 Current sequence

For any TabModal request:

```
t=0    setCurrent(req)
t=0+   SolidJS reactivity flushes → <Show> mounts the overlay subtree
t=0+   Browser inserts DOM nodes
t=0+   CSS keyframes start immediately:
         .tab-modal-backdrop  → tab-modal-fade-in (120ms)
         .tab-modal-panel     → tab-modal-pop-in  (140ms)
         .tab-modal-content   → tab-modal-content-in (140ms)
t=0+   SolidJS fires onMount handlers (also synchronous within the same tick)
t=0+   For install modal: new Terminal(...), terminal.open(ref), tryFit()
       — but the container is still being sized; first fit returns 0×0 or fails silently
t≈1ms  Browser computes layout, runs first paint with whatever was in the DOM
t≈4ms  Next layout cycle; container reaches its target size
t≈4ms  ResizeObserver fires; FitAddon retries; xterm finally sized
t=140ms Entrance animation completes
```

During steps `t=0+` through `t≈4ms`, the user is watching an animation play over content that's still resolving its layout. The eye perceives this as flicker / shift.

### 1.3 Why this isn't a one-off

Every modal that owns "non-trivial mount work" hits this:
- Install modal — xterm + FitAddon + ResizeObserver
- Launch modal — form autofocus, identity/memory dropdown population, OAuth state probe
- Future modal kinds (workflow setup, agent settings) will be worse, not better

A spec-level fix at the layer is preferable to per-modal workarounds.

---

## 2. Best practices research

### 2.1 The "FOUC pattern" (Flash Of Unstyled Content)

Standard web technique for hiding under-construction content:

```css
body { visibility: hidden; }
body.loaded { visibility: visible; }
```

`visibility: hidden` keeps the element in the layout flow (so width/height resolve normally and `getBoundingClientRect` returns the real rect) but doesn't paint. Once `loaded` is added, the element flips to visible without disrupting layout — and entrance animations can compose on top.

This is the right primitive for the modal-paint gate. xterm.js's FitAddon needs a non-zero rect to size correctly; `visibility: hidden` provides that.

### 2.2 The `rAF × 2` pattern

A single `requestAnimationFrame` after `onMount` runs before the first paint commits. Two rAFs runs *after* the first paint — guarantees the browser has actually drawn the hidden frame before we flip visibility:

```ts
onMount(() => {
    /* do work */
    requestAnimationFrame(() => {
        requestAnimationFrame(() => {
            setReady(true);   // flips visibility + starts entrance
        });
    });
});
```

Two-rAF is canonical in React/Vue/Solid communities. Lighthouse and Web Vitals tooling use the same pattern when measuring "interactive after paint."

### 2.3 What we should NOT do

- **`setTimeout(..., 0)`** — undefined timing relative to the paint cycle; introduces a perceptible delay on slow machines.
- **`Promise.resolve().then(...)`** — microtask, fires before paint.
- **Custom busy-wait until ResizeObserver reports non-zero** — works but couples the layer to a specific child's signal. The two-rAF pattern is content-agnostic.
- **SolidJS `<Suspense>`** — designed for async-resource loading, not for "wait until I've painted." Wrong fit.

---

## 3. Proposed architecture

### 3.1 The gate

In `TabModalLayer.tsx`, the outer `<Show when={current()}>` already mounts/unmounts the modal subtree. Add a `ready` signal that:

1. Starts `false` whenever a new `current` is set.
2. Flips to `true` after two `requestAnimationFrame` ticks following the mount.

```ts
const [ready, setReady] = createSignal(false);

createEffect(() => {
    // Re-arm on every replace; falls back to false on close.
    if (current() == null) { setReady(false); return; }
    setReady(false);
    requestAnimationFrame(() => {
        requestAnimationFrame(() => setReady(true));
    });
});
```

The render adds `data-ready={ready() ? "" : undefined}` to the overlay root:

```tsx
<div class="tab-modal-overlay" data-ready={ready() ? "" : undefined}>
    {/* backdrop + panel + content unchanged */}
</div>
```

### 3.2 The CSS gate

`tab-modal.scss` adds two rules:

```scss
.tab-modal-overlay:not([data-ready]) {
    .tab-modal-backdrop,
    .tab-modal-panel {
        visibility: hidden;
        animation: none;     // suppress entrance keyframes until ready
    }
}

.tab-modal-overlay[data-ready] {
    .tab-modal-backdrop { animation: tab-modal-fade-in 120ms ease-out; }
    .tab-modal-panel    { animation: tab-modal-pop-in  140ms cubic-bezier(0.2, 1, 0.3, 1); }
}
```

The content keyframe (`tab-modal-content-in`) is left on the inner `.tab-modal-content` and gated the same way:

```scss
.tab-modal-overlay:not([data-ready]) .tab-modal-content {
    visibility: hidden;
    animation: none;
}
.tab-modal-overlay[data-ready] .tab-modal-content {
    animation: tab-modal-content-in 140ms cubic-bezier(0.2, 1, 0.3, 1);
}
```

Why this works:
- `visibility: hidden` keeps layout flowing — xterm's container resolves its real rect during the hidden frame, FitAddon's first `fit()` succeeds.
- `animation: none` until `data-ready` means keyframes only run once content is settled.
- Outside-the-modal layout (TileLayout, tab bar) is unchanged across the gate.

### 3.3 Interaction with `tabModal.replace()`

The replace primitive (from `SPEC_MODAL_TRANSITIONS_2026_05_18.md`) keeps backdrop + outer panel mounted across a swap. Only `.tab-modal-content` remounts via the keyed inner `<Show>`. The paint gate must still apply to *content remounts*:

```ts
// Re-arm the inner gate on every replace (same effect, different scope).
createEffect(() => {
    current();  // track
    setContentReady(false);
    requestAnimationFrame(() => {
        requestAnimationFrame(() => setContentReady(true));
    });
});
```

And:

```scss
.tab-modal-overlay:not([data-content-ready]) .tab-modal-content {
    visibility: hidden;
    animation: none;
}
```

This adds a second attribute, but the cost is two booleans + four CSS rules. The benefit is that *both* the cold-open path (full backdrop + panel + content gate) and the replace path (content-only gate) avoid the half-painted moment.

### 3.4 Reduced motion

When `prefers-reduced-motion: reduce` is set, the entrance animations are already suppressed (per `SPEC_MODAL_TRANSITIONS_2026_05_18.md` §5). The paint gate still applies — hidden during mount, visible after rAF×2 — but no fade/pop happens. End result: modal appears instantly *after* paint, exactly as the user expects in reduced-motion mode.

### 3.5 Failsafe: max wait

In the unlikely case rAF never fires (background tab, suspended renderer), the gate would leave the modal hidden indefinitely. Add a `setTimeout` failsafe at 200ms that forces `ready = true` regardless:

```ts
const failsafe = setTimeout(() => setReady(true), 200);
requestAnimationFrame(() => {
    requestAnimationFrame(() => {
        clearTimeout(failsafe);
        setReady(true);
    });
});
```

200ms is well above one paint cycle on any machine that can run AgentMux. If we hit the failsafe, the user sees the entrance animation a few ms late — better than nothing.

---

## 4. Why not just "wait for ResizeObserver to fire"?

Tempting for the install modal specifically — wait until the xterm container reports a non-zero rect, then animate. But:

1. Couples the layer to the install modal's specific dependency (ResizeObserver). Other modal kinds (launch, future settings panels) have different "I'm settled" signals.
2. Requires plumbing a "this child is ready" callback up through `renderRequest`. Adds API surface for one use case.
3. Doesn't help non-xterm modals.

The rAF×2 pattern is content-agnostic: it just guarantees one full paint cycle has happened, which is enough for FitAddon, autofocus, dropdown population, and any other synchronous-after-mount work to settle.

---

## 5. Implementation

### 5.1 Edits

1. `frontend/app/tab/TabModalLayer.tsx` — two new signals (`ready`, `contentReady`), two `createEffect`s wiring the rAF×2 + failsafe, `data-ready` and `data-content-ready` attrs on the overlay.
2. `frontend/app/tab/tab-modal.scss` — visibility gate rules + animation gate; reduced-motion section unchanged (still suppresses keyframes).
3. `docs/specs/SPEC_MODAL_PAINT_GATE_2026_05_18.md` — this file.
4. Changeset: `patch — fix(modal): paint-gate so entrance animations run after content settles`.

### 5.2 Test plan

- [ ] Open install modal cold from agent picker. Backdrop appears already-fully-rendered (no half-painted xterm visible mid-animation).
- [ ] Install completes → click Continue to Launch. Inner content crossfades to launch modal with no half-painted form fields.
- [ ] System `prefers-reduced-motion: reduce` → modal appears instantly with no animation, still gated on paint.
- [ ] DevTools throttling at "4× slowdown" — entrance still completes within ~200ms (failsafe doesn't trigger under normal slow conditions).
- [ ] Backgrounded tab (modal opened, user switches tab) → no stuck-hidden state. Failsafe fires within 200ms even if rAF stays parked.

### 5.3 Risk

- **Two-rAF adds ~16-32ms of perceived latency before entrance begins.** Imperceptible to humans for modal opens, well below the 100ms "instant" threshold.
- **Failsafe could fire early on a slow machine.** 200ms is safe; the rAF×2 should fire in <33ms on any machine running AgentMux. If we see complaints, lower to 100ms or remove (rAF is reliable in CEF/Chromium).

---

## 6. Acceptance criteria

1. `data-ready` only flips to true after one full paint cycle from mount.
2. `visibility: hidden` keeps layout flowing (FitAddon's first `fit()` succeeds).
3. Entrance keyframes don't run on the hidden frame.
4. `tabModal.replace()` content swaps also gate on paint (no half-mounted post-replace content).
5. Reduced-motion mode still respects the gate but skips the animation.
6. No regression in the `tabModal.close()` path — close still tears down instantly.

---

## 7. Out of scope

- **Exit animations** (still deferred per `SPEC_MODAL_TRANSITIONS_2026_05_18.md` §3.6).
- **`<Suspense>`-based async content loading** — separate problem; this spec is about layout/paint settling for synchronously-mounted content.
- **Backend-driven content readiness** (modal wants to open but RPC hasn't returned) — caller's responsibility; resolve the data before calling `tabModal.open`.
- **Hardware-pane (`browser` view) clipping** during the hidden frame — already handled by the existing `PaneOverlayClip` ResizeObserver; the visibility flip doesn't change the overlay's bounding rect.
