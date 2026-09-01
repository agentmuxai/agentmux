# Spec: Tab Context Menu Cleanup

**Date:** 2026-03-18
**Status:** historical — moot (confirmed 2026-07-29, DOC-009 documentation
analyst). No successor document exists to point at: nothing replaced this spec,
its proposal simply never shipped and the area was built differently. Its own
text says "kept for historical reference only", which is what `historical`
means. `tab.tsx`'s `handleContextMenu` no longer opens a native menu at all; right-click opens a custom `TabContextPanel` (a 14-color `ColorSwatchPalette` plus Rename/Close buttons). None of this spec's described current-state baseline (native menu with Pin Tab/Rename/Copy TabId/Color/Backgrounds/Close) or its proposal (emoji labels) match what shipped. Kept for historical reference only — moved to `specs/archive`.

---

## Changes

### 1. Promote "Color" to top-level with emoji

Currently "Color" is a plain label after a separator. Move it up and add emoji prefix.

### 2. Add emojis to all menu items

Emojis go in the `label` string (native menu supports Unicode).

### Current Menu

```
Pin Tab
Rename Tab
Copy TabId
───────────
Color
───────────
Backgrounds  ▸
───────────
Close Tab
```

### Proposed Menu

```
📌 Pin Tab          (or "📌 Unpin Tab")
✏️ Rename Tab
🎨 Color
🖼️ Backgrounds  ▸
📋 Copy Tab ID
───────────
🗑️ Close Tab
```

### Notes

- No separator before Color — it flows naturally after Rename
- Copy TabId moved down (less common action)
- Single separator before Close (destructive action)
- Backgrounds submenu stays as-is (already has display names)
- The native OS menu left column will show the emoji; no gap issue since every item has one

## File

`frontend/app/tab/tab.tsx` — `handleContextMenu` function (~line 229)
