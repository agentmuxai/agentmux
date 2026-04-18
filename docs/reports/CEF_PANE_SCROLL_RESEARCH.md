# CEF Pane Mouse-Wheel Scroll Research

**Date:** 2026-04-17
**Problem:** Typing works on embedded CEF child-HWND pane (`google.com`); mouse-wheel scrolling does not.

---

## Diagnosis (most likely cause)

**Windows routes `WM_MOUSEWHEEL` to the *focused* window, not the window under the cursor**
(pre-Vista behavior, still applies when "scroll inactive windows" is off or when the focused
window is in the same process and explicitly grabs the message). The pane can never win focus
because `agentmux-cef/src/client.rs:988-995` has a subclass that **deliberately returns 0 from
WM_SETFOCUS** unless `ALLOW_PANE_FOCUS_ONCE` was toggled by the `browser_pane_focus` IPC:

```rust
if ALLOW_PANE_FOCUS_ONCE.swap(false, Ordering::Relaxed) { /* allow */ }
else { SetFocus(GetParent(hwnd)); return 0; }   // redirects focus away
```

So the outer `CefBrowserWindow` HWND of the pane refuses focus and redirects it to its parent
(the top-level, which then routes to MAIN's `Chrome_RenderWidgetHostHWND`). `WM_MOUSEWHEEL`
is then dispatched by Windows to the focused HWND = MAIN — which ignores it because the
cursor isn't over any scrollable MAIN content. Bringing the pane to the top of Z-order
(`SetWindowPos(HWND_TOP)`) doesn't help: Z-order governs hit-testing for `WM_LBUTTONDOWN`,
not `WM_MOUSEWHEEL` delivery. This exactly matches the magpcss forum thread
["Windows mouse wheel when out of focus"](https://www.magpcss.org/ceforum/viewtopic.php?f=8&t=13520):
> *"For using the mouse wheel, I have to click back within CEF widget, so CEF gets the focus
> and the mouse wheel events are routed to it... events are routed to the topmost focused
> window which is within the hover area."*

Typing works because the per-click user-initiated `SetFocus` path sets `ALLOW_PANE_FOCUS_ONCE=true`,
letting one focus call through — but subsequent wheel events arrive *before* any click and
focus is already elsewhere.

---

## Production CEF patterns

### cefclient `osr_window_win.cc` (reference implementation)
```cpp
case WM_MOUSEWHEEL:
  POINT screen_point = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
  HWND scrolled_wnd = ::WindowFromPoint(screen_point);
  if (scrolled_wnd != hwnd_) break;           // only if cursor over us
  ScreenToClient(hwnd_, &screen_point);
  ...
  browser_host->SendMouseWheelEvent(mouse_event, deltaX, deltaY);
```
```cpp
case WM_SETFOCUS: case WM_KILLFOCUS:
  self->OnFocus(message == WM_SETFOCUS);    // forwards to host->SetFocus()
  break;
```
Crucially, cefclient **never blocks WM_SETFOCUS** — it forwards it to
`browser_->GetHost()->SetFocus(setFocus)`.

### cefclient `root_window_win.cc`
```cpp
case WM_SETFOCUS: self->OnFocus(); return 0;
void RootWindowWin::OnFocus() {
  if (browser_window_ && ::IsWindowEnabled(hwnd_))
    browser_window_->SetFocus(true);        // delegates focus DOWN to browser
}
```
Root window **passes focus down** to the browser, not up.

### cefclient `browser_window_std_win.cc` (windowed mode — our case)
```cpp
CefWindowInfo window_info;
window_info.SetAsChild(parent_handle, rect);
if (GetWindowLongPtr(parent_handle, GWL_EXSTYLE) & WS_EX_NOACTIVATE)
  window_info.ex_style |= WS_EX_NOACTIVATE;
```
In windowed/alloy mode CEF creates its own `Chrome_WidgetWin_*` HWND that owns the render
widget and handles its own `WM_MOUSEWHEEL`. **No app subclass needed** — you just must not
steal focus away.

### CefSharp WinForms `ChromiumWebBrowser.cs`
```csharp
protected override void OnGotFocus(EventArgs e) {
  if (IsBrowserInitialized) browser.GetHost().SetFocus(true);
  base.OnGotFocus(e);
}
```
CefSharp's `DefaultFocusHandler.OnSetFocus` returns `true` **only** for
`FocusSource.FocusSourceNavigation` (blocking auto-focus on page load), allowing user-
initiated focus.

### CefSharp `parentFormMessageInterceptor.Moving`
```csharp
browser?.GetHost()?.NotifyMoveOrResizeStarted();
```
Called on parent-window moves — not required for basic wheel routing, but needed so
Chromium re-computes screen coords after SetWindowPos.

---

## Recommended fix

**Remove / scope the focus-redirect subclass** and let the pane HWND own focus normally.

1. **In `client.rs` wndproc_hook:** only intercept `WM_SETFOCUS` during the narrow navigation-
   load window (mirror CefSharp: use `CefFocusHandler::OnSetFocus(FOCUS_SOURCE_NAVIGATION) → true`
   instead of a Win32 subclass). Remove the `SetFocus(parent)` redirect entirely, or gate it
   on `source == NAVIGATION` captured from the CEF focus handler, not on every WM_SETFOCUS.

2. **On pane creation, once the render HWND exists:** call
   ```rust
   host.set_focus(1);
   SetFocus(pane_outer_hwnd);
   ```
   so the pane is the focused HWND at rest. Keep `SWP_NOACTIVATE` to avoid stealing top-level
   activation.

3. **Enable auto-focus-on-hover** (optional, nicer UX — matches Wave Terminal / modern
   browsers). In the outer-pane subclass handle `WM_MOUSEWHEEL` / `WM_MOUSEMOVE` and, if the
   pane isn't focused, call `SetFocus(hwnd)` then re-post the message. This mirrors the
   "non-intrusive mouse-wheel" MSDN pattern the ceforum thread references.

4. **After every `SetWindowPos` resize** call
   `browser.GetHost().NotifyScreenInfoChanged()` (or `NotifyMoveOrResizeStarted`) so
   Chromium updates its cached screen bounds — stale bounds can also make input routing
   miss the widget even when focus is correct.

5. **Parent strategy:** keep (a) — sibling of MAIN's widget under the top-level. This is what
   cefclient `root_window_win.cc` does; parenting under MAIN's `Chrome_RenderWidgetHostHWND`
   is unsupported and will break on tab-switch because Chromium swaps that HWND. A dedicated
   container HWND (option c) is only needed if you want to mask MAIN's widget in a region
   (e.g. for popovers over it).

The `SetWindowPos(HWND_TOP)` call on resize is correct and should stay — it fixes Z-order for
`WM_LBUTTONDOWN` hit-testing. It just can't fix wheel routing on its own.

---

## References

- [cefclient/browser/osr_window_win.cc](https://github.com/chromiumembedded/cef/blob/master/tests/cefclient/browser/osr_window_win.cc) — WM_MOUSEWHEEL + SendMouseWheelEvent
- [cefclient/browser/root_window_win.cc](https://github.com/chromiumembedded/cef/blob/master/tests/cefclient/browser/root_window_win.cc) — WM_SETFOCUS → host->SetFocus()
- [cefclient/browser/browser_window_std_win.cc](https://github.com/chromiumembedded/cef/blob/master/tests/cefclient/browser/browser_window_std_win.cc) — SetAsChild + WS_EX_NOACTIVATE
- [CefSharp ChromiumWebBrowser.cs](https://github.com/cefsharp/CefSharp/blob/master/CefSharp.WinForms/ChromiumWebBrowser.cs) — OnGotFocus → SetFocus(true)
- [CefSharp DefaultFocusHandler.cs](https://github.com/cefsharp/CefSharp/blob/master/CefSharp.WinForms/Internals/DefaultFocusHandler.cs) — OnSetFocus blocks only NAVIGATION
- [magpcss: Windows mouse wheel when out of focus](https://www.magpcss.org/ceforum/viewtopic.php?f=8&t=13520) — confirms focus-routing root cause
- [CefSharp #2408 — WPF/OffScreen wheel unresponsive](https://github.com/cefsharp/CefSharp/issues/2408)
- [CEF #2438 — OSR wheel stops responding](https://github.com/chromiumembedded/cef/issues/2438)
- Current buggy code: `agentmux-cef/src/client.rs:952-1062` (`install_pane_focus_redirect`),
  `agentmux-cef/src/browser_panes.rs:74-97` (`resize`), `142-164` (`focus`).
