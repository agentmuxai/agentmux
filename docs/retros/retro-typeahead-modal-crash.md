# Retro: TypeAheadModal Crash — getBoundingClientRect on null

**Date:** 2026-03-18
**Severity:** Critical — crashes the rendering pipeline, blocks widget interaction
**Introduced:** PR #120 (SolidJS migration, commit `f030661`, 2026-03-09)
**File:** `frontend/app/modals/typeaheadmodal.tsx:138`

---

## Error

```
TypeError: Cannot read properties of null (reading 'getBoundingClientRect')
    at Object.fn (typeaheadmodal.tsx:138)
```

## Root Cause Analysis

### The Bug

`TypeAheadModal` uses SolidJS `let ref!: HTMLDivElement` declarations (lines 90-94) with a `createEffect` (line 109) that reads DOM dimensions. The effect runs when the `height()` signal changes (from ResizeObserver on `blockRef`), but the refs may not yet be attached to the DOM.

### Why the null guard on line 111 doesn't work

```tsx
if (!modalRef || !inputGroupRef || !suggestionsRef || !suggestionsWrapperRef) return;
```

In SolidJS, `let ref!: HTMLDivElement` is a **definite assignment assertion** — TypeScript trusts that the ref will be assigned before use. But SolidJS's `ref={el}` callback runs during JSX rendering, not during component initialization. The `createEffect` can fire (triggered by `height()` signal change from ResizeObserver) **before** the Portal has rendered its children.

Specifically:
1. Component mounts
2. `onMount` creates ResizeObserver on `props.blockRef.current`
3. ResizeObserver fires immediately with initial dimensions → `setHeight(h)`
4. `createEffect` runs because `height()` changed
5. Refs are declared with `!` assertion but Portal children haven't rendered yet
6. `suggestionsWrapperRef` appears truthy (SolidJS may have partially initialized it) but the actual DOM element doesn't exist → `getBoundingClientRect` fails

### The React Version

The React version used `useRef<HTMLDivElement>(null)` which initializes to `null`. The `useEffect` dependency array included the ref values, and React guaranteed refs were attached after the first render. The null check `if (!ref.current)` was reliable.

### The SolidJS Problem

SolidJS `let ref!: T` with the `!` definite assignment assertion is a TypeScript escape hatch, not a runtime guarantee. The variable is `undefined` until the JSX `ref={...}` callback runs. Combined with `Portal` (which renders into a different DOM subtree), the timing is unpredictable.

### Additional Issue: Portal Mounting

Line 206: `<Portal mount={props.blockRef.current}>` renders children into a different DOM subtree. SolidJS Portals create their children asynchronously — the refs inside the Portal may not be assigned when effects in the parent scope run.

---

## Fix

### Option 1: Defer measurement to onMount + explicit null checks (Recommended)

Replace `createEffect` with `onMount` for initial measurement, and use a proper resize callback:

```tsx
let modalRef: HTMLDivElement | null = null;
let inputGroupRef: HTMLDivElement | null = null;
let suggestionsWrapperRef: HTMLDivElement | null = null;
let suggestionsRef: HTMLDivElement | null = null;

const recalcLayout = () => {
    if (!modalRef || !inputGroupRef || !suggestionsRef || !suggestionsWrapperRef) return;

    const h = height();
    if (h <= 0) return;

    const modalStyles = window.getComputedStyle(modalRef);
    const paddingTop = parseFloat(modalStyles.paddingTop) || 0;
    const paddingBottom = parseFloat(modalStyles.paddingBottom) || 0;
    const borderTop = parseFloat(modalStyles.borderTopWidth) || 0;
    const borderBottom = parseFloat(modalStyles.borderBottomWidth) || 0;
    const modalPadding = paddingTop + paddingBottom;
    const modalBorder = borderTop + borderBottom;

    const suggestionsWrapperStyles = window.getComputedStyle(suggestionsWrapperRef);
    const suggestionsWrapperMarginTop = parseFloat(suggestionsWrapperStyles.marginTop) || 0;

    const inputHeight = inputGroupRef.getBoundingClientRect().height;
    let suggestionsTotalHeight = 0;
    for (let i = 0; i < suggestionsRef.children.length; i++) {
        suggestionsTotalHeight += suggestionsRef.children[i].getBoundingClientRect().height;
    }

    const totalHeight = modalPadding + modalBorder + inputHeight + suggestionsTotalHeight + suggestionsWrapperMarginTop;
    const maxHeight = h * 0.8;
    const computedHeight = totalHeight > maxHeight ? maxHeight : totalHeight;

    modalRef.style.height = `${computedHeight}px`;
    suggestionsWrapperRef.style.height = `${computedHeight - inputHeight - modalPadding - modalBorder - suggestionsWrapperMarginTop}px`;
};

// Use createEffect but with robust null guards and height > 0 check
createEffect(() => {
    const h = height();
    if (h <= 0) return;
    // Schedule after next microtask to ensure Portal children are rendered
    queueMicrotask(recalcLayout);
});
```

Key changes:
- Drop `!` definite assignment assertions — use `| null = null` instead
- Extract measurement into a named function
- Use `queueMicrotask()` to defer measurement until Portal children are rendered
- Guard on `h <= 0` to skip initial zero-height state

### Option 2: Use requestAnimationFrame (Simpler)

```tsx
createEffect(() => {
    const h = height();
    if (h <= 0) return;
    requestAnimationFrame(() => {
        if (!modalRef || !inputGroupRef || !suggestionsRef || !suggestionsWrapperRef) return;
        // ... measurement code
    });
});
```

`requestAnimationFrame` guarantees the browser has finished layout before we measure. Simpler but slightly delayed.

### Width Effect (line 142) — Same Issue

The width `createEffect` at line 142 has the same potential crash if `props.blockRef.current` or `props.anchorRef.current` is null after a Portal remount. Apply the same `queueMicrotask` + null guard pattern.

---

## Related Issues

1. **Widget clicks broken:** The crash in `createEffect` corrupts the SolidJS reactive graph. Subsequent effects (including widget click handlers) may not fire because the owner computation is in an error state.

2. **Connection typeahead:** `conntypeahead.tsx` uses `TypeAheadModal` — same crash can occur there.

---

## Files

| File | Lines | Issue |
|------|-------|-------|
| `frontend/app/modals/typeaheadmodal.tsx` | 90-94 | `let ref!:` definite assignment — unsafe with Portal |
| `frontend/app/modals/typeaheadmodal.tsx` | 109-140 | `createEffect` reads DOM before Portal renders |
| `frontend/app/modals/typeaheadmodal.tsx` | 142-171 | Width effect — same timing issue |
| `frontend/app/modals/typeaheadmodal.tsx` | 206 | Portal mount — async child rendering |
