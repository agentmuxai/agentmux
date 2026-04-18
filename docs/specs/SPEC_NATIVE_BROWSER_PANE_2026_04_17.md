# SPEC: Native Browser Pane via CefBrowserView

**Date:** 2026-04-17
**Status:** Draft
**Priority:** High — iframe approach crashes CEF on external sites

---

## Problem

The iframe-based browser pane crashes CEF when loading external sites.
`X-Frame-Options: SAMEORIGIN` on sites like google.com triggers
`ERR_BLOCKED_BY_RESPONSE` which CEF propagates as a top-level navigation
failure, killing the entire app. The iframe approach is fundamentally
limited and cannot be fixed.

## Solution

Create a **native CefBrowserView** for each browser pane. This is a
second Chromium browser instance rendered as a child view of the main
window, positioned over the pane's DOM rect. It has its own process,
its own cookie jar, and full rendering capability — no iframe restrictions.

---

## Architecture

```
Main Window (CefWindow)
  └─ Main BrowserView (CefBrowserView #1 — SolidJS UI)
       └─ Pane layout (DOM)
            ├─ Terminal pane (xterm.js)
            ├─ Agent pane (SolidJS)
            └─ Browser pane placeholder (DOM div with known rect)
                 ↕ position sync via IPC
  └─ Browser BrowserView (CefBrowserView #2 — external URL)
       └─ positioned over the placeholder div's rect
```

The browser BrowserView is a **sibling** of the main BrowserView inside
the CefWindow, not a child of the DOM. It floats on top, positioned to
match the placeholder div.

---

## Implementation

### Rust: BrowserPaneManager (`agentmux-cef/src/browser_panes.rs`)

```rust
/// Manages embedded browser panes — one CefBrowserView per pane.
pub struct BrowserPaneManager {
    /// Active browser panes: block_id → BrowserPane
    panes: Mutex<HashMap<String, BrowserPane>>,
}

struct BrowserPane {
    browser_view: BrowserView,
    block_id: String,
    current_url: String,
}
```

#### IPC Commands

| Command | Args | Action |
|---------|------|--------|
| `browser_pane_create` | `{ block_id, url }` | Create CefBrowserView, add to window |
| `browser_pane_navigate` | `{ block_id, url }` | Navigate existing pane to new URL |
| `browser_pane_resize` | `{ block_id, x, y, width, height }` | Reposition the overlay view |
| `browser_pane_close` | `{ block_id }` | Remove view from window, destroy browser |
| `browser_pane_go_back` | `{ block_id }` | `browser.go_back()` |
| `browser_pane_go_forward` | `{ block_id }` | `browser.go_forward()` |
| `browser_pane_reload` | `{ block_id }` | `browser.reload()` |

#### Create Flow

```rust
pub fn create_pane(
    window: &Window,
    block_id: &str,
    url: &str,
    rect: Rect,
) -> Result<(), String> {
    // 1. Create a new CefBrowserView with a dedicated client
    let mut client = create_browser_client();
    let mut delegate = BrowserPaneBrowserViewDelegate::new();
    let settings = BrowserSettings::default();
    let url = CefString::from(url);

    let browser_view = browser_view_create(
        client.as_mut(),
        Some(&url),
        Some(&settings),
        None,   // extra_info
        None,   // request_context (use default = shared cookies)
        Some(&mut delegate),
    );

    // 2. Convert BrowserView → View and set bounds
    let mut view = View::from(&browser_view);
    view.set_bounds(Some(&rect));

    // 3. Add as child of the CefWindow (renders on top of main browser)
    window.add_child_view(Some(&mut view));

    // 4. Store in pane map
    self.panes.lock().unwrap().insert(block_id.to_string(), BrowserPane {
        browser_view,
        block_id: block_id.to_string(),
        current_url: url.to_string(),
    });

    Ok(())
}
```

#### Resize Flow

```rust
pub fn resize_pane(block_id: &str, rect: Rect) {
    if let Some(pane) = self.panes.lock().unwrap().get(block_id) {
        let mut view = View::from(&pane.browser_view);
        view.set_bounds(Some(&rect));
    }
}
```

### Frontend: Position Sync

The BrowserViewComponent renders a **placeholder div** and continuously
reports its screen position to the Rust host:

