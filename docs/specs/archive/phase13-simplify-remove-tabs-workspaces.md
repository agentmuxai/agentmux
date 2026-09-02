# Phase 13: Simplify - Remove Tabs & Workspaces (Temporary)

> **Status:** SPEC
> **Date:** 2026-02-08
> **Author:** AgentA
> **Priority:** HIGH
> **Target:** 0.18.7
> **Strategy:** Simplify now, restore complexity later

---

## Executive Summary

Instead of migrating the complex Electron multi-tab/workspace system to Tauri, **temporarily remove tabs and workspaces entirely**. Ship a simplified single-pane terminal application, then add back tabs/workspaces in a future release once the core Tauri migration is stable.

**Rationale:**
- ✅ Faster time to production Tauri build
- ✅ Eliminate most complex Electron → Tauri migration work
- ✅ Reduce testing surface area
- ✅ Focus on core terminal functionality first
- ✅ Can restore tabs/workspaces later with better architecture

**What We Keep:**
- ✅ Single terminal pane
- ✅ Backend (agentmuxsrv) unchanged
- ✅ Terminal rendering (xterm.js)
- ✅ AI chat integration
- ✅ File previews
- ✅ Settings/preferences

**What We Remove (Temporarily):**
- ❌ Multiple tabs
- ❌ Workspaces
- ❌ Tab bar UI
- ❌ Tab switching logic
- ❌ Workspace persistence
- ❌ Multi-WebContentsView management

---

## Current State Analysis

### Components to Remove

| Component | Purpose | LOC | Status |
|-----------|---------|-----|--------|
| **Electron Side** | | | |
| `emain/emain-tabview.ts` | WebContentsView per tab | ~400 | ❌ DELETE |
| `emain/emain-window.ts` | Window + tab coordination | ~600 | ⚠️ SIMPLIFY (keep window, remove tab logic) |
| **Frontend Side** | | | |
| `frontend/app/view/workspace.tsx` | Workspace container | ~300 | ⚠️ SIMPLIFY (single pane) |
| `frontend/app/view/tabbar.tsx` | Tab bar UI | ~200 | ❌ DELETE |
| `frontend/app/store/workspacestore.ts` | Workspace state | ~200 | ⚠️ SIMPLIFY (single pane state) |
| **Backend Side** | | | |
| `pkg/wstore/` | Workspace/tab persistence | N/A | ✅ KEEP (unused but harmless) |

### What Happens to Multi-Tab Users?

**Option A: Single Terminal Only**
- App opens with one terminal
- No tabs, no workspace switcher
- Clean, simple UX

**Option B: Multi-Window Instead**
- Users can open multiple windows (Cmd+N / Ctrl+N)
- Each window = one terminal
- OS manages "tabs" (Alt+Tab between windows)

**Recommendation: Option B** - Users can still work with multiple terminals, just in separate windows.

---

## Architecture Changes

### Before (Electron Multi-Tab)

```
┌─────────────────────────────────────────────────────────┐
│                  Electron Window                         │
│  ┌──────────────────────────────────────────────────┐  │
│  │ [Tab1] [Tab2] [Tab3] [+]                         │  │ ← Tab Bar
│  ├──────────────────────────────────────────────────┤  │
│  │                                                   │  │
│  │  WebContentsView #1 (Terminal)                   │  │ ← Active Tab
│  │  WebContentsView #2 (Hidden)                     │  │
│  │  WebContentsView #3 (Hidden)                     │  │
│  │                                                   │  │
│  └──────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

### After (Tauri Single Pane)

```
┌─────────────────────────────────────────────────────────┐
│                   Tauri Window #1                        │
│  ┌──────────────────────────────────────────────────┐  │
│  │                                                   │  │
│  │                                                   │  │
│  │          Single Webview (Terminal)               │  │
│  │                                                   │  │
│  │                                                   │  │
│  └──────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│                   Tauri Window #2                        │
│  ┌──────────────────────────────────────────────────┐  │
│  │          Single Webview (Terminal)               │  │
│  └──────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

