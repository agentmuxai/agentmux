# Zoom Functionality Spec - AgentMux

**Version:** 1.0
**Status:** Draft
**Target Release:** 0.24.6
**Created:** 2026-02-12

## Problem Statement

Zoom controls in AgentMux are currently broken or incomplete:

1. **Ctrl+- (Zoom Out) doesn't work** - Used to work, but has stopped functioning
2. **Ctrl++ (Zoom In) broken/inconsistent** - Keyboard shortcut not reliable
3. **Ctrl+Mouse Wheel not implemented** - Industry-standard zoom pattern missing
4. **No visual feedback** - Users don't know current zoom level
5. **Inconsistent zoom targets** - Unclear what gets zoomed (whole app vs individual panes)

### User Impact

- Cannot quickly adjust UI size for readability
- Missing standard browser-like zoom controls
- Frustrating UX regression (previously working feature broke)

### Root Cause: Electron → Tauri Architectural Difference

**Why it worked in Electron:**
- Electron's `BrowserWindow` has **built-in zoom shortcuts** (part of Chromium)
- Ctrl+Plus, Ctrl+Minus, Ctrl+0 worked **automatically** without any code
- Browser-native feature, deeply integrated into the webview

**Current Tauri implementation:**
- Zoom shortcuts defined in **menu items** (`src-tauri/src/menu.rs` lines 137-144):
  ```rust
  let zoom_reset = MenuItem::with_id(app, "zoom-reset", "Reset Zoom", true, Some("CommandOrControl+0"))?;
  let zoom_in = MenuItem::with_id(app, "zoom-in", "Zoom In", true, Some("CommandOrControl+="))?;
  let zoom_out = MenuItem::with_id(app, "zoom-out", "Zoom Out", true, Some("CommandOrControl+-"))?;
  ```
- Handlers exist (lines 238-268) and call `set_zoom_factor()` correctly

**Why menu-based shortcuts are unreliable:**
1. **OS-dependent behavior** - Menu shortcuts work differently on Windows/Mac/Linux
2. **Event capture timing** - Other handlers may intercept keys before menu sees them
3. **Focus sensitivity** - Menu shortcuts only work when menu system is active
4. **Tauri regression** - Possible Tauri version update broke menu shortcut handling

**Solution:** Move zoom shortcuts from menu system to **application-level keyboard handler** (keymodel.ts) for reliable, cross-platform behavior similar to Electron's built-in shortcuts.

## Current Implementation

### Existing Zoom Support

**Web Widget Zoom** (`frontend/app/view/webview/webview.tsx`):
- Zoom works via settings menu (right-click → Set Zoom Factor)
- Stored in block metadata as `"web:zoom"`
- Supports 25%-200% in predefined steps
- Uses `webview.setZoomFactor()` for iframe zoom

**Global App Zoom** (`frontend/wave.ts`):
- Tauri API provides `getZoomFactor()` and `onZoomFactorChange()`
- No keyboard shortcuts registered
- No mouse wheel handler

### What's Missing

1. **No global keyboard shortcuts** for zoom in/out/reset
2. **No mouse wheel handler** for Ctrl+Wheel zoom
3. **No zoom level indicator** in UI
4. **No smooth zoom steps** (current web widget uses discrete jumps)

## Requirements

### Functional Requirements

1. **Global Zoom Controls**
   - Ctrl+Plus (or Ctrl+=): Zoom In (increase by 10%)
   - Ctrl+Minus: Zoom Out (decrease by 10%)
   - Ctrl+0: Reset Zoom to 100%
   - Ctrl+Mouse Wheel: Zoom In/Out (smooth steps)

2. **Zoom Range**
   - Minimum: 25% (0.25x)
   - Maximum: 300% (3.0x)
   - Default: 100% (1.0x)
   - Step size: 10% for keyboard, 5% for mouse wheel

3. **Visual Feedback**
   - Transient zoom indicator when changing (shows "125%" for 1.5s)
   - Status bar indicator (optional, low priority)

4. **Persistence**
   - Save zoom level to settings
   - Restore on app launch
   - Per-window zoom (if multiple windows)

