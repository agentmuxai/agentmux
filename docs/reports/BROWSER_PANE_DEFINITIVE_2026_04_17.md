# Browser Pane — Definitive Solution

**Date:** 2026-04-17

---

## Root Cause of All Failures

| Approach | Why it failed | Confirmed by |
|----------|--------------|--------------|
| iframe | X-Frame-Options blocks external sites | Chromium renderer enforcement |
| add_child_view | FillLayout expands browser to full window | Tested — google.com fills entire window |
| AddOverlayView | **Bug: browser renderer never initializes for overlay BrowserViews** | CEF issue #3790, confirmed in our logs (total: 1) |
| popup window | Creates full AgentMux instance | User rejected |

## The Only Viable Path

`add_child_view` IS the right API — it properly initializes the browser
renderer (we confirmed "Browser created (total: 2)" in an earlier build).
The problem was that the window's default FillLayout expanded the child
to fill the entire window.

**Fix:** After adding the child view, explicitly set its bounds AND prevent
the layout manager from overriding them. Two options:

### Option A: Set bounds AFTER layout

CEF's FillLayout runs during the window's layout pass. If we set bounds
AFTER the layout pass completes, our bounds will stick until the next
layout pass. Use `post_task` with a deferred bounds set:

```rust
// 1. Add child view (triggers layout — view gets full window size)
window.add_child_view(Some(&mut view));

// 2. Post a deferred task to set bounds AFTER layout
let mut bounds_task = SetBoundsTask::new(browser_view, rect);
post_task(ThreadId::UI, Some(&mut bounds_task));
```

Problem: the next layout pass (resize) will override our bounds again.

### Option B: Use FillLayout with explicit bounds override

Call `view.set_bounds()` AND `view.set_size()` after adding — the
FillLayout may only run on initial add, not on every frame.

### Option C: Replace FillLayout with no layout

Set the window's layout to None (no automatic layout), then manually
position both the main BrowserView and the pane BrowserView:

```rust
// In on_window_created, don't use FillLayout:
// window.add_child_view(main_browser_view);  // still fills because window default

// Instead, manually set bounds for both views
main_view.set_bounds(Some(&full_window_rect));
pane_view.set_bounds(Some(&pane_rect));
```

### Option D: Use a Panel with BoxLayout

Create a Panel as intermediate container, add both BrowserViews to
the Panel with explicit layout constraints:

```rust
let panel = Panel::new();
panel.set_layout(BoxLayout::new(...));
panel.add_child_view(main_browser_view);
panel.add_child_view(pane_browser_view);
window.add_child_view(panel);
```

## Recommended: Option A (deferred bounds)

Simplest change. The add_child_view approach already works for browser
creation. We just need to override the bounds after the layout pass.

The layout only re-runs on window resize — handle that by re-setting
bounds in the resize observer callback (which already runs on every
resize via the frontend's IPC).
