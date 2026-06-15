---
type: patch
---
fix(host): eliminate ghost taskbar icon from pre-warmed pool window — apply WS_EX_TOOLWINDOW at on_window_created (reliable HWND) instead of on_after_created where BrowserHost::window_handle() can return null after page load