5. **Scope**
   - **Primary:** Global app zoom (entire UI scales)
   - **Secondary:** Web widget zoom remains independent

### Non-Functional Requirements

1. **Performance**: Zoom changes apply instantly (<50ms)
2. **Smoothness**: No jank or layout thrashing
3. **Compatibility**: Works on Windows, macOS, Linux
4. **Accessibility**: Respects system zoom/DPI settings

## Technical Design

### Architecture

```
┌─────────────────────────────────────────┐
│         Global Keyboard Handler         │
│    (frontend/app/store/keymodel.ts)     │
│                                         │
│  - Ctrl+Plus  → handleZoomIn()         │
│  - Ctrl+Minus → handleZoomOut()        │
│  - Ctrl+0     → handleZoomReset()      │
└─────────────────┬───────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────┐
│       Global Mouse Wheel Handler        │
│      (frontend/app/app.tsx)             │
│                                         │
│  - onWheel(e) if e.ctrlKey             │
│  - deltaY > 0 → Zoom Out               │
│  - deltaY < 0 → Zoom In                │
└─────────────────┬───────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────┐
│          Zoom Manager Module            │
│     (NEW: frontend/app/zoom.ts)         │
│                                         │
│  - currentZoom: Atom<number>           │
│  - setZoom(factor: number)             │
│  - zoomIn(step?: number)               │
│  - zoomOut(step?: number)              │
│  - zoomReset()                         │
│  - persistZoom()                       │
└─────────────────┬───────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────┐
│           Tauri Backend                 │
│       (src-tauri/src/window.rs)         │
│                                         │
│  - set_zoom_factor(factor: f64)        │
│  - get_zoom_factor() → f64             │
│  - save_zoom_to_config()               │
└─────────────────────────────────────────┘
```

### Implementation Details

#### 1. Zoom Manager Module

**File:** `frontend/app/store/zoom.ts` (NEW)

```typescript
import { atom } from "jotai";
import { globalStore, getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";

// Zoom constants
export const MIN_ZOOM = 0.25;
export const MAX_ZOOM = 3.0;
export const DEFAULT_ZOOM = 1.0;
export const KEYBOARD_STEP = 0.1; // 10%
export const WHEEL_STEP = 0.05;   // 5%

// Current zoom level atom
export const zoomFactorAtom = atom<number>(DEFAULT_ZOOM);

// Zoom indicator visibility (auto-hide after 1.5s)
export const zoomIndicatorVisibleAtom = atom<boolean>(false);
let zoomIndicatorTimeout: NodeJS.Timeout | null = null;

/**
 * Clamp zoom factor to valid range
 */
function clampZoom(factor: number): number {
    return Math.min(Math.max(factor, MIN_ZOOM), MAX_ZOOM);
}

/**
 * Round to nearest 5% for clean display
 */
function roundZoom(factor: number): number {
    return Math.round(factor * 20) / 20; // Round to 0.05 increments
}

/**
 * Set zoom factor and update UI
 */
export function setZoom(factor: number): void {
    const clampedZoom = clampZoom(roundZoom(factor));

    // Update atom
    globalStore.set(zoomFactorAtom, clampedZoom);

    // Apply to Tauri window
    getApi().setZoomFactor?.(clampedZoom);

    // Persist to settings
    persistZoom(clampedZoom);

    // Show indicator
    showZoomIndicator();
}

/**
 * Increase zoom by step
 */
export function zoomIn(step: number = KEYBOARD_STEP): void {
    const current = globalStore.get(zoomFactorAtom);
    setZoom(current + step);
}

/**
 * Decrease zoom by step
 */
export function zoomOut(step: number = KEYBOARD_STEP): void {
    const current = globalStore.get(zoomFactorAtom);
    setZoom(current - step);
}

/**
 * Reset zoom to 100%
 */
export function zoomReset(): void {
    setZoom(DEFAULT_ZOOM);
}

/**
 * Persist zoom level to user settings
 */
async function persistZoom(factor: number): Promise<void> {
    try {
        await RpcApi.SetConfigCommand(TabRpcClient, {
            key: "window:zoomfactor",
            value: factor.toString(),
        });
    } catch (e) {
        console.error("Failed to persist zoom factor:", e);
    }
}

/**
 * Load zoom level from settings on startup
 */
export async function loadZoom(): Promise<void> {
    try {
        const saved = await RpcApi.GetConfigCommand(TabRpcClient, {
            key: "window:zoomfactor",
        });
        if (saved) {
            const factor = parseFloat(saved);
            if (!isNaN(factor)) {
                setZoom(factor);
                return;
            }
        }
    } catch (e) {
        console.error("Failed to load zoom factor:", e);
    }

    // Fallback: use Tauri's current zoom
    const currentZoom = getApi().getZoomFactor?.() ?? DEFAULT_ZOOM;
    globalStore.set(zoomFactorAtom, currentZoom);
}

/**
 * Show zoom indicator with auto-hide
 */
function showZoomIndicator(): void {
    // Clear existing timeout
    if (zoomIndicatorTimeout) {
        clearTimeout(zoomIndicatorTimeout);
    }

    // Show indicator
    globalStore.set(zoomIndicatorVisibleAtom, true);

    // Hide after 1.5 seconds
    zoomIndicatorTimeout = setTimeout(() => {
        globalStore.set(zoomIndicatorVisibleAtom, false);
        zoomIndicatorTimeout = null;
    }, 1500);
}

/**
 * Get zoom as percentage string
 */
export function getZoomPercentage(): string {
    const zoom = globalStore.get(zoomFactorAtom);
    return `${Math.round(zoom * 100)}%`;
}
```

