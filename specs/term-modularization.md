# Term View Modularization Spec

**Status:** Ready to implement
**Date:** 2026-03-04
**Owner:** AgentA
**Target file:** `frontend/app/view/term/term.tsx` (1,181 lines)

---

## Problem

`term.tsx` is a 1,181-line file with 4 distinct responsibilities:

1. **TermViewModel class** (lines 49-815) — constructor with ~30 atom declarations, keybinding handlers, shell process status, VDom model accessors, controller restart, settings menu building
2. **VDom sub-components** (lines 841-963) — TermResyncHandler, TermVDomToolbarNode, TermVDomNodeSingleId, TermVDomNode, TermToolbarVDomNode
3. **TerminalView component** (lines 965-1179) — React component with search integration, terminal initialization, resize observer, scrollbar handling, multi-input
4. **Settings menu** (lines 570-814) — `getSettingsMenuItems()` is a 245-line method that builds theme, font size, zoom, transparency, and debug submenus

The TermViewModel constructor alone is 310 lines. The settings menu method is pure menu-building with no interaction with the rest of the class beyond reading atom values.

---

## Existing Module Context

The term directory already has reasonable separation:

| File | Lines | Responsibility | Status |
|------|-------|----------------|--------|
| `termwrap.ts` | 772 | xterm.js wrapper, PTY I/O, resize | Clean |
| `termsticker.tsx` | 163 | Sticker overlay rendering | Clean |
| `term-wsh.tsx` | 145 | WSH client for terminal | Clean |
| `ijson.tsx` | 117 | IJSON rendering | Clean |
| `fitaddon.ts` | 100 | Custom xterm fit addon | Clean |
| `termutil.ts` | 36 | Theme computation | Clean |
| `termtheme.ts` | 30 | Theme updater component | Clean |
| **term.tsx** | **1,181** | **Everything else** | **Target** |

External imports of `term.tsx` exports:
- `block.tsx` imports `TermViewModel`
- `term-wsh.tsx` imports `TermViewModel`
- `termtheme.ts` imports `TermViewModel`

Public API: `makeTerminalModel`, `TerminalView`, `TermViewModel`

---

## Proposed Split

### New File Structure

```
frontend/app/view/term/
  term.tsx                →  ~180 lines  (TerminalView component + exports)
  termViewModel.ts        →  ~380 lines  (TermViewModel class — constructor, atoms, methods)
  termSettingsMenu.ts     →  ~260 lines  (getSettingsMenuItems extracted as standalone function)
  termVDom.tsx            →  ~140 lines  (VDom sub-components: resync handler, toolbar, vdom nodes)
```

Total: ~960 lines (shrinks by ~220 lines of repetitive menu-building that gets tightened).

### Design: Same Mixin Pattern as LayoutModel

Extract functions that take `TermViewModel` as a parameter. The class delegates to them. Zero external API changes.

---

## Module Breakdown

### 1. `termSettingsMenu.ts` (~260 lines)

**Extracted from:** `getSettingsMenuItems()` method (lines 570-814)

**Functions:**
- `buildSettingsMenuItems(model: TermViewModel): ContextMenuItem[]` — the full settings menu builder

**Why separate:** This is a pure menu-construction function. It reads atom values via `globalStore.get()` and returns a `ContextMenuItem[]`. It has no side effects beyond the click handlers (which call `RpcApi.SetMetaCommand`). At 245 lines, it's the single largest method on the class.

**Submenus extracted:**
- Theme submenu
- Font size submenu
- Terminal zoom submenu
- Transparency submenu
- Force restart
- Clear output on restart
- Run on startup
- Close toolbar
- Debug connection

---

### 2. `termVDom.tsx` (~140 lines)

**Extracted from:** lines 841-963

**Components moved:**
- `TermResyncHandler` — connection status resync
- `TermVDomToolbarNode` — VDom toolbar sub-block
- `TermVDomNodeSingleId` — VDom content sub-block (single ID wrapper)
- `TermVDomNode` — VDom content sub-block (null guard)
- `TermToolbarVDomNode` — VDom toolbar sub-block (null guard)

**Why separate:** These are 5 small React components that render VDom sub-blocks inside the terminal. They're self-contained — they subscribe to events, create sub-block node models, and render `<SubBlock>`. They only need `TermViewModel` for accessing `model.connStatus`, `model.termRef`, `model.vdomBlockId`, etc.

---

### 3. `termViewModel.ts` (~380 lines)

**Extracted from:** TermViewModel class (lines 49-815 minus settings menu)

**Contains:**
- `TermViewModel` class definition with all atom declarations
- Constructor (atom initialization, shell proc status subscription)
- `isBasicTerm()`, `multiInputHandler()`, `sendDataToController()`
- `setTermMode()`, `triggerRestartAtom()`, `updateShellProcStatus()`
- `getVDomModel()`, `getVDomToolbarModel()`
- `dispose()`, `giveFocus()`
- `keyDownHandler()`, `handleTerminalKeydown()`
- `setTerminalTheme()`, `forceRestartController()`
- `getAllBasicTermModels()` helper
- `makeTerminalModel()` factory

**`getSettingsMenuItems()` becomes a delegate:**
```typescript
getSettingsMenuItems(): ContextMenuItem[] {
    return buildSettingsMenuItems(this);
}
```

---

### 4. `term.tsx` (what remains, ~180 lines)

- `TerminalView` React component (lines 965-1179)
- Search integration hooks and callbacks
- Terminal initialization effect
- Resize observer
- Scrollbar show/hide observer
- Multi-input effect
- Re-exports: `makeTerminalModel`, `TerminalView`, `TermViewModel`

---

## Implementation Order

| Step | Module | Risk | Verify |
|------|--------|------|--------|
| 1 | `termSettingsMenu.ts` | Low — pure function, no side effects | Right-click terminal → settings menu works |
| 2 | `termVDom.tsx` | Low — self-contained React components | VDom mode toggle, toolbar rendering |
| 3 | `termViewModel.ts` | Low — class moves wholesale, term.tsx re-exports | Terminal opens, types, focus, restart all work |

---

## What NOT to Change

- **No changes to `termwrap.ts`, `termsticker.tsx`, `term-wsh.tsx`, `termtheme.ts`, `termutil.ts`**
- **No changes to imports outside `view/term/`** — `block.tsx` and others import from `term.tsx` which re-exports
- **No new classes or inheritance**
- **No functional changes** — pure refactor, zero behavior changes

---

## Visibility Changes

All `TermViewModel` properties are already public (no `private` keyword), so no visibility changes needed. This is simpler than the LayoutModel refactor.

---

## Success Criteria

- [ ] `term.tsx` is under 200 lines
- [ ] No file in `view/term/` exceeds 800 lines (termwrap.ts is currently 772)
- [ ] Zero import changes outside `view/term/`
- [ ] `tsc --noEmit` passes
- [ ] Manual verification: terminal opens, types, resize, search, settings menu, VDom mode, restart controller
- [ ] Hot reload works in `task dev`
