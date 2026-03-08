# Sysinfo Module Modularization Plan

**Date:** 2026-03-08
**Current state:** Single 589-line file (`frontend/app/view/sysinfo/sysinfo.tsx`)
**Goal:** Split into focused modules for maintainability as we add complexity

---

## Current Structure (sysinfo.tsx — 589 lines)

| Lines     | Responsibility                    |
|-----------|-----------------------------------|
| 1-25      | Imports, constants                |
| 26-115    | Types, plot metadata defaults     |
| 116-263   | `SysinfoViewModel` class          |
| 264-340   | `getDefaultData()`, `getSettingsMenuItems()` |
| 341-361   | Helper functions                  |
| 363-413   | `SysinfoView` (subscription + connection) |
| 414-547   | `SingleLinePlot` (Observable Plot rendering) |
| 549-589   | `SysinfoViewInner` (layout + grid) |

---

## Proposed Module Structure

```
frontend/app/view/sysinfo/
├── index.ts                  # Re-exports (barrel file)
├── sysinfo-model.ts          # SysinfoViewModel class
├── sysinfo-view.tsx          # SysinfoView + SysinfoViewInner components
├── sysinfo-plot.tsx          # SingleLinePlot component
├── sysinfo-types.ts          # Types, interfaces, PlotMeta defaults
└── sysinfo-util.ts           # Helper functions, data conversion
```

### Module Details

#### `sysinfo-types.ts` (~80 lines)
- `DataItem` type alias
- `PlotMeta` interface (already `SysinfoPlotYValMeta`)
- `DefaultPlotMeta` map
- `PlotTypes` mapping (CPU/Memory/Network metric selectors)
- `DefaultNumPoints` constant
- `SysinfoViewProps` interface

#### `sysinfo-model.ts` (~180 lines)
- `SysinfoViewModel` class
- `viewType`, atoms, `loadInitialData()`, `getDefaultData()`
- `getSettingsMenuItems()` context menu builder
- Imports types from `sysinfo-types.ts`

#### `sysinfo-view.tsx` (~80 lines)
- `SysinfoView` — WPS event subscription, connection status gating
- `SysinfoViewInner` — layout grid, maps yvals to plots
- This is the "controller" layer between the model and the plot

#### `sysinfo-plot.tsx` (~150 lines)
- `SingleLinePlot` — Observable Plot rendering
- `resolveDomainBound()` helper
- All `@observablehq/plot` imports isolated here
- Future: swap in canvas-based renderer without touching other files

#### `sysinfo-util.ts` (~30 lines)
- `convertWaveEventToDataItem()`
- Any future data transformation helpers

#### `index.ts` (~5 lines)
```typescript
export { SysinfoViewModel } from "./sysinfo-model";
export { SysinfoView } from "./sysinfo-view";
```

---

## Migration Steps

1. Create `sysinfo-types.ts` — extract types and constants
2. Create `sysinfo-util.ts` — extract helper functions
3. Create `sysinfo-plot.tsx` — extract `SingleLinePlot` + `resolveDomainBound`
4. Create `sysinfo-model.ts` — extract `SysinfoViewModel` class
5. Create `sysinfo-view.tsx` — remaining view components
6. Create `index.ts` — barrel exports
7. Delete original `sysinfo.tsx`
8. Update imports in `block.tsx` (block registry)
9. Verify: `tsc --noEmit` + visual test

Each step is independently committable — no big-bang refactor needed.

---

## Future Complexity This Enables

With the modular structure, these additions become straightforward:

- **New plot types** (disk I/O, GPU, per-process) → add to `sysinfo-types.ts` PlotTypes + new metric collectors
- **Canvas/WebGL renderer** → swap `sysinfo-plot.tsx` without touching model or view
- **Interval-aware x-axis** → change in `sysinfo-plot.tsx` only
- **Per-connection models** → extend `sysinfo-model.ts` with connection pooling
- **Alerting/thresholds** → add `sysinfo-alerts.ts`, subscribe in `sysinfo-view.tsx`
- **Custom metrics** → plugin interface in `sysinfo-types.ts`
