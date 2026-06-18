# Retro: Circular Import → Blank Loading Screen
Date: 2026-03-21
Author: AgentA

---

## What Happened

### Symptom
App was stuck on the loading logo animation after a clean `task dev` restart. No `[fe]` log entries appeared at all — the frontend JS never initialized. The backend started fine.

### Root Cause: Circular ES Module Import

When wiring up `setCurrentDragPayload` in `TileLayout.win32.tsx`, the import chain formed a cycle:

```
TileLayout.win32.tsx
  → @/app/drag/CrossWindowDragMonitor   (new import we added)
  → @/layout/index                       (existing import in CrossWindowDragMonitor for LayoutNode type)
  → ./lib/TileLayout.platform            (re-exports TileLayout)
  → TileLayout.win32.tsx                 (resolved at build time on Windows)
```

ES module cycles don't always crash — sometimes they silently produce `undefined` bindings at the point of the cycle. In this case, the cycle caused the Vite module graph to fail to evaluate the module, leaving the app in a broken state where the JS entry point never finished loading. No error was surfaced to the log because the log pipe (`initLogPipe()`) hadn't initialized yet — it's called from within the app code that never ran.

### Why It Wasn't Caught Before Build

`tsc --noEmit` does not detect circular imports — it only checks types. The circular import was structurally valid TypeScript. Vite's dev server also doesn't fail on cycles by default; it attempts to evaluate them and silently produces broken state.

### The Specific Bad Import

In `CrossWindowDragMonitor.tsx`:
```typescript
// BEFORE (caused cycle)
import type { LayoutNode } from "@/layout/index";

// AFTER (fixed)
import type { LayoutNode } from "@/layout/lib/types";
```

`@/layout/index` re-exports `TileLayout` (which comes from `TileLayout.platform` → `TileLayout.win32.tsx`).
`@/layout/lib/types` is a pure type file with no component imports — no cycle possible.

---

## Why I Got It Wrong

### Failure mode: barrel file import for a type

`CrossWindowDragMonitor.tsx` already had `import type { LayoutNode } from "@/layout/index"` — this was a pre-existing import through the barrel file. I added the new import of `CrossWindowDragMonitor` in `TileLayout.win32.tsx` without checking whether `CrossWindowDragMonitor` itself imported anything back from the layout module.

The barrel `@/layout/index` was used for convenience to get `LayoutNode`. If the import had been from the source file (`@/layout/lib/types`) to begin with, the cycle would never have existed.

### Key signals I missed

1. `[fe]` logs completely absent after backend started — classic sign of a JS initialization crash before log pipe init
2. HMR corruption in the prior session (widgets dead, clicks doing nothing) — this was also caused by the same cycle appearing mid-session via HMR partial updates. HMR applied the `TileLayout` change, which introduced the cycle, and then SolidJS module state became inconsistent.
3. `tsc --noEmit` passed — should not have been treated as "all clear" for import correctness

---

## Plan Forward

### Immediate (done)
- Changed `@/layout/index` → `@/layout/lib/types` in `CrossWindowDragMonitor.tsx`

### Process Changes

1. **When importing a type from a module that could create a cycle:** prefer the source file over barrel files. Barrel files (`index.ts`) re-export components and may create cycles; type files (`types.ts`) typically don't.

2. **When adding a cross-module import:** mentally trace: does the new import target import (directly or transitively) from the file I'm editing? For layout ↔ drag modules especially — these are tightly coupled and cycles are likely.

3. **"No `[fe]` logs at all" = JS init crash.** When the loading screen is stuck and there are zero `[fe]` entries, don't look at the backend — look for a module evaluation error (circular import, syntax error, missing export).

4. **HMR corruption is a symptom, not the root cause.** When HMR leaves the app in a broken state (clicks do nothing, widgets dead), restart is the right move — but also investigate *why* HMR broke. In this case it was the same cycle appearing mid-session.
