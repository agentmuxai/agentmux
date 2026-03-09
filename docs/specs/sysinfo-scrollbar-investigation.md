# Sysinfo Scrollbar Regression — Investigation

**Date:** 2026-03-08
**Version:** 0.31.82
**Symptom:** CPU plot now shows a scrollbar; no longer resizes with pane.

---

## Comparison: Old vs New

The modularization moved code between files but the component structure is
identical. Specifically:

### Unchanged
- `OverlayScrollbarsComponent` wrapper with same className and options
- Grid layout: `grid-rows-[repeat(auto-fit,minmax(100px,1fr))]`
- `SingleLinePlot` container: `<div ref={containerRef} className="min-h-[100px]" />`
- `useDimensionsWithExistingRef(containerRef, 300)` for plot sizing
- `Plot.plot({ width: plotWidth, height: plotHeight, ... })` for SVG sizing

### Changed
1. **New `intervalSecs` prop to `SingleLinePlot`** — affects x-axis domain
   calculation: `maxX - targetLen * intervalSecs * 1000` (was `targetLen * 1000`)
2. **New `fullConfigAtom` subscription in `SysinfoViewInner`** (line 84) — reads
   `telemetry:interval` from settings to pass as `intervalSecs` prop
3. **`viewComponent` wired via `Object.defineProperty`** instead of direct getter

---

## Most Likely Cause: `fullConfigAtom` Re-render Cascade

The original `SysinfoViewInner` did NOT subscribe to `fullConfigAtom`. The new
version does:

```typescript
// NEW — line 84 of sysinfo-view.tsx
const fullConfig = jotai.useAtomValue(atoms.fullConfigAtom);
const intervalSecs = fullConfig?.settings?.["telemetry:interval"] || 1.0;
```

`fullConfigAtom` is a large object that gets replaced (new reference) on every
config broadcast. Even if `telemetry:interval` hasn't changed, `React.memo`'s
shallow comparison sees a new `fullConfig` object, causing `SysinfoViewInner`
to re-render.

However, `SysinfoViewInner` is wrapped in `React.memo` which compares props,
not internal atom subscriptions. Since `fullConfig` is read via `useAtomValue`
(not passed as a prop), the memo doesn't help — the atom subscription triggers
a re-render directly.

Each re-render causes:
1. OverlayScrollbarsComponent unmounts/remounts its content
2. The grid and plot containers lose their measured dimensions briefly
3. `useDimensionsWithExistingRef` fires with `0` dimensions, then re-measures
4. During the 0-dimension flash, OverlayScrollbars detects content overflow and
   shows a scrollbar
5. The scrollbar persists because OverlayScrollbars caches its state

This would explain why the plot "doesn't change size" — it's re-rendering at
a fixed size from the last measurement, not responding to container resize.

---

## Secondary Possibility: `Object.defineProperty` viewComponent

If `Object.defineProperty` on the prototype doesn't correctly override the
class getter, `viewComponent` could return `null` on some code paths,
causing the block system to show "No View Component" or remount.

Test: Add `console.log` in `SysinfoView` mount to verify it's only called once.

---

## Fix Options

### Option A: Derived atom for intervalSecs (Recommended)
Add `intervalSecsAtom` to the model — a derived atom that reads only the
specific setting, not the entire fullConfig. Only re-renders when the value
actually changes.

```typescript
// In SysinfoViewModel constructor
this.intervalSecsAtom = jotai.atom((get) => {
    const fullConfig = get(atoms.fullConfigAtom);
    return fullConfig?.settings?.["telemetry:interval"] || 1.0;
});
```

Then in `SysinfoViewInner`:
```typescript
const intervalSecs = jotai.useAtomValue(model.intervalSecsAtom);
```

### Option B: Remove fullConfigAtom from SysinfoViewInner entirely
Pass `intervalSecs` from `SysinfoView` (the parent) via a ref or context,
so `SysinfoViewInner` doesn't subscribe to config at all.

### Option C: Fix viewComponent wiring
Replace `Object.defineProperty` with a simpler pattern — pass the view
component during barrel registration instead of patching the prototype.