**Multi-tasking via Multiple Windows:**
- File → New Window (Ctrl+Shift+N)
- Each window independent
- OS handles window switching (Alt+Tab)

---

## Implementation Plan

### Phase 1: Frontend Simplification (Day 1)

#### 1.1 Remove Tab Bar UI

```tsx
// frontend/app/view/workspace.tsx (BEFORE)
export const WorkspaceView: React.FC = () => {
  return (
    <div className="workspace">
      <TabBar tabs={tabs} activeId={activeId} />  ← DELETE THIS
      <TabContent tabId={activeTabId} />
    </div>
  );
};

// frontend/app/view/workspace.tsx (AFTER)
export const WorkspaceView: React.FC = () => {
  return (
    <div className="workspace">
      <TerminalView />  {/* Single terminal, no tabs */}
    </div>
  );
};
```

**Files to DELETE:**
- ❌ `frontend/app/view/tabbar.tsx`
- ❌ `frontend/app/view/tabbar.less`

**Files to MODIFY:**
- `frontend/app/view/workspace.tsx` - Remove tab logic, render single terminal
- `frontend/app/view/workspace.less` - Remove tab bar styles

---

#### 1.2 Simplify Workspace Store

```typescript
// frontend/app/store/workspacestore.ts (BEFORE)
export const workspaceAtom = atom<Workspace>({
  workspaceId: string;
  windowId: string;
  tabs: Tab[];           ← DELETE
  activeTabId: string;   ← DELETE
});

// frontend/app/store/workspacestore.ts (AFTER)
export const workspaceAtom = atom<Workspace>({
  windowId: string;
  // Single terminal block ID
  terminalBlockId: string;
});
```

**Acceptance Criteria:**
- ✅ No tab state in store
- ✅ Single terminal block tracked
- ✅ No workspace switching UI

---

#### 1.3 Update Terminal View

```tsx
// frontend/app/view/term/term.tsx (MODIFIED)

// BEFORE: Terminal tied to tab
<TerminalView blockId={activeTab.blockId} />

// AFTER: Terminal always visible
<TerminalView blockId={workspace.terminalBlockId} />
```

**Files to MODIFY:**
- `frontend/app/view/term/term.tsx` - Remove tab-related props
- `frontend/app/view/term/termwrap.ts` - Remove tab visibility logic

**Acceptance Criteria:**
- ✅ Terminal renders without tab context
- ✅ No tab switching logic
- ✅ Terminal always visible/focused

---

### Phase 2: Backend Simplification (Day 2)

#### 2.1 Update Backend Initialization

```go
// pkg/service/clientservice/clientservice.go (MODIFIED)

// BEFORE: Create default workspace with tabs
func EnsureInitialData() error {
    workspace := &waveobj.Workspace{
        Name: "default",
        TabIds: []string{"tab1", "tab2", "tab3"},  ← DELETE
    }
    // ...
}

// AFTER: Create single terminal block
func EnsureInitialData() error {
    // Create single terminal block, no workspace/tabs
    block := &waveobj.Block{
        BlockId: uuid.New().String(),
        BlockDef: &waveobj.BlockDef{
            Controller: "cmd",
            View: "term",
        },
    }
    wstore.CreateBlock(ctx, block)
    // ...
}
```

**Files to MODIFY:**
- `pkg/service/clientservice/clientservice.go` - Remove workspace/tab creation
- `pkg/wcore/wcore.go` - Simplify window initialization

**Acceptance Criteria:**
- ✅ Backend creates single terminal block
- ✅ No workspace/tab persistence
- ✅ Single block ID returned to frontend

---

#### 2.2 Clean Up Unused RPC Endpoints

**Keep these (still needed):**
- ✅ `CreateBlock` - Create terminal
- ✅ `DeleteBlock` - Close terminal
- ✅ `GetBlock` - Get terminal state

