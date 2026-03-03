# Web Widget Implementation Spec - Tauri v2

**Version:** 1.0
**Status:** Draft
**Target Release:** 0.25.0

## Problem Statement

The web widget currently shows a black screen because it uses Electron's `<webview>` tag, which doesn't exist in Tauri v2. The temporary fix (iframe) displays content but lacks critical features like navigation controls, devtools, and proper sandboxing.

**Critical Constraint:** Any solution MUST NOT create separate taskbar items. Each web widget should be embedded within the main application window's UI without appearing as a separate application window in the OS taskbar.

### Current State (v0.24.3)

**What Works:**
- ✅ Widget button appears in WidgetBar
- ✅ Clicking opens a pane with iframe
- ✅ Basic URL loading
- ✅ URL bar displays current URL

**What Doesn't Work:**
- ❌ Back/forward navigation buttons (disabled)
- ❌ Reload button functionality
- ❌ WebView devtools
- ❌ Media controls (mute/unmute)
- ❌ Zoom controls
- ❌ User agent switching
- ❌ Find-in-page search
- ❌ Cookie/storage management
- ❌ Proper sandboxing
- ❌ Navigation event handling
- ❌ New window handling

### Impact

Users cannot:
- Navigate web history
- Debug web pages
- Control media playback
- Search page content
- Manage web state

## Requirements

### Functional Requirements

1. **URL Navigation**
   - Load arbitrary URLs
   - Navigate back/forward through history
   - Reload current page
   - Stop loading page
   - Handle redirects

2. **User Controls**
   - URL input with autocomplete
   - Navigation buttons (back, forward, home, reload)
   - Zoom in/out controls
   - Find-in-page search
   - Media controls (play/pause, mute)

3. **Developer Features**
   - Toggle devtools for web content
   - Inspect elements
   - View console logs
   - Network inspection

4. **Security**
   - Sandboxed execution
   - CSP enforcement
   - Separate cookie storage per partition
   - User agent control

5. **Integration**
   - Bookmark management
   - Homepage settings (global + per-block)
   - Session persistence
   - Multiple web widgets in tabs

### Non-Functional Requirements

1. **Performance**
   - Smooth scrolling and rendering
   - Memory efficient (unload inactive widgets)
   - Fast page load times

2. **Compatibility**
   - Support modern web standards
   - Handle PDFs, media, downloads
   - Work on Windows, macOS, Linux

3. **Usability**
   - Keyboard shortcuts (Cmd+L for URL, Cmd+R for reload)
   - Context menu integration
   - Error handling for failed loads

## Technical Approach

### Option 1: Enhanced IFrame (Current - Temporary)

**Pros:**
- Simple implementation
- Already working for basic display

**Cons:**
- No access to webview APIs
- Limited control over navigation
- No devtools integration
- Cannot intercept network requests
- Cross-origin restrictions

**Verdict:** ❌ Not suitable for production

### Option 2: Tauri WebviewWindow

Create a child WebviewWindow for each web widget instance.

**Pros:**
- Full Tauri v2 support
- Native webview with all features
- Devtools support via window methods
- Proper sandboxing
- Event handling (navigation, load, etc.)
- Can use `.skip_taskbar(true)` to prevent taskbar appearance

**Cons:**
- ❌ **FATAL:** Positioning synchronization glitches
  - Child window lags behind when dragging/resizing main window
  - Visible "sliding" effect as windows reposition
  - Cannot achieve smooth embedded appearance
- Each widget is a separate OS window
- Requires complex window management

**Verdict:** ❌ Not viable due to poor UX (positioning glitches)

### Option 3: Custom WebView Component (Recommended)

Embed platform-specific webview directly into Tauri window using native APIs.

**Approach:**
- Windows: Embed WebView2 control into Tauri window's HWND
- macOS: Embed WKWebView into Tauri window's NSView
- Linux: Embed webkit2gtk into Tauri window's GtkWindow

**Pros:**
- ✅ Properly embedded (no positioning sync issues)
- ✅ Full webview features available
- ✅ Native performance
- ✅ No taskbar issues (embedded in existing window)
- ✅ Smooth dragging/resizing

**Cons:**
- Complex platform-specific Rust code
- Requires FFI/unsafe code for each platform
- More maintenance burden
- Need to bridge events back to Tauri
- May conflict with Tauri's main webview (need testing)

