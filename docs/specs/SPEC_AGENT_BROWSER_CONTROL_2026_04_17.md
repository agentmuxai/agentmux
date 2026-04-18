# SPEC: Agent Browser Control

**Date:** 2026-04-17
**Status:** Draft
**Related:** SPEC_NATIVE_BROWSER_PANE_2026_04_17.md

---

## Problem

### Browser z-order
The native CefBrowserView browser pane renders behind the main BrowserView.
`Window::add_child_view` adds the view to the window's layout but the main
BrowserView fills the entire window — child views are occluded.

CEF's Views framework doesn't expose an `add_overlay_view` API in cef-rs.
The view hierarchy is: Window → Panel → BrowserView (fills panel). Adding
a second BrowserView as a child of the Window places it in the same panel
but behind the existing BrowserView.

### Agent control
Agents need to programmatically open URLs, navigate, interact with pages,
and read page content. This requires an API layer between the agent pane
and the browser pane.

### Logging
Browser pane activity (navigation, errors, console messages) needs to be
captured and accessible for debugging.

---

## Browser Z-Order: Options

### Option A: Frameless popup window (recommended)

Create a separate frameless CefWindow positioned exactly over the pane's
DOM rect. Not a child view — a separate top-level window with no title
bar, no border, always-on-top relative to the main window.

```
Main CefWindow
  └─ Main BrowserView (SolidJS UI — fills entire window)
       └─ Pane layout (DOM)
            └─ Browser pane placeholder div (reports rect via IPC)

Browser Popup CefWindow (frameless, no taskbar entry)
  └─ Browser BrowserView (loads external URL)
  └─ positioned over the placeholder div's rect
```

**Pros:**
- Separate window = separate z-order, always visible
- Already have the `create_window` pattern in `ui_tasks.rs`
- No overlay API needed

**Cons:**
- Two windows = focus management complexity
- Window dragging: popup must follow when main window moves
- Alt+Tab shows two entries (mitigatable with WS_EX_TOOLWINDOW on Windows)

**Implementation:**
1. Use `window_create_top_level` with a frameless WindowDelegate
2. Set WS_EX_TOOLWINDOW to hide from taskbar (Windows)
3. Position tracking: ResizeObserver + window move listener → reposition
4. Focus: clicking the popup doesn't steal focus from the main window
   (set WS_EX_NOACTIVATE on Windows)

### Option B: Off-screen rendering (OSR)

Render the browser to a pixel buffer, paint it onto a `<canvas>` in the
main BrowserView's DOM. Mouse/keyboard events are forwarded from the
canvas to the off-screen browser.

**Pros:**
- No separate window, no z-order issues
- Full control over rendering position

**Cons:**
- Complex: must implement CefRenderHandler (paint callback)
- Performance: pixel copy every frame
- Input forwarding: must translate mouse/keyboard events manually
- No hardware acceleration for the embedded browser

### Option C: CEF Views layout with explicit z-ordering

Restructure the window's view hierarchy so the browser pane view is added
AFTER the main BrowserView in the layout, making it render on top.

**Cons:**
- CEF Views layout fills views to their parent — hard to position arbitrary
  child views at specific rects
- May conflict with the main BrowserView's resize behavior

### Recommendation: Option A (frameless popup)

Most practical. The pattern already exists (DevTools popup, tear-off windows).
Z-order is guaranteed by the OS window manager.

---

## Agent Browser Control API

### MCP Tools for Browser

Agents control the browser pane via MCP tools (same as AgentBus, terminal
inject, etc.):

