# SPEC — Pane Minimize Caret Not Flipping

**Date:** 2026-06-24
**Status:** Root cause confirmed

---

## Symptom

Clicking the minimize button collapses a pane correctly (the pane shrinks to its header bar or slips into the adjacent column), but the chevron icon on the button does not change from `chevron-down` to `chevron-up`. Clicking again to restore also works, but the icon remains static throughout.

---

## Reactive chain (correctly implemented)

The intended data flow is:

```
minimizeNodeToggle()
  └─ _finishToggle()
       └─ model.minimizedNodeIds._set(...)   ← SignalAtom update
            └─ isMinimized createMemo         ← recomputes → true
                 └─ EndIcons.minimized()       ← accessor, reads memo
                      └─ OptMinimizeButton props.minimized getter
                           └─ createMemo<IconButtonDecl>()   ← recomputes
                                └─ icon: props.minimized ? "chevron-up" : "chevron-down"
                                     └─ IconButton decl prop getter
                                          └─ DOM <i class="fa-chevron-up">   ← NEVER UPDATES
```

Every step in the chain up to `createMemo<IconButtonDecl>()` is correct:

- `minimizedNodeIds` is a `SignalAtom`; `._set()` fires synchronously.
- `isMinimized` is a `createMemo` inside `model.runInModelRoot()` that reads `model.minimizedNodeIds()`. Cross-root memo reads work fine in SolidJS.
- `minimized = () => props.nodeModel.isMinimized()` in `EndIcons` is a reactive accessor.
- `<OptMinimizeButton minimized={minimized()} />` is compiled by SolidJS's JSX compiler as a getter: `get minimized() { return minimized(); }`. The prop IS reactive.
- Inside `OptMinimizeButton`, `createMemo<IconButtonDecl>(() => ({ icon: props.minimized ? "chevron-up" : "chevron-down", ... }))` reads `props.minimized`, which reads through the getter chain. The memo DOES recompute when `minimized()` changes.

The chain breaks at `IconButton`.

---

## Root cause — `IconButton` destructures `decl` from props

`frontend/app/element/iconbutton.tsx` line 12:

```typescript
export function IconButton({ decl, className }: IconButtonProps): JSX.Element {
    let btnRef!: HTMLButtonElement;
    const spin = decl.iconSpin ?? false;           // ← static snapshot
    useLongClick(() => btnRef, decl.click, decl.longClick, decl.disabled);
    const disabled = decl.disabled ?? false;        // ← static snapshot
    return (
        <button
            title={decl.title}                      // ← stale string
            disabled={disabled}                     // ← stale boolean
        >
            {typeof decl.icon === "string"
                ? <i class={makeIconClass(decl.icon, ...)} />  // ← STALE ICON
                : decl.icon}
        </button>
    );
}
```

**The problem:** SolidJS compiles `<OptMinimizeButton minimized={minimized()} />` as:

```js
createComponent(OptMinimizeButton, {
    get minimized() { return minimized(); }
})
```

When `OptMinimizeButton` renders `<IconButton decl={decl()} />`, SolidJS compiles it as:

```js
createComponent(IconButton, {
    get decl() { return decl(); }  // reactive getter
})
```

`IconButton`'s component function runs ONCE (SolidJS components do not re-run on prop changes — that is React's model). The destructuring `{ decl }` reads the getter exactly once, storing the current `IconButtonDecl` object in a local variable. All subsequent JSX reads (`decl.icon`, `decl.title`, etc.) read from this frozen snapshot.

When `OptMinimizeButton`'s `decl()` memo recomputes and returns a new object (`{ icon: "chevron-up", ... }`), the `props.decl` getter now returns that new object — but `IconButton`'s local `decl` still points to the old one. SolidJS has no signal to track on a plain object property, so the DOM is never updated.

**This is a standard SolidJS pitfall.** The [SolidJS docs](https://www.solidjs.com/docs/latest/api) explicitly warn: _"Destructuring props will lose reactivity unless done within a reactive scope."_

---

## Why other buttons don't appear broken

Most `IconButton` usages pass a literal object inline:

```tsx
<IconButton decl={{ elemtype: "iconbutton", icon: "xmark-large", click: ... }} />
```

A literal object is static — it never changes after mount. The destructuring bug is silent because there's nothing to react to. Only usages that construct `decl` from reactive state (like the minimize button's `createMemo`) expose the bug.

---

## Fix

`frontend/app/element/iconbutton.tsx` — stop destructuring, use `props.decl` directly so SolidJS's JSX compiler can track the reactive getter in every attribute expression:

```typescript
export function IconButton(props: IconButtonProps): JSX.Element {
    let btnRef!: HTMLButtonElement;
    useLongClick(
        () => btnRef,
        (e) => props.decl.click?.(e),
        (e) => props.decl.longClick?.(e),
        props.decl.disabled ?? false
    );
    return (
        <button
            ref={btnRef}
            class={clsx("wave-iconbutton", props.className, props.decl.className, {
                disabled: props.decl.disabled ?? false,
                "no-action": props.decl.noAction,
            })}
            title={props.decl.title}
            aria-label={props.decl.title}
            style={{ color: props.decl.iconColor ?? "inherit" }}
            disabled={props.decl.disabled ?? false}
        >
            {typeof props.decl.icon === "string"
                ? <i class={makeIconClass(props.decl.icon, true, { spin: props.decl.iconSpin ?? false })} />
                : props.decl.icon}
        </button>
    );
}
```

**Why this fixes it:** For DOM element attributes and children, SolidJS wraps each expression in a reactive effect. Reading `props.decl` inside those effects calls the `get decl()` getter, which calls `decl()` (the memo), which is tracked. When the memo recomputes (because `props.minimized` changed), SolidJS re-runs all dependent effects and the DOM updates.

**`useLongClick` handler wrapping:** The click/longClick callbacks are now lambdas that read `props.decl.click` at call time rather than at mount time. This picks up the current handler if `decl` changes between mount and click — correct behavior in general.

**`disabled` in `useLongClick`:** `props.decl.disabled ?? false` is still evaluated once at mount (passed by value to `useLongClick`). For the minimize button, `disabled` is always `false`. Fully reactive `disabled` gating in `useLongClick` would require changing its signature — out of scope here.

---

## Files to change

| File | Line | Change |
|---|---|---|
| `frontend/app/element/iconbutton.tsx` | 12–36 | Remove destructuring; use `props.decl` in JSX |

No changes needed in:
- `blockframe.tsx` — reactive chain above `IconButton` is correct
- `layoutMinimize.ts` — `_finishToggle` correctly calls `minimizedNodeIds._set()`
- `layoutNodeModels.ts` — `isMinimized` createMemo is correctly reactive
- `layoutGeometry.ts` — `updateTree` + `cleanupNodeModels` preserve the node model

---

## Verification

After the fix:
1. Open a workspace with two or more panes.
2. Click the `▾` (chevron-down) minimize button on any pane.
3. The pane collapses; the button icon should immediately change to `▴` (chevron-up).
4. Click again; the pane expands and the icon returns to `▾`.
5. Repeat for slip-minimize (pane in a Row slot with no vertical sibling) to confirm both paths.