**Verdict:** ✅ Most viable for production quality embedded browser

**Technical Notes:**
- Tauri uses tao (window library) + wry (webview library)
- Can potentially use wry's WebViewBuilder directly for embedded views
- Need to investigate if wry supports embedding multiple webviews in single window

### Option 4: Enhanced IFrame with Feature Bridge

Keep iframe but add features via postMessage bridge and parent window proxying.

**Approach:**
- Use iframe for display (stable, no positioning issues)
- Inject bridge script into iframe pages
- Proxy navigation, devtools, zoom via parent window
- Use Tauri commands to control iframe indirectly

**Pros:**
- ✅ Simple, stable implementation
- ✅ No positioning sync issues
- ✅ Properly embedded
- ✅ Can add navigation controls (via history API)
- ✅ Can add zoom (via CSS transform)

**Cons:**
- ❌ No native devtools for iframe content
- ❌ Cross-origin limitations (can't inject into all sites)
- ❌ Limited sandboxing control
- ❌ Performance overhead from proxying

**Verdict:** ⚠️  Pragmatic fallback - 70% feature parity, 100% stability

### Option 5: Wait for Tauri WebView Plugin

Tauri may add webview component support in future.

**Pros:**
- Official solution
- Proper integration

**Cons:**
- Not available yet
- Unknown timeline

**Verdict:** ❌ Not feasible for current release

## Selected Approach: Option 3 (Custom WebView Component) + Option 4 Fallback

**Primary:** Embed platform-specific webview directly into Tauri window (Option 3)
**Fallback:** Enhanced iframe with feature bridge (Option 4)

### Decision Rationale

**Why Not Option 2 (WebviewWindow)?**
- User testing revealed fatal UX flaw: "you can drag and move it around, and as you do that the webview moves in weird ways"
- Child window repositioning cannot sync perfectly with parent
- Visible lag/sliding effect breaks embedded appearance
- Even with `.skip_taskbar(true)`, positioning glitches make it unusable

**Why Option 3 (Custom WebView)?**
- Properly embedded - no positioning sync issues
- Full webview features (navigation, devtools, sandboxing)
- Professional UX with smooth dragging/resizing
- Worth complexity for production-quality browser widget

**Why Option 4 (Enhanced IFrame) as Fallback?**
- If Option 3 proves too complex or has compatibility issues
- Gets 70% of features with 100% stability
- Already partially working
- Quick to implement improvements (navigation, zoom, etc.)

### Implementation Strategy

**Phase 1: Investigate wry multi-webview support** (3-5 days)
- Research if Tauri's wry library supports multiple embedded webviews
- Prototype single embedded webview in Tauri window
- Test compatibility with main Tauri webview

**Phase 2a: If wry supports embedding** → Implement Option 3
**Phase 2b: If wry doesn't support** → Implement Option 4 (Enhanced iframe)

### Architecture

```
Main Window (AgentMux UI)
├── TabBar
├── Blocks
│   ├── Terminal Block
│   ├── Web Block (placeholder div)
│   │   └── [Child WebviewWindow positioned over placeholder]
│   └── Preview Block
└── WidgetBar
```

### Implementation Strategy

1. **Frontend Changes**
   - Replace `<iframe>` with positioned `<div>` placeholder
   - Calculate absolute position of placeholder
   - Create child WebviewWindow at that position
   - Handle resize/move events to update child window

2. **Backend Commands** (Rust)
   ```rust
   // Create web widget window
   create_web_widget(block_id: String, url: String, bounds: Rect) -> WindowLabel

   // Navigate web widget
   navigate_web_widget(label: String, url: String)

   // Control web widget
   web_widget_go_back(label: String)
   web_widget_go_forward(label: String)
   web_widget_reload(label: String)
   web_widget_stop(label: String)

   // Toggle devtools for web widget
   toggle_web_widget_devtools(label: String)

   // Manage web widget
   resize_web_widget(label: String, bounds: Rect)
   close_web_widget(label: String)
   ```

3. **Event Handling**
   - Listen for navigation events from child window
   - Update URL bar in main UI
   - Handle window focus/blur
   - Sync zoom levels

### Detailed Design

#### 0. Taskbar Solution

**Problem:** Default WebviewWindow behavior creates OS windows that appear in the taskbar as separate applications.

**Solution:** Configure window with `.skip_taskbar(true)` flag:

```rust
WebviewWindowBuilder::new(&app, &label, url)
    .skip_taskbar(true)  // Prevents taskbar appearance
    .parent_window(main_window)  // Makes it a child window
    .decorations(false)  // No title bar
    .transparent(true)   // Blends with UI
    .build()
```

**Platform Behavior:**
- **Windows:** Window excluded from taskbar and Alt+Tab
- **macOS:** Window excluded from Dock and Cmd+Tab
- **Linux:** Behavior depends on window manager (most respect skip_taskbar hint)

**Result:** Web widget appears embedded in main UI without separate OS presence.

#### 1. WebViewModel Updates

```typescript
// frontend/app/view/webview/webview.tsx

class WebViewModel {
    webviewWindowLabel: PrimitiveAtom<string | null>;
    webviewBounds: Atom<Rect>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        // ... existing code ...
        this.webviewWindowLabel = atom(null);
        this.webviewBounds = atom({ x: 0, y: 0, width: 0, height: 0 });
    }

    async createWebviewWindow(url: string, bounds: Rect) {
        const label = await invoke<string>("create_web_widget", {
            blockId: this.blockId,
            url,
            bounds
        });
        globalStore.set(this.webviewWindowLabel, label);
    }

    async updateWebviewBounds(bounds: Rect) {
        const label = globalStore.get(this.webviewWindowLabel);
        if (label) {
            await invoke("resize_web_widget", { label, bounds });
        }
    }

    async closeWebviewWindow() {
        const label = globalStore.get(this.webviewWindowLabel);
        if (label) {
            await invoke("close_web_widget", { label });
            globalStore.set(this.webviewWindowLabel, null);
        }
    }
}
```

#### 2. WebView Component

```typescript
const WebView = memo(({ model, blockRef }: WebViewProps) => {
    const [placeholderRef, setPlaceholderRef] = useState<HTMLDivElement | null>(null);
    const webviewLabel = useAtomValue(model.webviewWindowLabel);

    // Calculate bounds when placeholder moves/resizes
    useEffect(() => {
        if (!placeholderRef) return;

        const updateBounds = () => {
            const rect = placeholderRef.getBoundingClientRect();
            const bounds = {
                x: rect.left,
                y: rect.top,
                width: rect.width,
                height: rect.height
            };

            if (webviewLabel) {
                model.updateWebviewBounds(bounds);
            } else {
                const url = globalStore.get(model.url) || defaultUrl;
                model.createWebviewWindow(url, bounds);
            }
        };

        updateBounds();
        window.addEventListener('resize', updateBounds);

        return () => window.removeEventListener('resize', updateBounds);
    }, [placeholderRef, webviewLabel]);

    // Cleanup on unmount
    useEffect(() => {
        return () => {
            model.closeWebviewWindow();
        };
    }, []);

    return (
        <div
            ref={setPlaceholderRef}
            className="webview-placeholder"
            style={{ width: '100%', height: '100%', position: 'relative' }}
        />
    );
});
```

#### 3. Rust Backend Commands

```rust
// src-tauri/src/commands/webwidget.rs

use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct WebWidgetState {
    widgets: Mutex<HashMap<String, WebviewWindow>>,
}

#[tauri::command]
pub async fn create_web_widget(
    app: tauri::AppHandle,
    state: tauri::State<'_, WebWidgetState>,
    block_id: String,
    url: String,
    bounds: Rect,
) -> Result<String, String> {
    let label = format!("webwidget-{}", block_id);

    let window = WebviewWindowBuilder::new(
        &app,
        &label,
        WebviewUrl::External(url.parse().map_err(|e| format!("{}", e))?)
    )
    .title("Web Widget")
    .position(bounds.x, bounds.y)
    .inner_size(bounds.width, bounds.height)
    .decorations(false)
    .transparent(true)
    .skip_taskbar(true)  // CRITICAL: Prevents window from appearing in taskbar
    .parent_window(app.get_webview_window("main").unwrap())
    .build()
    .map_err(|e| format!("Failed to create web widget: {}", e))?;

    state.widgets.lock().unwrap().insert(label.clone(), window);

    Ok(label)
}

#[tauri::command]
pub async fn navigate_web_widget(
    state: tauri::State<'_, WebWidgetState>,
    label: String,
    url: String,
) -> Result<(), String> {
    let widgets = state.widgets.lock().unwrap();
    if let Some(window) = widgets.get(&label) {
        window.navigate(url.parse().map_err(|e| format!("{}", e))?)
            .map_err(|e| format!("{}", e))?;
        Ok(())
    } else {
        Err("Web widget not found".to_string())
    }
}

#[tauri::command]
pub fn web_widget_go_back(
    state: tauri::State<'_, WebWidgetState>,
    label: String,
) -> Result<(), String> {
    let widgets = state.widgets.lock().unwrap();
    if let Some(window) = widgets.get(&label) {
        // Note: Tauri v2 doesn't have direct back/forward
        // May need to inject JavaScript or use custom navigation
        Err("Not implemented - requires JS injection".to_string())
    } else {
        Err("Web widget not found".to_string())
    }
}

#[tauri::command]
pub fn resize_web_widget(
    state: tauri::State<'_, WebWidgetState>,
    label: String,
    bounds: Rect,
) -> Result<(), String> {
    let widgets = state.widgets.lock().unwrap();
    if let Some(window) = widgets.get(&label) {
        window.set_position(tauri::Position::Physical(
            tauri::PhysicalPosition::new(bounds.x as i32, bounds.y as i32)
        )).map_err(|e| format!("{}", e))?;

        window.set_size(tauri::Size::Physical(
            tauri::PhysicalSize::new(bounds.width as u32, bounds.height as u32)
        )).map_err(|e| format!("{}", e))?;

        Ok(())
    } else {
        Err("Web widget not found".to_string())
    }
}

#[tauri::command]
pub fn close_web_widget(
    state: tauri::State<'_, WebWidgetState>,
    label: String,
) -> Result<(), String> {
    let mut widgets = state.widgets.lock().unwrap();
    if let Some(window) = widgets.remove(&label) {
        window.close().map_err(|e| format!("{}", e))?;
        Ok(())
    } else {
        Err("Web widget not found".to_string())
    }
}

#[tauri::command]
pub fn toggle_web_widget_devtools(
    state: tauri::State<'_, WebWidgetState>,
    label: String,
) -> Result<(), String> {
    let widgets = state.widgets.lock().unwrap();
    if let Some(window) = widgets.get(&label) {
        if window.is_devtools_open() {
            window.close_devtools();
        } else {
            window.open_devtools();
        }
        Ok(())
    } else {
        Err("Web widget not found".to_string())
    }
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
```

#### 4. Navigation Limitations

**Challenge:** Tauri's WebviewWindow doesn't expose back/forward history.

**Solutions:**

A. **JavaScript Injection**
```rust
// Inject history navigation
window.eval("window.history.back()")
```

B. **Custom Navigation Manager**
```rust
// Track URL history in Rust state
// Manually navigate to previous URLs
```

C. **Use WebView2 API directly** (Windows only)
```rust
// Access platform-specific APIs for full control
```

**Recommended:** Start with (A), fall back to (B) for cross-platform support.

## Implementation Plan

### Phase 1: Basic WebviewWindow (Week 1)

**Goals:**
- Replace iframe with WebviewWindow
- Basic URL loading works
- Placeholder positioning works

**Tasks:**
1. Create `commands/webwidget.rs` with basic commands
2. Update `WebViewModel` to manage window lifecycle
3. Update `WebView` component with placeholder
4. Test single web widget creation/destruction

**Success Criteria:**
- Web widget displays content
- Widget moves/resizes with block
- Widget closes when block closes

### Phase 2: Navigation Controls (Week 2)

**Goals:**
- Back/forward buttons work
- Reload button works
- URL bar updates on navigation

**Tasks:**
1. Implement navigation commands (with JS injection)
2. Set up event listeners for navigation
3. Update URL bar on navigation events
4. Handle history state

**Success Criteria:**
- Can navigate back/forward
- URL bar reflects current page
- Reload works

### Phase 3: Advanced Features (Week 3)

**Goals:**
- Devtools toggle works
- Zoom controls work
- Find-in-page works
- Media controls work

**Tasks:**
1. Implement devtools toggle for web widget
2. Add zoom commands
3. Implement find-in-page with JS injection
4. Add media event handling

**Success Criteria:**
- All toolbar buttons functional
- Settings menu works
- User agent switching works

### Phase 4: Polish & Testing (Week 4)

**Goals:**
- Multi-widget support
- Performance optimization
- Edge case handling
- Documentation

**Tasks:**
1. Test multiple web widgets simultaneously
2. Handle tab switching (hide/show windows)
3. Optimize window creation/destruction
4. Write user documentation
5. Add error boundaries

**Success Criteria:**
- 5+ web widgets work smoothly
- No memory leaks
- All features documented
- Error handling robust

## Testing Strategy

### Unit Tests
- WebViewModel state management
- URL validation and normalization
- Event handler registration

### Integration Tests
- Create and destroy web widgets
- Navigation between pages
- Multi-widget scenarios
- Window positioning accuracy

### Manual Tests
- Complex web apps (Gmail, YouTube, etc.)
- PDF rendering
- Media playback
- Downloads handling
- Devtools functionality

### Performance Tests
- Memory usage with 10+ widgets
- Window creation time
- Navigation responsiveness
- CPU usage during video playback

## Risks & Mitigations

### Risk 1: Window Positioning Glitches

**Impact:** High
**Probability:** Medium

**Mitigation:**
- Debounce resize events
- Use requestAnimationFrame for updates
- Test on multiple displays

### Risk 2: Back/Forward Not Available

**Impact:** High
**Probability:** Low

**Mitigation:**
- Implement custom history tracking
- Use JS injection as fallback
- Document limitations if unsolvable

### Risk 3: Performance with Many Widgets

**Impact:** Medium
**Probability:** Medium

**Mitigation:**
- Lazy load widgets
- Destroy off-screen widgets
- Limit maximum concurrent widgets

### Risk 4: Cross-Platform Differences

**Impact:** Medium
**Probability:** High

**Mitigation:**
- Test on all platforms early
- Use platform-specific code where needed
- Document platform limitations

### Risk 5: Taskbar Behavior on Linux

**Impact:** Low
**Probability:** Low

**Issue:** Some Linux window managers may ignore `.skip_taskbar(true)` hint

**Mitigation:**
- Test on popular DEs (GNOME, KDE, XFCE)
- Use additional X11/Wayland hints if needed
- Document workarounds for affected WMs
- Fallback: Use iframe on platforms where skip_taskbar fails

## Success Metrics

1. **Feature Parity:** 95% of Electron webview features working
2. **Performance:** < 100ms widget creation time
3. **Stability:** Zero crashes in 1-hour stress test
4. **Usability:** User can complete all web browsing tasks
5. **Memory:** < 150MB per widget average

## Future Enhancements (Post-0.25.0)

1. **Tab-based browsing** within web widget
2. **Extension support** (uBlock, etc.)
3. **Session restoration** on app restart
4. **Custom CSS injection**
5. **Network interception** for debugging
6. **Screenshot capture** of web content
7. **Print to PDF** functionality

## Alternatives Considered

### Keep IFrame with Limitations
- Document missing features
- Focus on other improvements
- **Rejected:** Too limiting for users

### Embed Chromium Directly
- Use CEF (Chromium Embedded Framework)
- **Rejected:** Massive dependency, size increase

### Use System Browser
- Open URLs in default browser
- **Rejected:** Breaks embedded workflow

## References

- [Tauri v2 WebviewWindow Docs](https://beta.tauri.app/references/v2/js/webviewwindow/)
- [Tauri Window Management](https://beta.tauri.app/guides/window-management/)
- [Electron WebView Migration Guide](https://www.electronjs.org/docs/latest/api/webview-tag)
- Original AgentMux webview implementation: `frontend/app/view/webview/webview.tsx`

## Approvals

- [ ] Engineering Lead
- [ ] Product Manager
- [ ] QA Lead
- [ ] Security Review

---

**Document Version:** 1.2
**Last Updated:** 2026-02-12
**Next Review:** 2026-02-19
**Changelog:**
- v1.2 (2026-02-12): Marked Option 2 as non-viable due to positioning glitches; promoted Option 3 (Custom WebView) to recommended approach with Option 4 (Enhanced iframe) as fallback
- v1.1 (2026-02-12): Added `.skip_taskbar(true)` solution to prevent taskbar appearance
- v1.0 (2026-02-11): Initial draft
