# Spec: Remove AI Panel Sidebar

**Status:** Ready for implementation
**Date:** 2026-03-09

## Problem

The AI panel sidebar (`frontend/app/aipanel/`) is a legacy Wave Terminal feature — a built-in AI chat assistant rendered in a resizable left panel. AgentMux doesn't use it. The agent pane (`view/agent/`) handles all AI agent interactions through external CLIs. The aipanel adds 2,575 lines of dead code and its tentacles reach into the workspace layout, focus manager, keyboard shortcuts, and RPC layer.

## Goal

Completely remove `aipanel/` and all references to it. Simplify the workspace layout to remove the resizable left panel. Remove AI panel keyboard shortcuts and focus management code.

## Impact Analysis

### Files to DELETE (entire files)

| File | Lines | Purpose |
|------|-------|---------|
| `frontend/app/aipanel/agentai-model.tsx` | 350 | Singleton AI chat model |
| `frontend/app/aipanel/aipanel.tsx` | 494 | Main panel component |
| `frontend/app/aipanel/aipanelinput.tsx` | 174 | Input textarea |
| `frontend/app/aipanel/aipanelmessages.tsx` | 66 | Message list |
| `frontend/app/aipanel/aipanelheader.tsx` | 87 | Panel header |
| `frontend/app/aipanel/aimessage.tsx` | 258 | Message renderer |
| `frontend/app/aipanel/aitooluse.tsx` | 296 | Tool use renderer |
| `frontend/app/aipanel/aidroppedfiles.tsx` | 62 | Dropped files display |
| `frontend/app/aipanel/aifeedbackbuttons.tsx` | 97 | Feedback buttons |
| `frontend/app/aipanel/airatelimitstrip.tsx` | 145 | Rate limit strip |
| `frontend/app/aipanel/telemetryrequired.tsx` | 93 | Telemetry consent |
| `frontend/app/aipanel/agentai-focus-utils.ts` | 58 | Focus utilities |
| `frontend/app/aipanel/ai-utils.ts` | 340 | File validation utils |
| `frontend/app/aipanel/aitypes.ts` | 55 | Type definitions |
| **Total** | **2,575** | |

### Files to MODIFY

#### 1. `frontend/app/workspace/workspace.tsx`

**Remove:**
- Import of `WaveAIModel` and `AIPanel`
- Import of `react-resizable-panels` (`Panel`, `PanelGroup`, `PanelResizeHandle`, imperative refs)
- `isAIPanelVisible` atom subscription
- `panelGroupRef` and `aiPanelRef` refs
- `useEffect` for panel expand/collapse
- `useEffect` for AI focus after visibility change
- The entire `PanelGroup` wrapper with resizable panels

**Replace with:** Simple layout without resizable panels — just render `<TabContent>` directly.

```tsx
// BEFORE (with AI panel)
<PanelGroup direction="horizontal" ref={panelGroupRef}>
    <Panel ref={aiPanelRef} collapsible ...>
        <AIPanel onClose={...} />
    </Panel>
    <PanelResizeHandle />
    <Panel order={2}>
        <TabContent ... />
    </Panel>
</PanelGroup>

// AFTER (no AI panel)
<TabContent key={tabId} tabId={tabId} />
```

#### 2. `frontend/app/workspace/workspace-layout-model.ts`

**This entire class may become unnecessary.** It exists solely to manage AI panel state:
- `panelVisibleAtom` — AI panel visibility
- `aiPanelWidth` — AI panel width
- `getAIPanelVisible()` / `setAIPanelVisible()` — toggle
- `captureResize()` / `onCollapsed()` / `onExpanded()` — panel resize
- `handleWindowResize()` — resize proportions
- Tab meta keys: `waveai:panelopen`, `waveai:panelwidth`
- `setWaveAIOpen()` Tauri API call

**Action:** Delete the file entirely, or gut it down to just `handleWindowResize` if needed elsewhere. Check if any other code references `WorkspaceLayoutModel` for non-AI-panel purposes.

#### 3. `frontend/app/store/focusManager.ts`

**Remove:**
- Import of `waveAIHasFocusWithin` from aipanel
- Import of `WaveAIModel` from aipanel
- `FocusStrType` union: remove `"waveai"` option, keep only `"node"`
- `setWaveAIFocused()` method
- `waveAIFocusWithin()` method
- `requestWaveAIFocus()` method
- In `refocusNode()`: remove the `waveai` branch that calls `WaveAIModel.getInstance().focusInput()`

**Simplified version:** The focus manager becomes trivial — it only tracks block focus. Consider inlining it or simplifying significantly.

#### 4. `frontend/app/store/keymodel.ts`

