# AgentMux Workspace & Tab System Modernization

> **Status:** SPEC
> **Date:** 2026-02-08
> **Author:** AgentA
> **Priority:** HIGH
> **Target:** 0.18.x series

---

## Executive Summary

AgentMux currently uses an Electron-based multi-WebContentsView architecture where each tab runs in a separate Chromium renderer process. With the Tauri v2 migration, this architecture is incompatible - Tauri provides **one webview per window**, requiring a complete redesign of the workspace and tab system to be **frontend-managed** instead of main-process-managed.

**Current State:**
- ❌ Electron WebContentsView per tab (not available in Tauri)
- ❌ Main process manages tab lifecycle
- ❌ Heavy IPC overhead for tab operations
- ❌ Each tab = separate renderer process (~50-100MB each)

**Target State:**
- ✅ Single webview with frontend-managed tab switching
- ✅ React components handle tab lifecycle
- ✅ Lightweight tab switching (virtual DOM)
- ✅ Shared memory across all tabs (~30-50MB total)

**Key Benefits:**
- **Performance:** Tab switching from ~200ms → <50ms
- **Memory:** 5-10x reduction in memory footprint
- **Simplicity:** Remove complex IPC for tab management
- **Flexibility:** Easier to implement drag-drop, split panes, etc.

---

## Current Architecture (Electron)

### Tab Management Flow

```
┌─────────────────────────────────────────────────────────┐
│            Electron Main Process                         │
│                                                          │
│  emain/emain-tabview.ts                                 │
│  ├─ createTab(windowId)                                 │
│  │   ├─ new WebContentsView()  ← Separate Chromium     │
│  │   ├─ webContents.loadURL()                          │
│  │   └─ browserWindow.contentView.addChildView()       │
│  │                                                      │
│  ├─ setActiveTab(windowId, tabId)                      │
│  │   ├─ Hide all WebContentsView                       │
│  │   └─ Show target WebContentsView                    │
│  │                                                      │
│  └─ destroyTab(tabId)                                  │
│      └─ webContents.destroy()                          │
└──────────────────┬──────────────────────────────────────┘
                   │ IPC (invoke/send)
                   ▼
┌─────────────────────────────────────────────────────────┐
│         React Frontend (Single WebContentsView)         │
│                                                          │
│  frontend/app/view/workspace.tsx                        │
│  └─ Renders tab bar UI only (not content)              │
│                                                          │
│  User clicks tab → IPC to main → main shows/hides view │
└─────────────────────────────────────────────────────────┘
```

### Key Files

| File | Purpose | LOC | Migration Status |
|------|---------|-----|------------------|
| `emain/emain-tabview.ts` | WebContentsView management | ~400 | ❌ **Must rewrite** |
| `emain/emain-window.ts` | Window + tab coordination | ~600 | ⚠️ **Partial rewrite** |
| `frontend/app/view/workspace.tsx` | Workspace UI | ~300 | ✅ **Expand functionality** |
| `frontend/app/store/workspacestore.ts` | State management | ~200 | ✅ **Already frontend-only** |

### Problems with Current Design

1. **IPC Overhead**
   - Every tab switch requires IPC round-trip
   - Main process must coordinate visibility of multiple views
   - ~50-200ms latency per tab switch

2. **Memory Waste**
   - Each tab = separate Chromium renderer (50-100MB)
   - Duplicate React runtime per tab
   - No shared state/cache between tabs

3. **Tauri Incompatibility**
   - No WebContentsView equivalent in Tauri
   - Single webview per window (cannot have multiple child views)

4. **Complexity**
   - Split logic between main process and frontend
   - Hard to debug (IPC boundaries)
   - Difficult to add features (drag-drop, split panes)

---

## Target Architecture (Tauri + Frontend Tabs)

### New Tab Management Flow

