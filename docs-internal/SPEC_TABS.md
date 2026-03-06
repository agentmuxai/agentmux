# SPEC: Tab Bar — Clean Rewrite Plan

## Current State Audit

### Files in `frontend/app/tab/`

| File | Status | Decision |
|------|--------|----------|
| `tab.tsx` | **Active** — Tab chip component (name, close, pin) | Keep — reuse in new TabBar |
| `tab.scss` | **Active** — styles for Tab chip | Keep |
| `tabcontent.tsx` | **Active** — renders TileLayout for a tabId | Keep — unchanged |
| `tabbar-model.ts` | Minimal — only `jigglePinAtom` for pinned tab animation | Keep — Tab component uses it |
| `widgetbar.tsx` | **Orphaned** — duplicate of `action-widgets.tsx`, zero imports | **Delete** |
| `workspaceswitcher.tsx` + `.scss` | **Orphaned** — nothing imports it | **Delete** |
| `workspaceeditor.tsx` + `.scss` | **Orphaned** — only imported by workspaceswitcher | **Delete** |

### Files in `frontend/app/window/`

| File | Status | Decision |
|------|--------|----------|
| `window-header.tsx` | Active — 61 lines, drag region + SystemStatus | Keep — add TabBar here |
| `window-header.scss` | Active — has orphaned `.tab-bar`, `.tabs-wrapper` styles still present | Keep — clean up orphaned styles |
| `action-widgets.tsx` | **Active** — renders widget buttons (term, web, help, devtools) | Keep — already live in SystemStatus |
| `system-status.tsx` | **Active** — ActionWidgets + window buttons | Keep |
| `update-banner.tsx` | **Orphaned** — no importers, update UI lives in StatusBar now | **Delete** |

### Workspaces

The `WorkspaceSwitcher` lets users switch between named workspaces (each = a set of tabs with a theme). With multi-window support now live, workspaces are not needed — opening a new window serves the same purpose. The backend still has the workspace data model, but the UI adds complexity for minimal gain.

**Decision: Leave workspace switcher out of scope.** The tab bar will render tabs from the single current workspace only.

---

## Visual Layout

### BEFORE (current) — macOS

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│ ●  ●  ●  ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░  [>_ term][? help][⌨ dev]  [─][□][×] │  ← WindowHeader 33px
│           └── WindowDrag (middle, flex-grow, empty) ──┘  └── ActionWidgets ──┘  └─ WinBtns ┘ │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│                                                                                 │
│                         TabContent (single static tab)                         │
│                           TileLayout → Blocks                                  │
│                                                                                 │
│                                                                                 │
├─────────────────────────────────────────────────────────────────────────────────┤
│ ● backend  ⇄ local                              ⚙ config   ↑ update   v0.31.44 │  ← StatusBar
└─────────────────────────────────────────────────────────────────────────────────┘
```

### AFTER (with tabs restored) — macOS

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│ ●  ●  ●  ┌──────────┐ ┌──────────┐ ┌──────────┐ [+]░░░░░  [>_ term][? help][⌨ dev]  [─][□][×] │  ← WindowHeader 33px
│          │  main  × │ │ work   × │ │ notes  × │         │  └── ActionWidgets ─┘  └─ WinBtns ┘ │
│          └──────────┘ └──────────┘ └──────────┘         │                                     │
│          └──────── TabBar (flex-grow, scrollable) ───────┘                                     │
├─────────────────────────────────────────────────────────────────────────────────┤
│                                                                                 │
│                                                                                 │
│                      TabContent (active tab — switches on click)                │
│                           TileLayout → Blocks                                  │
│                                                                                 │
│                                                                                 │
├─────────────────────────────────────────────────────────────────────────────────┤
│ ● backend  ⇄ local                              ⚙ config   ↑ update   v0.31.44 │  ← StatusBar
└─────────────────────────────────────────────────────────────────────────────────┘
```

### AFTER — with pinned tab

