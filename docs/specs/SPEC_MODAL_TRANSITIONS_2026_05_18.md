# SPEC: Modal Transitions & Chained-Flow Crossfades

**Status:** Draft
**Date:** 2026-05-18
**Author:** AgentA
**Related:**
- [`SPEC_ROBUST_MODAL_SYSTEM_2026_04_23.md`](./SPEC_ROBUST_MODAL_SYSTEM_2026_04_23.md) — Modal v2 (window-scoped)
- [`launch-modal-rearchitecture-2026-05-01.md`](./launch-modal-rearchitecture-2026-05-01.md) — TabModalLayer rationale
- [`SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`](./SPEC_AGENT_INSTALL_STAGE_2026_05_17.md) §6/§11 — the chained flow that surfaces this gap

---

## 0. TL;DR

When the install modal finishes and the user clicks **Continue to Launch**, the install panel unmounts instantly, the backdrop disappears, and the launch panel mounts and re-plays its entrance animation. The visible result is a flicker of the underlying tab + a "modal pop" that reads as two separate events even though the user is in one continuous flow.

Root cause: AgentMux's modal infrastructure has **entrance animations but no exit animations**, and no notion of "this next modal is a continuation of the current one." Every `tabModal.open(...)` is treated as a fresh appearance.

Fix: add a `tabModal.replace(next)` operation that keeps the backdrop mounted and crossfades the panel content. Make the install→launch handoff use it. Add proper exit animations as the foundation so this primitive can later be reused for any chained-modal flow (auth → launch, install → auth → launch, etc.).

---

## 1. Problem

### 1.1 Observed (2026-05-18)

Install completes. User clicks **Continue to Launch**. The screen:

1. Loses the install panel + backdrop instantly (one frame).
2. Shows the bare tab content (cards, action bar) for 1–10ms.
3. Replays the entrance animation: backdrop fades in 120ms, panel pops in 140ms.

The user perceives this as a jolt. Two discrete events were rendered when the user did one thing.

### 1.2 Current infrastructure (per audit 2026-05-18)

`TabModalLayer.tsx` is a single-slot host using SolidJS `<Show when={current()}>`. `tabModal.open(req)` replaces the current request; `tabModal.close()` nulls it. Both transitions are instant:

```ts
// TabModalLayer.tsx — today
open: (req) => { setSubmitting(false); setCurrent(req); }
close: () => setCurrent(null)
```

Mount triggers CSS keyframes (`tab-modal-fade-in` 120ms + `tab-modal-pop-in` 140ms). Unmount has no symmetrical exit — the DOM nodes vanish.

The install→launch handoff in `TabModalLayer.tsx` lines 166–171:

```ts
api.close();          // unmount install panel + backdrop NOW
req.onInstalled();    // → AgentPicker.openLaunchModal() → tabModal.open(launch)
```

Same event-loop tick. The render pipeline tears down the install layer, processes the new open, then mounts the launch layer. There's no coordination between them.

### 1.3 Why this isn't a one-off

Three more chained flows are already on the roadmap or implemented:
- Install → Auth (OpenClaw OAuth happens after install)
- Auth → Launch (OAuth completes, launch begins)
- Workflow setup → Workflow run

Patching the install→launch case alone would mean re-solving the same problem three more times. A reusable primitive is the right move.

---

## 2. Best practices research

### 2.1 Material Design — "Container transform" / "Shared axis"

When two screens are part of the same task, the chrome (container) should persist across the transition; only the contents crossfade or slide. The hallmark is **continuity of the container**: same shape, same rough position, content swaps inside.

Applied here: the modal panel itself stays mounted across install→launch. Only its body content swaps.

### 2.2 Apple Human Interface — "Sheet → sheet"

iOS/macOS handle sheet replacement by holding the surface in place while the content view re-renders with a brief crossfade (~200ms). The user reads it as "the same panel said something new" rather than "panel A closed and panel B opened."

### 2.3 CSS View Transitions API

Browser-native crossfade for arbitrary DOM swaps. Wrap a DOM mutation in `document.startViewTransition(() => { ... })` and the browser captures before/after snapshots, animates between them, and resolves. Available in Chromium 111+ (CEF includes it). Falls back to instant swap on older runtimes.

