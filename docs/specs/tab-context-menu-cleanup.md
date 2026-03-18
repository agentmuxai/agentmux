# Spec: Tab Context Menu Cleanup

**Date:** 2026-03-18
**Status:** Ready to implement

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