```
┌─────────────────────────────────────────────────────────────────────────────────┐
│ ●  ●  ●  ┌──────────┐│┌──────────┐ ┌──────────┐ [+]░░░░░  [>_ term][? help][⌨ dev]  [─][□][×] │
│          │ 📌 ai    ││ │  main  × │ │ work   × │         │                                     │
│          └──────────┘│└──────────┘ └──────────┘         │                                     │
│          └─pinned──┘ │└──────────── regular tabs ───────┘│                                     │
│                      └ spacer                                                                   │
├─────────────────────────────────────────────────────────────────────────────────┤
```

### Tab states

```
Active tab:     ┌────────────┐
                │  main    × │   — full opacity, visible bottom border
                └────────────┘

Inactive tab:   ┌────────────┐
                │  work    × │   — reduced opacity
                └────────────┘

Pinned tab:     ┌────────────┐
                │ 📌 ai      │   — pin icon replaces close button
                └────────────┘

Overflow:       ┌──────┐ ┌──────┐ ┌──────┐ ┌──────┐ [+]  ◄► scroll
                │ tab1 │ │ tab2 │ │ tab3 │ │ tab4 │
                └──────┘ └──────┘ └──────┘ └──────┘
```

### Component tree (after)

```
WindowHeader
  ├── WindowDrag.left          (macOS traffic light space, flex-shrink:0)
  ├── TabBar                   (flex-grow, min-width:0, horizontally scrollable)
  │     ├── [pinned tab chips] (workspace.pinnedtabids)
  │     ├── <div.spacer/>      (only if pinnedtabids.length > 0)
  │     ├── [regular tab chips](workspace.tabids)
  │     └── <button.add-tab>   (+)
  └── SystemStatus             (flex-shrink:0)
        ├── ActionWidgets      ([>_ term][? help][⌨ devtools][custom...])
        └── WindowActionButtons([─][□][×])

workspace.tsx
  └── TabContent(tabId=activeTabIdAtom)   ← derived from workspace.activetabid
```

### Current Architecture (before fix)

```
WindowHeader
  ├── WindowDrag.left
  ├── WindowDrag.middle   ← flex-grow filler, takes all tab space
  └── SystemStatus
        ├── ActionWidgets
        └── WindowActionButtons

workspace.tsx
  └── TabContent(tabId=staticTabId)   ← set once at init, never changes
```

### Core Problem

`atoms.staticTabId` is set once at window init (`initOpts.tabId`) and never changes. There is no reactive atom tracking the currently active tab. Even if a tab bar existed, clicking it would have no visible effect.

---

## Implementation Plan

### Step 1: Delete orphaned files

Remove all dead code before adding new code.

Files to delete:
- `frontend/app/tab/widgetbar.tsx`
- `frontend/app/tab/workspaceswitcher.tsx`
- `frontend/app/tab/workspaceswitcher.scss`
- `frontend/app/tab/workspaceeditor.tsx`
- `frontend/app/tab/workspaceeditor.scss`
- `frontend/app/window/update-banner.tsx`