Pros: zero hand-rolled animation code; automatic crossfade; respects `prefers-reduced-motion`.
Cons: same-document only; requires modern Chromium; less explicit control over timing.

### 2.4 Anti-patterns to avoid

- **Delaying close to let exit animation finish, then opening next.** Adds latency without fixing the gap (backdrop still disappears mid-transition).
- **Animating opacity on the outer overlay during the swap.** Creates a double-fade that reads as flicker.
- **Reusing the same `<Modal>` instance with cleverly memoized content.** Couples consumer code to the host. Doesn't scale.

---

## 3. Proposed architecture

### 3.1 Goals

1. Visually continuous transition between two modals that belong to one flow.
2. Reusable primitive — not bespoke to install→launch.
3. Backwards-compatible: existing `open` / `close` callers see no behavior change.
4. Honors `prefers-reduced-motion` (instant swap when reduced).
5. No new dependencies (no `solid-transition-group`).

### 3.2 The primitive: `tabModal.replace(next)`

```ts
interface TabModalApi {
    open(req: TabModalRequest): void;
    close(): void;
    /**
     * Replace the current modal with `next` as a continuation of the
     * same flow. The backdrop stays mounted across the swap; the panel
     * content crossfades. No-op if there is no current modal — falls
     * back to `open(next)`.
     */
    replace(next: TabModalRequest): void;
}
```

`replace` is the only new surface area. Everything else stays.

### 3.3 Implementation (CSS-only crossfade)

`TabModalLayer.tsx` adds a transient `replacing` flag and a `swap-key` derived from the request identity. The panel content is keyed on this so React/Solid re-renders the inner subtree with a CSS class that triggers fade-out → fade-in.

```tsx
// Pseudocode
const [current, setCurrent] = createSignal<TabModalRequest | null>(null);
const [swapKey, setSwapKey] = createSignal(0);

const replace = (next: TabModalRequest) => {
    if (current() == null) { open(next); return; }
    setSubmitting(false);
    setCurrent(next);
    setSwapKey(k => k + 1);    // forces inner swap CSS to retrigger
};

return (
    <Show when={current()}>
        <div class="tab-modal-backdrop" />
        <div class="tab-modal-panel">
            <div class="tab-modal-content" data-swap-key={swapKey()}>
                {/* renderRequest(current()) */}
            </div>
        </div>
    </Show>
);
```

CSS (in `tab-modal.scss`):

```scss
.tab-modal-content {
    animation: tab-modal-content-in 140ms cubic-bezier(0.2, 1, 0.3, 1);
}

@keyframes tab-modal-content-in {
    from { opacity: 0; transform: translateY(2px); }
    to   { opacity: 1; transform: translateY(0); }
}

@media (prefers-reduced-motion: reduce) {
    .tab-modal-content { animation: none; }
}
```

The `[data-swap-key]` attribute change forces SolidJS to re-key the inner subtree, restarting the keyframe. The backdrop + outer panel never unmount, so there's no backdrop flicker or pop-in replay.

### 3.4 Optional enhancement: View Transitions API

Behind a feature check:

```ts
const replace = (next: TabModalRequest) => {
    if (current() == null) { open(next); return; }
    const swap = () => { setSubmitting(false); setCurrent(next); };
    if (typeof (document as any).startViewTransition === "function") {
        (document as any).startViewTransition(swap);
    } else {
        swap();
        // CSS keyframe (3.3) handles fallback animation.
    }
};
```

Phase 2. Not required for the initial fix.

### 3.5 Panel size changes (not animated in v1)

If install modal and launch modal have different heights, the panel resizes during the swap. CSS `transition: min-height` on `.tab-modal-panel` was tried but doesn't work — the panel sizes itself from intrinsic content (children supply `min-height`, panel itself is `auto`), and `auto` is not animatable. Reagent caught this on PR #896.

The content-fade crossfade already removes the user-reported jolt, so v1 ships with an un-animated size snap. A FLIP-measured height animation can be added later if the snap becomes user-visible — the work would live inside the keyed `<Show>` callback, measuring the old content's `getBoundingClientRect().height` before unmount and animating the new content's height from old → new.

### 3.6 Exit animations (deferred)