```typescript
// In BrowserViewComponent
const placeholderRef: HTMLDivElement;

// ResizeObserver + scroll listener → report rect to host
const syncPosition = () => {
    if (!placeholderRef) return;
    const rect = placeholderRef.getBoundingClientRect();
    invokeCommand("browser_pane_resize", {
        block_id: model.blockId,
        x: Math.round(rect.x),
        y: Math.round(rect.y),
        width: Math.round(rect.width),
        height: Math.round(rect.height),
    });
};

// Sync on mount, resize, scroll, and layout changes
onMount(() => {
    invokeCommand("browser_pane_create", {
        block_id: model.blockId,
        url: model.urlAtom() || "about:blank",
    });

    const observer = new ResizeObserver(syncPosition);
    observer.observe(placeholderRef);
    window.addEventListener("scroll", syncPosition, true);

    // Poll position every 200ms as a safety net
    const interval = setInterval(syncPosition, 200);

    onCleanup(() => {
        observer.disconnect();
        window.removeEventListener("scroll", syncPosition, true);
        clearInterval(interval);
        invokeCommand("browser_pane_close", { block_id: model.blockId });
    });
});
```

### Navigation Events (Rust → Frontend)

The browser pane's `DisplayHandler` captures URL/title changes and sends
them back to the frontend via IPC events:

```rust
impl DisplayHandler for BrowserPaneClient {
    fn on_title_change(&self, browser: &Browser, title: &CefString) {
        emit_event("browser-pane-title", json!({
            "block_id": self.block_id,
            "title": title.to_string(),
        }));
    }

    fn on_address_change(&self, browser: &Browser, url: &CefString) {
        emit_event("browser-pane-url", json!({
            "block_id": self.block_id,
            "url": url.to_string(),
        }));
    }
}
```

The frontend listens for these events and updates the address bar + title.

---

## Focus Management

When the user clicks inside the browser pane overlay, CEF gives focus to
browser #2. The main browser (#1) loses focus. This means:

- Keyboard shortcuts (Ctrl+T, Ctrl+W) need to be intercepted at the
  window level, not the browser level
- The pane border highlight needs to track which browser has focus
- Clicking back in the main UI (terminal, agent) needs to restore
  focus to browser #1

**Implementation:** The `BrowserPaneClient`'s `FocusHandler` detects
focus changes and emits an IPC event. The frontend uses this to update
the focused-pane UI state.

---

## Cookie Isolation

By default, all CefBrowserViews share the same `RequestContext` (cookies,
cache). For browser panes, options:

1. **Shared cookies (default):** User logs into GitHub in one pane, stays
   logged in across all panes. Simpler, matches user expectations.
2. **Isolated cookies:** Pass a new `RequestContext` per pane. More secure
   but confusing (log in separately per pane).

**Recommendation:** Shared cookies (option 1) for v1. Add per-pane
isolation as a settings toggle later.

---

## DPI / Zoom Handling

CefBrowserView positions use DIPs (device-independent pixels). The
frontend's `getBoundingClientRect()` returns CSS pixels. On high-DPI
displays, these may differ by the device pixel ratio.

The main browser's zoom level also affects the rect. If the user zooms
the chrome (Ctrl++/-), the placeholder div's rect changes but the
CefWindow's coordinate system doesn't.

**Fix:** Multiply the rect by the device pixel ratio and divide by the
chrome zoom factor before passing to `set_bounds()`.

---

## Implementation Plan

### PR 1: BrowserPaneManager + IPC commands (Rust)

1. `agentmux-cef/src/browser_panes.rs` — manager struct, create/resize/close
2. `agentmux-cef/src/ipc.rs` — route browser_pane_* commands
3. State: add `BrowserPaneManager` to `AppState`

### PR 2: Frontend position sync

1. Update `BrowserViewComponent` — placeholder div + ResizeObserver
2. Remove iframe code
3. IPC calls for create/navigate/resize/close
4. Listen for url/title change events from host

### PR 3: Navigation + focus

1. Back/forward/reload via IPC commands
2. Address bar updates from host events
3. Focus management between main and pane browsers

---

## Risks

- **Position sync latency:** The 200ms poll + ResizeObserver should cover
  most cases, but fast window resizes may show a brief misalignment.
  CEF's Views framework handles resize natively so this may not be
  noticeable.

- **Z-order:** The browser overlay always renders on top of the main UI.
  If a modal or context menu opens, it may render behind the browser
  pane. Fix: hide the browser view when modals are open.

- **Performance:** Each browser pane is a separate renderer process
  (~50-100MB). Limit to a reasonable number (e.g., 4 concurrent panes).

---

## Non-Goals

- **Tab management inside a pane.** One URL per pane. Multiple pages =
  multiple panes.
- **Browser extensions.** CEF doesn't support Chrome extensions.
- **Devtools for browser panes.** Use the main app's DevTools (F12).