#### 2. Keyboard Shortcuts

**File:** `frontend/app/store/keymodel.ts` (UPDATE)

Add to `registerGlobalKeys()`:

```typescript
import { zoomIn, zoomOut, zoomReset } from "@/app/store/zoom";

// In registerGlobalKeys() function:

// Zoom In: Ctrl+Plus or Ctrl+=
globalKeyMap.set("Cmd:=", (waveEvent) => {
    zoomIn();
    return true;
});
globalKeyMap.set("Cmd:+", (waveEvent) => {
    zoomIn();
    return true;
});

// Zoom Out: Ctrl+Minus
globalKeyMap.set("Cmd:-", (waveEvent) => {
    zoomOut();
    return true;
});

// Zoom Reset: Ctrl+0
globalKeyMap.set("Cmd:0", (waveEvent) => {
    zoomReset();
    return true;
});
```

**Note:** `Cmd` is keyutil's platform-agnostic modifier (Ctrl on Windows/Linux, Cmd on macOS)

#### 3. Mouse Wheel Handler

**File:** `frontend/app/app.tsx` (UPDATE)

Add wheel event listener:

```typescript
import { zoomIn, zoomOut, WHEEL_STEP } from "@/app/store/zoom";

function AppInner() {
    // ... existing code ...

    // Handle Ctrl+Wheel zoom
    const handleWheel = useCallback((e: WheelEvent) => {
        // Only zoom if Ctrl/Cmd is held
        if (!e.ctrlKey && !e.metaKey) {
            return;
        }

        // Prevent default browser zoom
        e.preventDefault();

        // Zoom direction based on wheel delta
        // Note: deltaY > 0 = scroll down = zoom out
        if (e.deltaY > 0) {
            zoomOut(WHEEL_STEP);
        } else if (e.deltaY < 0) {
            zoomIn(WHEEL_STEP);
        }
    }, []);

    useEffect(() => {
        // Add passive: false to allow preventDefault
        window.addEventListener("wheel", handleWheel, { passive: false });

        return () => {
            window.removeEventListener("wheel", handleWheel);
        };
    }, [handleWheel]);

    // ... rest of component ...
}
```

#### 4. Zoom Indicator Component

**File:** `frontend/app/element/zoomindicator.tsx` (NEW)