True exit animations (panel slides out, backdrop fades out) on `close()` are a separate problem. This spec doesn't add them — adding `replace()` solves the visible regression. A follow-up spec can introduce `data-state="closing"` + `animationend` listeners for the close path if a future flow needs it.

Rationale: the only place the user currently sees the gap is during chained flows. Plain `close()` (user dismisses with Cancel or X) is fine instant — instant feedback is what they want for cancel.

---

## 4. Call-site changes

### 4.1 TabModalLayer

In `renderRequest()`, replace the `api.close(); req.onInstalled()` pair with a single signal back to AgentPicker indicating "next request should use replace, not open." Cleanest: extend the request shape so `onInstalled` returns the next request descriptor, and `TabModalLayer` performs the `replace` internally.

```ts
// install-agent request
onInstalled: () => TabModalRequest | null
```

If non-null, `TabModalLayer.replace(returned)`. If null, `close()`. Backward-compat: callers that currently return `void` continue to work as close().

Alternative: keep `onInstalled` void and have AgentPicker call `tabModal.replace(launchReq)` directly. Slightly less encapsulated but doesn't change the request shape. **Recommend this alternative** — fewer churned types, the primitive is explicit at the call site.

### 4.2 AgentPicker

```ts
// before
onInstalled: () => {
    // ... mark all sibling cards installed ...
    openLaunchModal(agent);       // tabModal.open(launchReq)
}

// after
onInstalled: () => {
    // ... mark all sibling cards installed ...
    tabModal.replace(buildLaunchRequest(agent));
}
```

### 4.3 Other consumers

None today. Future chained flows (install→auth, auth→launch) opt in by calling `replace` instead of `close`/`open`.

---

## 5. Reduced motion

Already honored by the existing animations via the `respect-reduced-motion` mixin (`frontend/app/mixins.scss:123`). Add the new `tab-modal-content-in` keyframe under the same gate:

```scss
@media (prefers-reduced-motion: reduce) {
    .tab-modal-backdrop, .tab-modal-panel, .tab-modal-content {
        animation: none;
    }
    .tab-modal-panel {
        transition: none;
    }
}
```

Setting source: `window:reducedmotion` setting OR `(prefers-reduced-motion: reduce)` system MQ — both wired through `global.ts:107` and `app.tsx:365`.

---

## 6. Acceptance criteria

1. Install completes → click **Continue to Launch** → no visible backdrop flicker, no entrance-pop replay. The content area crossfades in place.
2. `tabModal.open(launchReq)` (cold open from picker) still plays the full backdrop fade-in + panel pop-in.
3. `tabModal.close()` (Cancel) still unmounts instantly.
4. Under `prefers-reduced-motion: reduce`, all three operations (open/close/replace) are instant — no animations, no crossfade.
5. If install modal height (e.g. 480px with terminal) differs from launch modal height (e.g. 320px without), the panel resizes smoothly during the swap, not abruptly.
6. Browser console shows no warnings about competing animations or unkeyed `<For>` warnings from the swap-key trick.

---

## 7. Out of scope

- True exit animations on `close()`. See §3.6.
- Multi-modal stacking (already not supported by TabModalLayer; explicit non-goal).
- Modal v2 (window-scoped) — same pattern would apply, but no consumer needs it today. Mirror the pattern only when first needed.
- Animating the swap *during* a long-running async hop (e.g. "click → 800ms RPC → next modal"). Use a loading overlay inside the current modal instead; `replace` is for already-resolved transitions.

---

## 8. Implementation order

1. Add `replace()` to `TabModalApi` + impl in `TabModalLayer.tsx` (no consumers yet).
2. Add `.tab-modal-content` + keyframe + reduced-motion guard in `tab-modal.scss`.
3. Add `transition: min-height` on `.tab-modal-panel`.
4. Update AgentPicker to call `tabModal.replace(buildLaunchRequest(agent))` on `onInstalled`.
5. Manual smoke: install + Continue to Launch; full cold-open path; cancel path; reduced-motion both system and setting; install vs. launch height delta.
6. (Optional, phase 2) Wrap `replace` body in `startViewTransition` when available.

Single PR. Changeset entry: `patch — fix(modal): crossfade install→launch handoff to remove the visual jolt`.