```
┌─────────────────────────────────────────────────────────┐
│               Tauri Rust Backend (Minimal)               │
│                                                          │
│  src-tauri/src/commands/workspace.rs                    │
│  └─ #[tauri::command]                                   │
│      └─ get_workspace_state() → JSON                    │
│          (Read-only, no tab lifecycle management)       │
└──────────────────┬──────────────────────────────────────┘
                   │ Tauri invoke (async)
                   ▼
┌─────────────────────────────────────────────────────────┐
│          React Frontend (Single Webview)                │
│                                                          │
│  frontend/app/view/workspace.tsx  ← **EXPANDED**        │
│  ├─ <WorkspaceView>                                     │
│  │   ├─ <TabBar tabs={allTabs} active={activeTabId} /> │
│  │   └─ <TabContent>                                    │
│  │       └─ {activeTab.type === 'terminal' ?           │
│  │              <TerminalView /> : <PreviewView />}     │
│  │                                                      │
│  ├─ State: workspaceStore (Jotai atoms)                │
│  │   ├─ allTabs: Tab[]                                 │
│  │   ├─ activeTabId: string                            │
│  │   └─ tabCache: Map<string, TabState>               │
│  │                                                      │
│  └─ Tab Switching Logic (frontend-only)                │
│      ├─ Click tab → Update activeTabId atom            │
│      ├─ Render only active tab content                 │
│      └─ Cache inactive tab state (virtual DOM)         │
└─────────────────────────────────────────────────────────┘
```

### Key Changes

| Component | Before (Electron) | After (Tauri) |
|-----------|-------------------|---------------|
| **Tab creation** | Main process creates WebContentsView | Frontend creates React component |
| **Tab switching** | IPC → main hides/shows views | Frontend updates state → React re-renders |
| **Tab content** | Separate renderer per tab | Single renderer, virtual DOM caching |
| **State persistence** | Main process + backend DB | Frontend state + backend DB sync |
| **Memory per tab** | 50-100MB (full renderer) | ~5-10MB (React component) |

---

## Implementation Plan

### Phase 1: Frontend Tab Container (Week 1)

**Goal:** Render all tab content in a single webview with React-managed switching.

#### 1.1 Update Workspace Store

```typescript
// frontend/app/store/workspacestore.ts

// Current (simplified)
export const workspaceAtom = atom<Workspace>({
  workspaceId: string;
  windowId: string;
  tabs: Tab[];  // Metadata only (id, name, type)
});

// New (expanded)
export const workspaceAtom = atom<Workspace>({
  workspaceId: string;
  windowId: string;
  tabs: Tab[];
  activeTabId: string;
  tabStates: Map<string, TabState>;  // Cache rendered state
  splitPanes?: PaneLayout;           // Future: split panes
});

export interface TabState {
  tabId: string;
  type: 'terminal' | 'preview' | 'editor';
  content: TerminalState | PreviewState | EditorState;
  scrollPos: { x: number; y: number };
  lastActive: number;
}
```

**Files to modify:**
- `frontend/app/store/workspacestore.ts` - Add tab state caching
- `frontend/types/gotypes.ts` - Add TabState interface

**Acceptance Criteria:**
- ✅ Store maintains state for all tabs (active + inactive)
- ✅ Tab switch updates activeTabId atom
- ✅ Inactive tab state cached (no re-render)

---

#### 1.2 Refactor WorkspaceView Component

```tsx
// frontend/app/view/workspace.tsx (NEW ARCHITECTURE)

export const WorkspaceView: React.FC = () => {
  const workspace = useAtomValue(workspaceAtom);
  const [activeTabId, setActiveTabId] = useAtom(activeTabIdAtom);
  const activeTab = workspace.tabs.find(t => t.id === activeTabId);

  return (
    <div className="workspace-container">
      <TabBar
        tabs={workspace.tabs}
        activeTabId={activeTabId}
        onTabClick={(id) => setActiveTabId(id)}
        onTabClose={(id) => closeTab(id)}
        onTabDragDrop={(fromId, toId) => reorderTabs(fromId, toId)}
      />

      <div className="tab-content-area">
        {/* Render ONLY active tab (other tabs cached in virtual DOM) */}
        {activeTab && <TabContentRenderer tab={activeTab} />}
      </div>
    </div>
  );
};

const TabContentRenderer: React.FC<{ tab: Tab }> = ({ tab }) => {
  switch (tab.type) {
    case 'terminal':
      return <TerminalView blockId={tab.blockId} />;
    case 'preview':
      return <PreviewView blockId={tab.blockId} />;
    case 'editor':
      return <EditorView blockId={tab.blockId} />;
    default:
      return <div>Unknown tab type</div>;
  }
};
```