```typescript
import { useAtomValue } from "jotai";
import { zoomFactorAtom, zoomIndicatorVisibleAtom, getZoomPercentage } from "@/app/store/zoom";
import "./zoomindicator.scss";

export function ZoomIndicator() {
    const visible = useAtomValue(zoomIndicatorVisibleAtom);
    const zoomPercent = getZoomPercentage();

    if (!visible) {
        return null;
    }

    return (
        <div className="zoom-indicator">
            <div className="zoom-indicator-content">
                {zoomPercent}
            </div>
        </div>
    );
}
```

**File:** `frontend/app/element/zoomindicator.scss` (NEW)

```scss
.zoom-indicator {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 10000;
    pointer-events: none;

    animation: zoom-fade-in-out 1.5s ease-out;
}

.zoom-indicator-content {
    background: rgba(0, 0, 0, 0.8);
    color: white;
    padding: 16px 32px;
    border-radius: 8px;
    font-size: 48px;
    font-weight: 600;
    box-shadow: 0 4px 20px rgba(0, 0, 0, 0.3);
}

@keyframes zoom-fade-in-out {
    0% {
        opacity: 0;
        transform: translate(-50%, -50%) scale(0.8);
    }
    10% {
        opacity: 1;
        transform: translate(-50%, -50%) scale(1);
    }
    90% {
        opacity: 1;
        transform: translate(-50%, -50%) scale(1);
    }
    100% {
        opacity: 0;
        transform: translate(-50%, -50%) scale(0.8);
    }
}
```

#### 5. Tauri Backend Support

**Check existing implementation:**
- Tauri v2 has built-in `WebviewWindow::set_zoom()` method
- Current implementation already has `getZoomFactor()` exposed
- Need to add `setZoomFactor()` command if not present

**File:** `src-tauri/src/window.rs` (or similar)

```rust
#[tauri::command]
pub fn set_zoom_factor(
    window: tauri::Window,
    factor: f64,
) -> Result<(), String> {
    window
        .set_zoom(factor)
        .map_err(|e| format!("Failed to set zoom: {}", e))
}
```

Register command in `main.rs`:
```rust
.invoke_handler(tauri::generate_handler![
    // ... existing commands ...
    set_zoom_factor,
])
```

## Implementation Plan

### Phase 1: Core Zoom Manager (Day 1)
- Create `zoom.ts` module with atoms and functions
- Add keyboard shortcuts to keymodel
- Test Ctrl+/- zoom in/out works

**Success Criteria:**
- Ctrl+Plus zooms in by 10%
- Ctrl+Minus zooms out by 10%
- Ctrl+0 resets to 100%
- Zoom persists across app restarts

### Phase 2: Mouse Wheel Support (Day 2)
- Add wheel event handler to app.tsx
- Implement Ctrl+Wheel zoom
- Test smooth zooming with wheel

**Success Criteria:**
- Ctrl+Wheel zooms smoothly
- No interference with normal scrolling
- Works in all panes/views

### Phase 3: Visual Feedback (Day 3)
- Create ZoomIndicator component
- Add fade-in/out animation
- Integrate into app layout

**Success Criteria:**
- Indicator shows on zoom changes
- Auto-hides after 1.5 seconds
- Displays current percentage (e.g., "125%")

### Phase 4: Polish & Testing (Day 4)
- Test on all platforms
- Verify Tauri backend integration
- Handle edge cases (min/max limits)
- Add unit tests

**Success Criteria:**
- All features work on Windows/macOS/Linux
- No crashes or errors
- Respects zoom limits
- Settings persistence works

## Testing Strategy

### Manual Tests

1. **Keyboard Shortcuts**
   - Press Ctrl+Plus → UI zooms in
   - Press Ctrl+Minus → UI zooms out
   - Press Ctrl+0 → UI resets to 100%
   - Zoom indicator appears on each change

2. **Mouse Wheel**
   - Hold Ctrl, scroll up → UI zooms in
   - Hold Ctrl, scroll down → UI zooms out
   - Without Ctrl, scroll works normally

3. **Limits**
   - Zoom out to 25% → cannot go lower
   - Zoom in to 300% → cannot go higher