**Mark as deprecated (but don't delete yet):**
- ⏳ `CreateTab` - Not used by frontend
- ⏳ `CloseTab` - Not used by frontend
- ⏳ `SetActiveTab` - Not used by frontend
- ⏳ `GetWorkspace` - Not used by frontend

**Rationale:** Keep backend endpoints for now (harmless), remove later when we restore tabs.

---

### Phase 3: Electron Cleanup (Day 3)

#### 3.1 Remove Tab View Management

```typescript
// emain/emain-tabview.ts - DELETE ENTIRE FILE

// emain/emain-window.ts (MODIFIED)

// BEFORE: Manage multiple WebContentsView
export function setActiveTab(windowId: string, tabId: string) {
  // Hide all views, show target view
}

// AFTER: Single view, no tab management
export function createWindow() {
  const win = new BrowserWindow({
    width: 1200,
    height: 800,
    webPreferences: { preload: preloadPath }
  });

  // Single webview, no child views
  win.loadURL(indexPath);
}
```

**Files to DELETE:**
- ❌ `emain/emain-tabview.ts`

**Files to MODIFY:**
- `emain/emain-window.ts` - Remove tab coordination logic
- `emain/preload.ts` - Remove tab IPC handlers

**Acceptance Criteria:**
- ✅ No WebContentsView per tab
- ✅ Single webview per window
- ✅ No IPC for tab management

---

#### 3.2 Simplify Preload API

```typescript
// emain/preload.ts (MODIFIED)

// BEFORE: 40+ APIs including tab management
contextBridge.exposeInMainWorld('api', {
  createTab: (url) => ipcRenderer.invoke('create-tab', url),     ← DELETE
  closeTab: (id) => ipcRenderer.invoke('close-tab', id),         ← DELETE
  setActiveTab: (id) => ipcRenderer.invoke('set-active-tab', id),← DELETE
  getWorkspace: () => ipcRenderer.invoke('get-workspace'),       ← DELETE
  // ... 10+ tab-related APIs
});

// AFTER: Simplified API (no tabs)
contextBridge.exposeInMainWorld('api', {
  getTerminalBlock: () => ipcRenderer.invoke('get-terminal-block'),
  // ... other non-tab APIs
});
```

**Files to MODIFY:**
- `emain/preload.ts` - Remove tab IPC methods
- `frontend/types/custom.d.ts` - Remove ElectronApi tab types

---

### Phase 4: Multi-Window Support (Day 4)

#### 4.1 Add "New Window" Menu Item

```typescript
// emain/menu.ts (MODIFIED)

const menu = Menu.buildFromTemplate([
  {
    label: 'File',
    submenu: [
      {
        label: 'New Window',           // ← ADD THIS
        accelerator: 'CmdOrCtrl+Shift+N',
        click: () => createNewWindow()
      },
      { type: 'separator' },
      { label: 'Close Window', role: 'close' }
    ]
  }
]);
```

**Files to MODIFY:**
- `emain/menu.ts` - Add "New Window" menu item
- `emain/emain-window.ts` - Add `createNewWindow()` function

**Acceptance Criteria:**
- ✅ File → New Window creates second window
- ✅ Each window independent (separate backend block)
- ✅ Keyboard shortcut works (Ctrl+Shift+N)

---

#### 4.2 Independent Window State

**Challenge:** Each window needs its own terminal block.

```typescript
// emain/emain-window.ts

export function createNewWindow() {
  const blockId = uuid.v4();  // Generate new block ID

  const win = new BrowserWindow({ /* ... */ });

  // Pass blockId to frontend via query param
  win.loadURL(`${indexPath}?blockId=${blockId}`);

  windows.set(win.id, { windowId: win.id, blockId });
}
```

**Frontend:**
```typescript
// frontend/wave.ts

const urlParams = new URLSearchParams(window.location.search);
const blockId = urlParams.get('blockId');

// Use blockId to fetch terminal state from backend
initializeTerminal(blockId);
```

**Acceptance Criteria:**
- ✅ Each window has unique terminal block
- ✅ Windows don't interfere with each other
- ✅ Close window → cleanup backend block

---

### Phase 5: UI Polish (Day 5)

#### 5.1 Update Window Title

```typescript
// frontend/app/view/term/term.tsx

useEffect(() => {
  // Update window title with current directory
  const cwd = terminalState.cwd || '~';
  document.title = `AgentMux - ${cwd}`;
}, [terminalState.cwd]);
```

#### 5.2 Remove Workspace Switcher

**Files to DELETE:**
- ❌ `frontend/app/view/workspaceswitcher.tsx`
- ❌ `frontend/app/view/workspaceswitcher.less`

#### 5.3 Simplify Settings

**Remove from settings UI:**
- ❌ "Default workspace layout"
- ❌ "Tab bar position"
- ❌ "Auto-save workspace"

**Keep:**
- ✅ Terminal font/size
- ✅ Theme
- ✅ Keyboard shortcuts
- ✅ AI settings

---

## Migration Path for Existing Users

### Option A: Destructive Migration

**On first launch of simplified version:**
1. Detect old workspace/tab data in DB
2. Show dialog: "AgentMux has been simplified. Your workspace/tabs will be restored in a future update."
3. Delete workspace/tab data from DB
4. Create single terminal block

**Pros:**
- Clean slate
- No migration complexity

**Cons:**
- Users lose tab layouts
- May frustrate power users

---

### Option B: Preserve Data (But Don't Use It)

**On first launch:**
1. Keep workspace/tab data in DB (don't delete)
2. Create new "single terminal" block
3. Ignore old workspace/tab data
4. When tabs return in future version, restore from preserved data

**Pros:**
- Non-destructive
- Users don't lose data

**Cons:**
- Unused data in DB
- Slightly more complex

**Recommendation: Option B** - Preserve data for future restoration.

---

## Testing Strategy

### Unit Tests

```typescript
// frontend/app/view/__tests__/workspace.test.tsx

describe('WorkspaceView (Simplified)', () => {
  test('renders single terminal', () => {
    render(<WorkspaceView />);
    expect(screen.getByTestId('terminal-view')).toBeInTheDocument();
  });

  test('no tab bar rendered', () => {
    render(<WorkspaceView />);
    expect(screen.queryByTestId('tab-bar')).not.toBeInTheDocument();
  });

  test('no workspace switcher', () => {
    render(<WorkspaceView />);
    expect(screen.queryByTestId('workspace-switcher')).not.toBeInTheDocument();
  });
});
```

### Integration Tests

```typescript
// test/e2e/simplified.e2e.test.ts

test('single terminal on startup', async () => {
  const app = await launchApp();
  const terminalCount = await app.$$('[data-testid="terminal-view"]');
  expect(terminalCount).toHaveLength(1);
});

test('new window creates independent terminal', async () => {
  const app = await launchApp();

  // Open second window
  await app.keyboard.press('Control+Shift+N');
  const windows = await app.windows();
  expect(windows).toHaveLength(2);

  // Each window has its own terminal
  const term1 = await windows[0].$('[data-testid="terminal-view"]');
  const term2 = await windows[1].$('[data-testid="terminal-view"]');
  expect(term1).toBeTruthy();
  expect(term2).toBeTruthy();
});
```

---

## Rollback Plan

**If simplification causes major issues:**

1. **Revert Git Commits**
   ```bash
   git revert <simplification-commits>
   git push origin main
   ```

2. **Restore Tab/Workspace Code**
   - Restore deleted files from git history
   - Re-enable tab bar UI
   - Re-enable workspace persistence

3. **Database Migration**
   - Restore workspace/tab data from backup
   - Re-create tab blocks in backend

**Risk:** Low - simplified version has less code, fewer bugs expected.

---

## Future Restoration Plan

**When to restore tabs/workspaces:**
- Tauri migration stable (6+ months)
- User feedback requests tabs
- Team capacity available

**How to restore:**
1. Use `workspace-tab-modernization.md` spec
2. Implement frontend-managed tabs (not Electron WebContentsView)
3. Restore from preserved DB data (if Option B chosen)
4. Gradual rollout (beta flag, then production)

---

## Success Criteria

### Must Have (P0)

- ✅ Single terminal renders on startup
- ✅ No tab bar visible
- ✅ No workspace switcher
- ✅ Terminal functional (input/output works)
- ✅ New window creates independent terminal
- ✅ Close window cleans up backend block

### Should Have (P1)

- ✅ Window title updates with cwd
- ✅ Settings simplified (remove tab/workspace options)
- ✅ Existing data preserved for future restoration

### Nice to Have (P2)

- ⏳ Migration dialog explaining simplification
- ⏳ "Tabs coming soon" banner in UI

---

## Timeline

| Task | Duration | Assignee |
|------|----------|----------|
| **Phase 1:** Frontend simplification | 1 day | AgentA |
| **Phase 2:** Backend simplification | 1 day | AgentA |
| **Phase 3:** Electron cleanup | 1 day | AgentA |
| **Phase 4:** Multi-window support | 1 day | AgentA |
| **Phase 5:** UI polish | 1 day | AgentA |
| **Testing & QA** | 1 day | AgentA |
| **Total** | **6 days** | |

**Target Release:** 0.18.7

---

## Metrics

### Code Deletion (Expected)

| Category | Files Deleted | LOC Removed |
|----------|---------------|-------------|
| Frontend | 5 files | ~800 LOC |
| Backend | 0 files | ~0 LOC (endpoints kept but unused) |
| Electron | 2 files | ~600 LOC |
| **Total** | **7 files** | **~1400 LOC** |

**Result:** Smaller, simpler codebase.

---

## Communication Plan

### User-Facing Messaging

**Release Notes (0.18.7):**
```markdown
## AgentMux 0.18.7 - Simplified Release

To accelerate the Tauri migration and improve stability, we've temporarily
simplified AgentMux to focus on core terminal functionality.

**What's Changed:**
- ✅ Faster, more stable terminal experience
- ✅ Multi-window support (Ctrl+Shift+N for new window)
- ⚠️ Tabs and workspaces temporarily removed

**What's Coming Back:**
- 📅 Tabs and workspaces will return in a future release (Q2 2026)
- 💾 Your existing workspace data is preserved

**Why This Change?**
This simplification allows us to ship a production-ready Tauri build faster,
with better performance and stability. Once the core is solid, we'll restore
tabs and workspaces with an improved architecture.
```

---

## Appendix: Files Changed

### Files to DELETE

```
frontend/app/view/tabbar.tsx              (~200 LOC)
frontend/app/view/tabbar.less             (~100 LOC)
frontend/app/view/workspaceswitcher.tsx   (~150 LOC)
frontend/app/view/workspaceswitcher.less  (~50 LOC)
emain/emain-tabview.ts                    (~400 LOC)
```

### Files to MODIFY (Major Changes)

```
frontend/app/view/workspace.tsx           (Simplify to single terminal)
frontend/app/store/workspacestore.ts      (Remove tab state)
frontend/app/view/term/term.tsx           (Remove tab context)
emain/emain-window.ts                     (Remove tab coordination)
emain/preload.ts                          (Remove tab IPC)
emain/menu.ts                             (Add "New Window" item)
```

### Files to MODIFY (Minor Changes)

```
frontend/types/custom.d.ts                (Remove ElectronApi tab types)
pkg/service/clientservice/clientservice.go (Simplify initialization)
```

---

**END OF SPEC**

---

## Questions for Discussion

1. **Should we preserve workspace/tab data for future restoration?** (Recommended: Yes)
2. **Should we show a migration dialog to users?** (Recommended: Yes, brief explanation)
3. **Should we add a "Coming Soon: Tabs" banner in UI?** (Optional)
4. **What's the minimum viable multi-window support?** (File → New Window sufficient?)

---

**Next Steps:**

1. ✅ Review this spec with team
2. ⏳ Get user approval for simplification strategy
3. ⏳ Begin Phase 1 implementation
4. ⏳ Monitor user feedback after release