**Files to modify:**
- `frontend/app/view/workspace.tsx` - Complete rewrite
- `frontend/app/view/tabbar.tsx` - Extract into separate component
- `frontend/app/view/tabcontent.tsx` - New file for tab rendering logic

**Acceptance Criteria:**
- ✅ Single WorkspaceView renders all tabs
- ✅ Tab switching happens without IPC
- ✅ Inactive tabs don't re-render on switch
- ✅ Tab bar shows all tabs with visual active state

---

#### 1.3 Terminal View State Management

**Challenge:** xterm.js instances must be cached when tab is inactive, not destroyed.

```typescript
// frontend/app/view/term/termwrap.ts (MODIFIED)

export class TermWrap {
  terminal: Terminal;
  fitAddon: FitAddon;
  isVisible: boolean = true;

  // NEW: Pause rendering when tab inactive
  setVisibility(visible: boolean) {
    this.isVisible = visible;
    if (visible) {
      this.terminal.focus();
      this.fitAddon.fit();
    } else {
      // Detach xterm from DOM but keep in memory
      this.terminal.blur();
    }
  }

  // NEW: Serialize state for caching
  serializeState(): TerminalState {
    return {
      scrollPos: this.terminal.buffer.active.viewportY,
      history: this.terminal.buffer.active.length,
      // ... other state
    };
  }
}
```

**Files to modify:**
- `frontend/app/view/term/termwrap.ts` - Add visibility + state serialization
- `frontend/app/view/term/term.tsx` - Connect to workspace state

**Acceptance Criteria:**
- ✅ Terminal state persists when switching tabs
- ✅ Scroll position maintained
- ✅ xterm.js instances cached (not destroyed)
- ✅ Terminal focus/blur on tab switch

---

### Phase 2: Remove Main Process Tab Logic (Week 2)

**Goal:** Delete Electron-specific tab management, migrate to Tauri commands.

#### 2.1 Delete Electron Tab Management

**Files to DELETE:**
- ❌ `emain/emain-tabview.ts` (~400 LOC)
- ❌ `emain/emain-window.ts` (partial, ~200 LOC related to tabs)

**Files to MODIFY:**
- `emain/preload.ts` - Remove tab-related IPC handlers
- `frontend/types/custom.d.ts` - Remove ElectronApi tab methods

**Removed APIs:**
```typescript
// DELETE from window.api
window.api.createTab(windowId, url);     // ❌
window.api.closeTab(tabId);              // ❌
window.api.setActiveTab(windowId, tabId); // ❌
window.api.getTabById(tabId);            // ❌ (move to backend RPC)
```

---

#### 2.2 Add Tauri Workspace Commands

```rust
// src-tauri/src/commands/workspace.rs (NEW FILE)

#[tauri::command]
pub async fn get_workspace(workspace_id: String) -> Result<Workspace, String> {
    // Query backend via HTTP/WebSocket
    let workspace = backend_client::get_workspace(&workspace_id).await?;
    Ok(workspace)
}

#[tauri::command]
pub async fn update_tab_layout(
    workspace_id: String,
    tabs: Vec<TabLayout>
) -> Result<(), String> {
    // Persist tab order/layout to backend
    backend_client::update_tab_layout(&workspace_id, tabs).await?;
    Ok(())
}
```

**Files to CREATE:**
- `src-tauri/src/commands/workspace.rs` - Workspace Tauri commands
- `src-tauri/src/backend_client.rs` - HTTP client to agentmuxsrv

**Files to MODIFY:**
- `src-tauri/src/lib.rs` - Register workspace commands
- `frontend/util/tauri-api.ts` - Add Tauri workspace methods

---

#### 2.3 Frontend → Backend State Sync

**Pattern:** Frontend is source of truth for UI state, backend persists to DB.

```typescript
// frontend/app/store/workspacestore.ts

// Auto-sync tab layout changes to backend
useEffect(() => {
  const unsubscribe = workspaceAtom.subscribe((workspace) => {
    // Debounce: only sync every 2 seconds or on explicit save
    debouncedSyncToBackend(workspace);
  });
  return unsubscribe;
}, []);

async function syncToBackend(workspace: Workspace) {
  await invoke('update_tab_layout', {
    workspaceId: workspace.workspaceId,
    tabs: workspace.tabs.map(t => ({
      id: t.id,
      name: t.name,
      order: t.order,
      // ... layout metadata
    }))
  });
}
```