Clean orphaned CSS from `window-header.scss`:
- Remove `.tabs-wrapper`, `.tab-bar`, `.pinned-tab-spacer`, `.os-theme-dark/.os-theme-light` scrollbar overrides (will be re-added cleanly in TabBar's own scss)

---

### Step 2: Add `activeTabIdAtom` to global store

**File:** `frontend/app/store/global.ts`

Add a derived atom that reads `workspace.activetabid`, falling back to the first tab:

```ts
const activeTabIdAtom: Atom<string> = atom((get) => {
    const ws = get(workspaceAtom);
    if (!ws) return initOpts.tabId;
    return ws.activetabid || ws.pinnedtabids?.[0] || ws.tabids?.[0] || initOpts.tabId;
});
```

Export it alongside `staticTabId` (keep `staticTabId` for any code that truly needs the init-time value).

**Why this works:** The backend emits `waveobj:update` for workspace whenever `activetabid` changes. `WOS.getWaveObjectAtom` already subscribes to these — so `workspaceAtom` is already reactive. `activeTabIdAtom` is a pure derived atom that re-derives whenever workspace updates.

---

### Step 3: Wire `activeTabIdAtom` into workspace renderer

**File:** `frontend/app/workspace/workspace.tsx`

Change:
```tsx
const tabId = useAtomValue(atoms.staticTabId);
```
To:
```tsx
const tabId = useAtomValue(atoms.activeTabIdAtom);
```

`TabContent` already uses its `tabId` prop as a React key, so switching tabs will correctly unmount the old layout and mount the new one.

---

### Step 4: Write `tabbar.tsx`

**File:** `frontend/app/tab/tabbar.tsx` (new)

A focused, clean component — no drag-and-drop, no workspace switcher:

```
TabBar
  ├── scrollable tab row (OverlayScrollbars, already a dependency)
  │     ├── [pinned tabs]  (from workspace.pinnedtabids)
  │     ├── [pinned tab spacer] (if pinned tabs exist)
  │     └── [regular tabs] (from workspace.tabids)
  └── [+ new tab button]
```

Each tab uses the existing `Tab` component from `tab.tsx`.

Tab click → `setActiveTab(tabId)` (calls `WorkspaceService.SetActiveTab` via RPC — **no Tauri stub needed**).
Add button → `createTab()` (calls `WorkspaceService.CreateTab` via RPC).
Close → `WorkspaceService.CloseTab(wsId, tabId)` + `deleteLayoutModelForTab(tabId)` (from `@/layout/index`).

**File:** `frontend/app/tab/tabbar.scss` (new) — tab bar layout styles, self-contained.

---

### Step 5: Wire TabBar into WindowHeader

**File:** `frontend/app/window/window-header.tsx`

```tsx
// Before
<WindowDrag left />
<WindowDrag middle />   // filler
<SystemStatus />

// After
<WindowDrag left />
<TabBar workspace={workspace} />   // flex-grow, takes middle space
<SystemStatus />                   // ActionWidgets + window buttons — unchanged
```

`WindowHeader` already receives `workspace` as a prop.

---

### Step 6: Fix keyboard shortcuts

**File:** `frontend/app/store/keymodel.ts`

Three places reference `atoms.staticTabId` to get the current tab for close/cycle operations. Switch to `atoms.activeTabIdAtom`:

- Line ~150: `isTabPinned` check
- Line ~159: `closeTab` handler
- Line ~297: `switchTab` cycle

---

## File Change Summary

| File | Action |
|------|--------|
| `frontend/app/tab/widgetbar.tsx` | Delete |
| `frontend/app/tab/workspaceswitcher.tsx` + `.scss` | Delete |
| `frontend/app/tab/workspaceeditor.tsx` + `.scss` | Delete |
| `frontend/app/window/update-banner.tsx` | Delete |
| `frontend/app/window/window-header.scss` | Remove orphaned tab styles |
| `frontend/app/store/global.ts` | Add + export `activeTabIdAtom` |
| `frontend/app/workspace/workspace.tsx` | Use `activeTabIdAtom` |
| `frontend/app/tab/tabbar.tsx` | **New** — clean TabBar component |
| `frontend/app/tab/tabbar.scss` | **New** — tab bar styles |
| `frontend/app/window/window-header.tsx` | Add `<TabBar>`, remove `<WindowDrag middle>` |
| `frontend/app/store/keymodel.ts` | Use `activeTabIdAtom` in 3 places |

**Rust/Tauri changes:** None. Tab operations go through WebSocket RPC directly.

---

## What We Are NOT Doing

- No drag-and-drop reordering (can add later)
- No workspace switcher (use multiple windows instead)
- No tab background themes (can add later)
- No update banner in header (already handled in StatusBar)

---

## Status

- [ ] Step 1: Delete orphaned files
- [ ] Step 2: `activeTabIdAtom` in global store
- [ ] Step 3: `workspace.tsx` uses `activeTabIdAtom`
- [ ] Step 4: Write `tabbar.tsx` + `tabbar.scss`
- [ ] Step 5: Wire into `window-header.tsx`
- [ ] Step 6: Fix keyboard shortcuts in `keymodel.ts`