**Remove:**
- Import of `WaveAIModel` from aipanel
- In `uxCloseBlock()` (lines 180-188): remove AI panel fallback logic that switches to launcher and focuses AI input
- In `genericClose()` (lines 197-223): remove `"waveai"` focus type check that closes AI panel; remove AI panel fallback logic
- In `switchBlockInDirection()` (lines 267-296): remove all `"waveai"` focus type checks, `requestWaveAIFocus()` calls, and `inWaveAI` parameter
- `Ctrl:Shift:0` keybinding (lines 638-644): remove AI panel focus shortcut
- `Cmd:Shift:a` keybinding (lines 679-683): remove AI panel toggle shortcut

#### 5. `frontend/app/store/tabrpcclient.ts`

**Remove:**
- Import of `WaveAIModel` from aipanel
- Entire `handle_waveaiaddcontext()` method (lines 63-79+): RPC handler for adding context to AI panel

#### 6. `frontend/app/store/global.ts`

**Remove:**
- `waveAIRateLimitInfoAtom` from the atoms object (line 213)
- The `waveai:ratelimit` event handler (lines 256-260)

#### 7. `frontend/app/store/services.ts`

**Remove:**
- `SaveWaveAiData()` method (line 18-19)

#### 8. `frontend/app/store/wshclientapi.ts`

**Remove (or leave as dead — this may be generated):**
- `GetWaveAIChatCommand()` (line 291)
- `GetWaveAIRateLimitCommand()` (line 296)
- `StreamWaveAiCommand()` (line 451)
- `WaveAIAddContextCommand()` (line 491)
- `WaveAIEnableTelemetryCommand()` (line 496)
- `WaveAIToolApproveCommand()` (line 501)

**Note:** `wshclientapi.ts` may be auto-generated from the backend. If so, leave these until the backend removes the corresponding RPC handlers. If hand-written, delete them.

#### 9. `frontend/util/tauri-api.ts`

**Remove:**
- `setWaveAIOpen()` method (line 321-323) from the API shim

#### 10. `src-tauri/src/commands/stubs.rs`

**Remove:**
- `set_waveai_open` stub command

#### 11. `src-tauri/src/lib.rs`

**Remove:**
- `commands::stubs::set_waveai_open` from the invoke handler list

#### 12. `frontend/app/onboarding/` (also dead)

These files reference AI panel and are already orphaned (not registered in modal registry):

- `onboarding.tsx` — calls `setAIPanelVisible(true)`
- `onboarding-features.tsx` — has `WaveAIPage` component
- `fakechat.tsx` — has `FakeAIPanelHeader`

**Action:** Delete the entire `frontend/app/onboarding/` directory.

### npm Dependencies to Consider Removing

After removing aipanel, check if these are still imported anywhere:

| Package | Used by aipanel? | Other consumers? |
|---------|-----------------|-----------------|
| `react-resizable-panels` | workspace.tsx (for AI panel) | Check if used elsewhere |

## Implementation Order

1. **Delete `frontend/app/aipanel/`** directory
2. **Delete `frontend/app/onboarding/`** directory
3. **Simplify `workspace.tsx`** — remove PanelGroup, render TabContent directly
4. **Delete or gut `workspace-layout-model.ts`**
5. **Simplify `focusManager.ts`** — remove waveai focus type
6. **Clean `keymodel.ts`** — remove AI shortcuts and fallback logic
7. **Clean `tabrpcclient.ts`** — remove waveaiaddcontext handler
8. **Clean `global.ts`** — remove rate limit atom and event handler
9. **Clean `services.ts`** — remove SaveWaveAiData
10. **Clean `wshclientapi.ts`** — remove WaveAI RPC methods (if hand-written)
11. **Clean `tauri-api.ts`** — remove setWaveAIOpen
12. **Clean Rust stubs** — remove set_waveai_open command
13. **Verify build** — `npm run build:dev` should succeed with no errors
14. **Test** — app launches, tabs work, blocks focus correctly without AI panel

## Risks

- **Focus management regression:** The focus system currently has two modes (`"node"` and `"waveai"`). Removing `"waveai"` simplifies it, but test that arrow-key navigation between panes still works.
- **Keyboard shortcuts:** `Cmd+Shift+A` and `Ctrl+Shift+0` will become unbound. Verify no user-facing documentation references them.
- **Generated code:** If `wshclientapi.ts` or `gotypes.d.ts` are generated from the backend, the WaveAI types/methods will reappear on next generation. The backend RPC handlers should also be removed to prevent this.

## Lines Removed

| Category | Lines |
|----------|-------|
| aipanel/ directory | 2,575 |
| onboarding/ directory | ~400 |
| workspace.tsx simplification | ~50 |
| workspace-layout-model.ts | ~200 |
| focusManager.ts simplification | ~30 |
| keymodel.ts cleanup | ~50 |
| Other store/API cleanup | ~30 |
| **Total** | **~3,335** |