**Acceptance Criteria:**
- ✅ Tab changes auto-sync to backend (debounced)
- ✅ Backend persists to SQLite (wstore)
- ✅ Workspace state restored on app restart

---

### Phase 3: Advanced Tab Features (Week 3)

#### 3.1 Drag-Drop Tab Reordering

**Goal:** Drag tabs to reorder in tab bar.

**Libraries:**
- `@dnd-kit/core` - React drag-drop (already used in AgentMux?)
- `@dnd-kit/sortable` - List reordering

```tsx
// frontend/app/view/tabbar.tsx

import { DndContext, closestCenter } from '@dnd-kit/core';
import { SortableContext, horizontalListSortingStrategy } from '@dnd-kit/sortable';

export const TabBar: React.FC = ({ tabs, onReorder }) => {
  const handleDragEnd = (event) => {
    const { active, over } = event;
    if (active.id !== over.id) {
      onReorder(active.id, over.id);
    }
  };

  return (
    <DndContext onDragEnd={handleDragEnd}>
      <SortableContext items={tabs} strategy={horizontalListSortingStrategy}>
        {tabs.map(tab => <SortableTab key={tab.id} tab={tab} />)}
      </SortableContext>
    </DndContext>
  );
};
```

**Acceptance Criteria:**
- ✅ Drag tab left/right to reorder
- ✅ Visual feedback during drag
- ✅ Order persisted to backend

---

#### 3.2 Split Panes (Future)

**Goal:** Split workspace into multiple panes, each with independent tab sets.

**Architecture:**
```typescript
export interface PaneLayout {
  type: 'horizontal' | 'vertical';
  children: Array<PaneLayout | PaneLeaf>;
  sizes: number[];  // Relative sizes (sum = 1.0)
}

export interface PaneLeaf {
  type: 'leaf';
  tabs: Tab[];
  activeTabId: string;
}
```

**UI:**
```
┌──────────────────────────────────────┐
│ Tab1 | Tab2 | Tab3              [+] │  ← Pane 1 (tabs)
├──────────────────────────────────────┤
│ Tab4 | Tab5                      [+] │  ← Pane 2 (tabs)
└──────────────────────────────────────┘
```

**Deferred to Phase 4** (not part of this spec).

---

## Testing Strategy

### Unit Tests

```typescript
// frontend/app/view/__tests__/workspace.test.tsx

describe('WorkspaceView', () => {
  test('renders all tabs in tab bar', () => {
    const tabs = [
      { id: '1', name: 'Terminal 1', type: 'terminal' },
      { id: '2', name: 'Preview', type: 'preview' },
    ];
    render(<WorkspaceView workspace={{ tabs }} />);
    expect(screen.getByText('Terminal 1')).toBeInTheDocument();
    expect(screen.getByText('Preview')).toBeInTheDocument();
  });

  test('switches active tab on click', () => {
    const { getByText } = render(<WorkspaceView />);
    fireEvent.click(getByText('Tab2'));
    expect(store.get(activeTabIdAtom)).toBe('tab2-id');
  });

  test('caches inactive tab state', () => {
    // Switch from Tab1 → Tab2 → Tab1
    // Verify Tab1 state unchanged (scroll position, terminal history)
  });
});
```

### Integration Tests

```typescript
// test/e2e/workspace.e2e.test.ts

test('tab switching performance', async () => {
  const start = performance.now();
  await page.click('[data-tab-id="tab2"]');
  await page.waitForSelector('[data-tab-content="tab2"]');
  const duration = performance.now() - start;
  expect(duration).toBeLessThan(50); // <50ms tab switch
});

test('tab state persists across restarts', async () => {
  // Create tab, write terminal history
  // Restart app
  // Verify tab still exists with same content
});
```

---

## Migration Checklist

### Pre-Migration (Electron)

- [x] Electron WebContentsView architecture
- [x] Tab management in main process
- [x] Heavy IPC for tab operations