4. **Persistence**
   - Set zoom to 150%
   - Restart app
   - Zoom is still 150%

5. **Multi-Window** (if applicable)
   - Open second window
   - Zoom in one window
   - Other window unaffected

### Automated Tests

```typescript
// tests/zoom.test.ts
import { describe, it, expect } from "vitest";
import { zoomIn, zoomOut, zoomReset, setZoom, MIN_ZOOM, MAX_ZOOM } from "@/app/store/zoom";

describe("Zoom Manager", () => {
    it("should zoom in by step", () => {
        zoomReset();
        zoomIn(0.1);
        expect(getZoom()).toBe(1.1);
    });

    it("should zoom out by step", () => {
        zoomReset();
        zoomOut(0.1);
        expect(getZoom()).toBe(0.9);
    });

    it("should reset to 1.0", () => {
        setZoom(1.5);
        zoomReset();
        expect(getZoom()).toBe(1.0);
    });

    it("should clamp to min zoom", () => {
        setZoom(0.1);
        expect(getZoom()).toBe(MIN_ZOOM);
    });

    it("should clamp to max zoom", () => {
        setZoom(5.0);
        expect(getZoom()).toBe(MAX_ZOOM);
    });
});
```

## Risks & Mitigations

### Risk 1: Conflicts with Browser Zoom

**Issue:** Browser shortcuts (Ctrl+Plus) might conflict with app shortcuts

**Mitigation:**
- Use `e.preventDefault()` in keyboard handler
- Tauri apps don't have browser chrome, so less conflict
- Test thoroughly on all platforms

### Risk 2: Tauri API Limitations

**Issue:** Tauri may not expose `setZoomFactor()` in current version

**Mitigation:**
- Check Tauri docs for zoom API
- If missing, use CSS transform as fallback:
  ```typescript
  document.body.style.transform = `scale(${factor})`;
  ```
- File feature request with Tauri team

### Risk 3: Performance with Large Zoom

**Issue:** High zoom levels (200%+) may cause performance issues

**Mitigation:**
- Limit max zoom to 300%
- Use CSS `will-change: transform` for performance
- Monitor frame rates during testing

### Risk 4: DPI Scaling Conflicts

**Issue:** OS-level DPI scaling + app zoom may compound unexpectedly

**Mitigation:**
- Test on high-DPI displays (150%, 200% scaling)
- Document interaction with system scaling
- Consider exposing "zoom relative to system" setting

## Success Metrics

1. **Feature Completeness:** All 3 input methods work (Ctrl+/-, Ctrl+0, Ctrl+Wheel)
2. **Performance:** Zoom changes apply in <50ms
3. **Reliability:** No crashes or errors in 100 zoom cycles
4. **User Satisfaction:** Fix resolves user's reported issue (Ctrl+- works again)
5. **Regression:** No existing features broken by zoom implementation

## Future Enhancements (Post-0.24.6)

1. **Per-Pane Zoom** - Zoom individual terminal/web widget instead of whole app
2. **Zoom Presets** - Quick access to 75%, 100%, 125%, 150%
3. **Status Bar Indicator** - Always-visible zoom level in corner
4. **Zoom Animation** - Smooth CSS transitions between zoom levels
5. **Text-Only Zoom** - Zoom font size without zooming layout
6. **Accessibility Shortcut** - Alt+Shift+Plus for screen reader users

## References

- Tauri Window Zoom API: https://tauri.app/v1/api/js/window/#setwebviewzoom
- Electron Zoom Implementation: https://www.electronjs.org/docs/latest/api/webcontents#contentssetzoomfactorfactor
- Chrome Zoom Shortcuts: Ctrl+Plus, Ctrl+Minus, Ctrl+0, Ctrl+MouseWheel
- AgentMux Zoom Discussion: (user feedback: "ctrl+- used to work but stops")

## Approvals

- [ ] Engineering Lead
- [ ] UX Review
- [ ] Testing Complete

---

**Document Version:** 1.0
**Last Updated:** 2026-02-12
**Next Review:** 2026-02-13