| MCP Tool | Args | Description |
|----------|------|-------------|
| `mcp__agentmux__open_browser` | `{ url, position? }` | Open URL in browser pane (create if needed) |
| `mcp__agentmux__browser_navigate` | `{ url }` | Navigate current browser pane |
| `mcp__agentmux__browser_back` | `{}` | Go back |
| `mcp__agentmux__browser_forward` | `{}` | Go forward |
| `mcp__agentmux__browser_reload` | `{}` | Reload |
| `mcp__agentmux__browser_get_url` | `{}` | Get current URL |
| `mcp__agentmux__browser_get_title` | `{}` | Get current page title |
| `mcp__agentmux__browser_screenshot` | `{}` | Capture screenshot (base64 PNG) |
| `mcp__agentmux__browser_execute_js` | `{ script }` | Execute JavaScript in the page |
| `mcp__agentmux__browser_get_text` | `{ selector? }` | Get text content (full page or selector) |

### App API (Frontend → Host)

The frontend browser view component calls these IPC commands:

| IPC Command | Notes |
|-------------|-------|
| `browser_pane_create` | Creates popup window + BrowserView |
| `browser_pane_navigate` | `frame.load_url()` |
| `browser_pane_resize` | Reposition popup window |
| `browser_pane_close` | Close popup window |
| `browser_pane_go_back` | `browser.go_back()` |
| `browser_pane_go_forward` | `browser.go_forward()` |
| `browser_pane_reload` | `browser.reload()` |
| `browser_pane_screenshot` | CefBrowserHost::CaptureScreenshot → base64 |
| `browser_pane_exec_js` | `frame.execute_javascript()` |
| `browser_pane_get_text` | Execute JS: `document.body.innerText` |

### Agent → Browser Flow

```
Agent pane: "Open the PR on GitHub and check the status"
  → Agent calls mcp__agentmux__open_browser({ url: "https://github.com/..." })
  → Sidecar receives MCP tool call
  → Sidecar sends IPC to CEF host: browser_pane_create({ url, block_id, rect })
  → CEF host creates popup window with BrowserView
  → Page loads, agent calls browser_get_text() to read the page
  → Agent processes the content and responds
```

---

## Logging

### Browser Console Forwarding

The browser pane's `DisplayHandler::on_console_message` captures
`console.log/warn/error` from the loaded page and forwards to the
sidecar log:

```rust
impl DisplayHandler for BrowserPaneClient {
    fn on_console_message(
        &self,
        browser: &Browser,
        level: LogSeverity,
        message: &CefString,
        source: &CefString,
        line: i32,
    ) -> bool {
        tracing::info!(
            block_id = %self.block_id,
            level = ?level,
            source = %source,
            line = line,
            "browser console: {}", message
        );
        false // don't suppress
    }
}
```

### Navigation Events

Log all navigation events for debugging:

```
[browser-pane] block=abc123 navigating to https://github.com/...
[browser-pane] block=abc123 load started
[browser-pane] block=abc123 load complete (200 OK, 1.2s)
[browser-pane] block=abc123 title changed: "Pull Request #422"
[browser-pane] block=abc123 console.log: "App loaded"
```

### Access Pattern

```bash
muxlog host '[browser-pane]'     # Tail browser pane logs
muxlog host '[browser console]'  # Tail page console.log forwarding
```

---

## Implementation Plan

### PR 1: Frameless popup window (fix z-order)

1. Change `browser_panes.rs` to create a frameless popup CefWindow
   instead of `add_child_view`
2. Set WS_EX_TOOLWINDOW + WS_EX_NOACTIVATE on Windows
3. Track main window position — reposition popup on move
4. Verify: google.com renders visibly in front of the main UI

### PR 2: Navigation events + console forwarding

1. Custom CefClient for browser panes with DisplayHandler
2. Log navigation + console messages
3. IPC events back to frontend (URL change, title change)

### PR 3: Agent MCP tools

1. MCP tool definitions for browser control
2. Sidecar → CEF host IPC bridge for each tool
3. Screenshot capture via CaptureScreenshot API

### PR 4: JavaScript execution + text extraction

1. `browser_pane_exec_js` IPC command
2. `browser_pane_get_text` IPC command
3. Agent can read page content for analysis