### Phase 1: Frontend Tabs ✅

- [ ] Expand workspaceStore with tab state caching
- [ ] Rewrite WorkspaceView to render all tabs
- [ ] Implement TabContentRenderer with conditional rendering
- [ ] Add terminal visibility/state serialization
- [ ] Test: Tab switching without IPC
- [ ] Test: Terminal state persists on switch

### Phase 2: Remove Electron Logic ✅

- [ ] Delete `emain/emain-tabview.ts`
- [ ] Remove tab IPC handlers from preload
- [ ] Add Tauri workspace commands
- [ ] Implement frontend → backend state sync
- [ ] Test: Workspace state saves to backend
- [ ] Test: Workspace restores on app restart

### Phase 3: Advanced Features ✅

- [ ] Implement drag-drop tab reordering
- [ ] Add keyboard shortcuts (Ctrl+Tab, Ctrl+W)
- [ ] Add tab context menu (close, rename, duplicate)
- [ ] Test: Drag-drop reordering
- [ ] Test: Keyboard navigation

---

## Performance Targets

| Metric | Current (Electron) | Target (Tauri) |
|--------|-------------------|----------------|
| **Tab switch latency** | 50-200ms | <50ms |
| **Memory per tab** | 50-100MB | 5-10MB |
| **Tab creation time** | 200-500ms | <100ms |
| **Max tabs before lag** | ~20 tabs | >100 tabs |

---

## Risks & Mitigations

### Risk 1: xterm.js Performance with Many Tabs

**Issue:** Keeping 50+ xterm.js instances in memory may cause lag.

**Mitigation:**
- Serialize inactive terminal state to JSON
- Destroy xterm instances after 5 minutes of inactivity
- Restore from serialized state on reactivation

### Risk 2: State Sync Race Conditions

**Issue:** Frontend and backend state may diverge if sync fails.

**Mitigation:**
- Backend is source of truth on startup (fetch from DB)
- Frontend optimistic updates with rollback on error
- Periodic background sync every 30 seconds

### Risk 3: Complex State Management

**Issue:** Frontend state management becomes complex with many tabs.

**Mitigation:**
- Use Jotai atoms for granular reactivity
- Normalize state (Map<tabId, TabState> instead of array)
- Add Redux DevTools for debugging

---

## Success Criteria

### Must Have (P0)

- ✅ All tabs render in single webview
- ✅ Tab switching <50ms latency
- ✅ Terminal state persists on tab switch
- ✅ Tab order/layout persists on restart
- ✅ No IPC for tab switching

### Should Have (P1)

- ✅ Drag-drop tab reordering
- ✅ Keyboard shortcuts (Ctrl+Tab, Ctrl+W)
- ✅ Tab context menu

### Nice to Have (P2)

- ⏳ Split panes (deferred to Phase 4)
- ⏳ Tab groups/favorites
- ⏳ Tab search/filter

---

## Timeline

| Phase | Duration | Completion Date |
|-------|----------|-----------------|
| **Phase 1:** Frontend Tab Container | 1 week | TBD |
| **Phase 2:** Remove Electron Logic | 1 week | TBD |
| **Phase 3:** Advanced Features | 1 week | TBD |
| **Total** | **3 weeks** | **TBD** |

---

## Appendix: Code Snippets

### A. Current Electron Tab Creation (DELETE)

```typescript
// emain/emain-tabview.ts (TO DELETE)

export function createTab(windowId: string, url: string) {
  const view = new WebContentsView({
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: true,
    }
  });

  view.webContents.loadURL(url);

  const window = BrowserWindow.fromId(windowId);
  window.contentView.addChildView(view);

  tabs.set(view.webContents.id, { view, windowId, url });
}
```

### B. New React Tab Rendering (CREATE)

```tsx
// frontend/app/view/workspace.tsx (NEW)

export const WorkspaceView: React.FC = () => {
  const workspace = useAtomValue(workspaceAtom);
  const activeTabId = useAtomValue(activeTabIdAtom);

  return (
    <div className="workspace">
      <TabBar tabs={workspace.tabs} activeId={activeTabId} />
      <TabContent tabId={activeTabId} />
    </div>
  );
};
```

---

**END OF SPEC**
