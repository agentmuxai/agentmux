---
type: patch
---

refactor(A3): split global.ts god-module; break global.ts ⇄ wos.ts cycle

Extracts three focused modules from the 1047-line `store/global.ts`:

- `app-api.ts` — `getApi()` host bridge accessor (13 LOC)
- `config-signals.ts` — `fullConfigAtom`, `settingsAtom`, `hasCustomAIPresetsAtom` (23 LOC)
- `block-atom-cache.ts` — per-block/tab SolidJS signal caches and settings
  memo helpers: `getBlockMetaKeyAtom`, `getSettingsKeyAtom`, `getOverrideConfigAtom`,
  `useBlockAtom`, etc. (260 LOC)

All extractions are re-exported from `global.ts` for full backward-compat — no
callers change import paths.

Cycle fixed: `wos.ts` imported `getApi` from `global.ts`, while `global.ts`
imported `* as WOS` from `wos.ts`. Now `wos.ts` imports from `app-api.ts`.

Leaf violations fixed:
- `util/logger.ts` — now imports `getApi` from `@/app/store/app-api`
- `layout/lib/layoutModel.ts` — now imports `getSettingsKeyAtom` from
  `@/app/store/block-atom-cache`; unused `atoms` import removed

Part of the A1–A15 architecture refactor board (A3).
