# Analysis: Consolidate Object Update Return Paths

**Date:** 2026-03-18
**Status:** Analysis

---

## Problem

`UpdateObjectMeta` in `service.rs` only returns `waveobj:update` events for `OTYPE_BLOCK`. All other object types (tab, window, client, workspace) silently succeed — the frontend WOS cache never updates reactively.

This means:
- **Tab color** — set via `UpdateObjectMeta("tab:<id>", {"tab:color": hex})` — doesn't update until something else triggers a re-fetch (like rename)
- **Any meta change on non-block objects** has the same bug

Meanwhile, dedicated endpoints like `UpdateTabName` each have their own bespoke update-return logic. This creates a maintenance burden and inconsistency.

## Current Architecture

```
UpdateObjectMeta(oref, meta)
  → update_object_meta(store, oref, meta)    // persists to DB
  → match oref.otype:
      OTYPE_BLOCK → return updated block       ✓ reactive
      OTYPE_TAB   → success_empty()            ✗ not reactive (FIXED in pending code)
      *           → success_empty()            ✗ not reactive
```

Dedicated endpoints (each with own update logic):
```
UpdateTabName   → tab.name = name; store.update(); return updated tab    ✓
SetMetaCommand  → block only; return updated block                       ✓
UpdateBlock     → store.update(); return updated block                   ✓
```

## Proposed: Generic Update Return

Make `UpdateObjectMeta` return updates for ALL object types:

```rust
// After update_object_meta succeeds:
match oref.otype.as_str() {
    OTYPE_BLOCK => return_updated::<Block>(store, &oref),
    OTYPE_TAB => return_updated::<Tab>(store, &oref),
    OTYPE_WINDOW => return_updated::<Window>(store, &oref),
    OTYPE_WORKSPACE => return_updated::<Workspace>(store, &oref),
    OTYPE_CLIENT => return_updated::<Client>(store, &oref),
    _ => WebReturnType::success_empty(),
}
```

With a helper:
```rust
fn return_updated<T: WaveObj + Serialize>(
    store: &WaveStore, oref: &ORef
) -> WebReturnType {
    match store.must_get::<T>(&oref.oid) {
        Ok(obj) => WebReturnType::success_with_updates(vec![WaveObjUpdate {
            updatetype: "update".into(),
            otype: oref.otype.clone(),
            oid: oref.oid.clone(),
            obj: Some(wave_obj_to_value(&obj)),
        }]),
        Err(_) => WebReturnType::success_empty(),
    }
}
```

## Deduplication Opportunities

Several endpoints duplicate the "update + return updated object" pattern:

| Endpoint | Object | Could Use Generic? |
|----------|--------|-------------------|
| `UpdateObjectMeta` | any | Yes — the proposal above |
| `UpdateTabName` | tab | Yes — could be `UpdateObjectMeta("tab:id", {"name": ...})` |
| `SetMetaCommand` (RPC) | block | Already works, but duplicates logic |
| `UpdateBlock` | block | Separate — bulk update, keep as-is |

### Consolidation Plan

1. **Phase 1 (quick fix):** Make `UpdateObjectMeta` return updates for all otypes (the fix already in `service.rs`)
2. **Phase 2 (cleanup):** Extract `return_updated<T>()` helper to reduce boilerplate
3. **Phase 3 (optional):** Deprecate `UpdateTabName` in favor of `UpdateObjectMeta("tab:id", {"name": ...})` — but this requires the tab `name` field to be in `MetaType` or handled by `merge_meta`

## Files

| File | Lines | Issue |
|------|-------|-------|
| `agentmuxsrv-rs/src/server/service.rs` | 149-177 | `UpdateObjectMeta` — only BLOCK returns updates |
| `agentmuxsrv-rs/src/server/service.rs` | 182-209 | `UpdateTabName` — bespoke tab update return |
| `frontend/app/store/wos.ts` | 131-133 | `callBackendService` — correctly processes returned updates |
| `frontend/app/tab/tab.tsx` | 224 | `ObjectService.UpdateObjectMeta` for tab:color |

## Immediate Fix (Already Applied)

Added `OTYPE_TAB` case to `UpdateObjectMeta` return path in `service.rs`. Needs backend rebuild to take effect.
