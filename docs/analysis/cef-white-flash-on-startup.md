# CEF White Flash on Startup - Analysis & Solutions

**Date:** 2026-03-31
**Platform:** Windows 10/11, Rust CEF host (cef-rs)
**Problem:** Brief white screen visible before CEF renders dark-themed content

---

## Root Cause Analysis

The white flash has **three independent sources**, all of which contribute to the visible artifact. Fixing only one may reduce but not eliminate the flash.

### Source 1: DWM Default Window Background (Win32 layer)

When a Win32 window is first created, the Desktop Window Manager (DWM) allocates a
**default white background surface** for it. This is the surface DWM composites before
your application or any child window has painted anything. This white surface is visible
from the moment `ShowWindow` is called until the first successful
`WM_PAINT`/`WM_ERASEBKGND` cycle completes.

Even if you set `hbrBackground` in your `WNDCLASSEX` to a dark brush, there is a race:
DWM shows its default surface before your window procedure receives `WM_ERASEBKGND`.

### Source 2: Chromium's Internal White Default (Blink/Renderer layer)

Chromium's `RenderWidgetHostViewBase::background_color_` defaults to **white**. The
first frame the compositor produces uses this color. The `background_color` field in
`CefSettings` and `CefBrowserSettings` instructs CEF to override this default, but the
override only takes effect **after** the renderer process has initialized and the
`RenderWidgetHost` has been created. There is a window of time (tens to hundreds of
milliseconds) where the browser HWND exists but the renderer hasn't applied the
background color yet.

Additionally, in certain CEF versions (notably 129-131 era), `background_color` had
regressions where it stopped working entirely, though this was later determined to be
related to pages that set their own CSS background overriding the setting.

### Source 3: CEF Views Framework Hardcoded White (Views layer)

When using CEF Views mode (`CefBrowserView` + `CefWindow`), the underlying Chromium
Views framework initializes `view_view.h` with a **hardcoded white background**. The
`CefBrowserViewImpl::SetBackgroundColor()` method only changes the
`resizeBackgroundColor` (used during window resize), not the initial composition
background. This means the Views layer paints white independently of what the renderer
will eventually produce.

### Timeline of a typical launch (without fixes)

```
t=0ms    CreateWindowEx() called
         DWM shows default white surface
t=1ms    WM_ERASEBKGND received (if hbrBackground is set, paints dark here)
t=2ms    CEF child HWND created inside parent
         Child also has DWM white default
t=5ms    ShowWindow(SW_SHOW) on parent
         USER SEES WHITE (from DWM default and/or child background)
t=20ms   CEF renderer process initializes
t=50ms   Renderer creates RenderWidgetHost, applies background_color
t=80ms   First compositor frame with dark background arrives
         White flash ends
```

---

## Solutions (Ranked by Reliability)

### 1. DWM Cloaking Technique (BEST - eliminates flash entirely)

**Reliability: Highest**
**Complexity: Medium**
**Works in: Native mode and CEF Views mode**

This is the technique that Chromium itself (Chrome/Edge) adopted in 2025 to fix the
identical problem in dark mode. It completely eliminates the white flash by hiding the
window from DWM until you have painted the correct background.

#### How it works

1. After `CreateWindowEx`, **cloak** the window using `DwmSetWindowAttribute` with `DWMWA_CLOAK = TRUE`. The window exists but is invisible to the user -- DWM does not composite it.
2. Call `ShowWindow(SW_SHOW)` -- the window is "shown" in Win32 terms but still invisible because it's cloaked.
3. Use GDI to `FillRect` the entire client area with your dark background color.
4. **Uncloak** the window with `DWMWA_CLOAK = FALSE`. The user now sees the dark-painted window immediately.

#### Code example (Rust, using windows-sys)

```rust
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows_sys::Win32::Graphics::Gdi::{
    CreateSolidBrush, FillRect, GetDC, ReleaseDC, DeleteObject,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows_sys::Win32::Foundation::{BOOL, TRUE, FALSE, RECT};

/// Call this AFTER CreateWindowEx but BEFORE the window is visible to the user.
/// `hwnd` is your top-level application window handle.
unsafe fn prevent_white_flash(hwnd: isize, r: u8, g: u8, b: u8) {
    // Step 1: Cloak the window (hide from DWM compositor)
    let cloak: BOOL = TRUE;
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_CLOAK,
        &cloak as *const _ as *const _,
        std::mem::size_of::<BOOL>() as u32,
    );

    // Step 2: Show the window (Win32 state, but user can't see it yet)
    windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
        hwnd,
        windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOW,
    );

    // Step 3: Paint the client area with the desired dark color
    let hdc = GetDC(hwnd);
    if hdc != 0 {
        let brush = CreateSolidBrush(r as u32 | (g as u32) << 8 | (b as u32) << 16);
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        FillRect(hdc, &rect, brush);
        ReleaseDC(hwnd, hdc);
        DeleteObject(brush as _);
    }

    // Step 4: Uncloak -- window becomes visible with dark background
    let uncloak: BOOL = FALSE;
    DwmSetWindowAttribute(
        hwnd,
        DWMWA_CLOAK,
        &uncloak as *const _ as *const _,
        std::mem::size_of::<BOOL>() as u32,
    );
}
```

#### Notes

- `DWMWA_CLOAK` has value `13` and is available on Windows 8+.
- This addresses **Source 1** (DWM default). The CEF child HWND still has its own
  DWM surface, but if the parent is painted dark and CEF's `background_color` is set
  correctly, the child's brief white is masked by the parent's dark fill.
- The Chromium commit implementing this lives in `ui/views/win/hwnd_message_handler.cc`.

---

### 2. Win32 Window Class Background Brush + WM_ERASEBKGND (Good baseline)

**Reliability: High for parent window, partial for CEF child**
**Complexity: Low**
**Works in: Native mode and CEF Views mode**

This ensures your parent Win32 window never shows white, even without cloaking.

#### Implementation

**A) Set the window class background brush at registration time:**

```rust
use windows_sys::Win32::Graphics::Gdi::CreateSolidBrush;

let dark_brush = CreateSolidBrush(0x00222222); // RGB(34, 34, 34) in COLORREF format

let wc = WNDCLASSEXW {
    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
    hbrBackground: dark_brush,
    // ... other fields ...
};
RegisterClassExW(&wc);
```

**B) Override WM_ERASEBKGND on the CEF child's parent:**

```rust
// In your window procedure (wndproc)
match msg {
    WM_ERASEBKGND => {
        let hdc = wparam as isize;
        let mut rect: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rect);
        let brush = CreateSolidBrush(0x00222222);
        FillRect(hdc, &rect, brush);
        DeleteObject(brush as _);
        return 1; // We handled it, no further erasing needed
    }
    // ...
}
```

**C) Change the CEF browser child window's class brush after creation:**

```rust
use windows_sys::Win32::UI::WindowsAndMessaging::{SetClassLongPtrW, GCLP_HBRBACKGROUND};

// After CefBrowserHost::CreateBrowser returns and you have the browser HWND:
let browser_hwnd = browser.get_host().get_window_handle();
let dark_brush = CreateSolidBrush(0x00222222);
SetClassLongPtrW(browser_hwnd, GCLP_HBRBACKGROUND, dark_brush as isize);
```

#### Limitations

- This only addresses **Source 1** (Win32/DWM layer). The Chromium renderer's initial
  white frame (**Source 2**) will still briefly appear inside the CEF child area.
- `SetClassLongPtrW` changes the brush for ALL windows of that class, which may affect
  other CEF browser instances if you need different colors.

---

### 3. CefSettings.background_color + CefBrowserSettings.background_color (Essential but insufficient alone)

**Reliability: Medium (addresses Source 2 only)**
**Complexity: Low**
**Works in: Both modes**

This is what you already have. It is necessary but not sufficient because it only
controls the renderer's default background -- it does not affect the Win32/DWM layer
or the Views framework layer.

```rust
// In your CEF initialization:
let mut settings = cef::Settings::default();
settings.background_color = 0xFF222222; // ARGB: fully opaque, rgb(34,34,34)

// When creating each browser:
let mut browser_settings = cef::BrowserSettings::default();
browser_settings.background_color = 0xFF222222;
```

#### Important details

- Alpha must be `0xFF` (fully opaque) or `0x00` (fully transparent). No partial alpha.
- If alpha is `0x00` for a windowed browser, it falls back to `CefSettings.background_color`.
- The page's own CSS `background-color` will override this once loaded.
- The `--background-color` command-line switch (`cefclient --background-color=green`)
  is an alternative way to set the same value.

---

### 4. Create Window Hidden, Show After First Paint (Eliminates flash visually)

**Reliability: High**
**Complexity: Medium**
**Works in: Both modes (different implementation)**

Instead of trying to paint the right color before the user sees the window, hide the
window entirely until CEF has rendered its first frame.

#### Native mode (Alloy style)

```rust
// Option A: Use WindowInfo.hidden field
window_info.hidden = 1; // Create browser view initially hidden

// Option B: Create parent window without WS_VISIBLE
let hwnd = CreateWindowExW(
    0,
    class_name,
    window_title,
    WS_OVERLAPPEDWINDOW, // Note: no WS_VISIBLE
    // ...
);

// In your CefLoadHandler::OnLoadingStateChange implementation:
fn on_loading_state_change(&self, browser: &Browser, is_loading: bool, ...) {
    if !is_loading {
        // Page finished loading -- safe to show
        let hwnd = browser.get_host().get_window_handle();
        ShowWindow(hwnd, SW_SHOW);
    }
}
```

#### CEF Views mode

```rust
// In CefWindowDelegate::GetInitialShowState, return hidden:
fn get_initial_show_state(&self, window: &CefWindow) -> cef_show_state_t {
    CEF_SHOW_STATE_HIDDEN
}

// In CefLoadHandler::OnLoadingStateChange:
fn on_loading_state_change(&self, browser: &Browser, is_loading: bool, ...) {
    if !is_loading {
        // Get the CefWindow and show it
        if let Some(view) = browser.get_host().get_browser_view() {
            if let Some(window) = view.get_window() {
                window.show();
            }
        }
    }
}
```

#### Limitations

- **Perceived startup time increases.** The user sees nothing until the page loads,
  which may feel slower than seeing a dark window appear instantly.
- **Splash screen needed?** Some apps show a native splash window (painted dark) while
  CEF loads, then swap to the CEF window.
- **CEF issue #3638 warning:** On Windows, changing visibility during startup can trigger
  native window occlusion tracking bugs that result in permanently blank browser content.
  Avoid `Show()` -> `Hide()` -> `Show()` patterns. Show exactly once.

---

### 5. DWM Dark Mode Title Bar (Cosmetic improvement for title bar only)

**Reliability: High (for title bar area)**
**Complexity: Very low**
**Works in: Both modes**

This does not fix the client area flash but prevents the title bar from being bright
white on dark-themed apps.

```rust
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};

let use_dark: BOOL = TRUE;
DwmSetWindowAttribute(
    hwnd,
    DWMWA_USE_IMMERSIVE_DARK_MODE, // value = 20
    &use_dark as *const _ as *const _,
    std::mem::size_of::<BOOL>() as u32,
);
```

---

### 6. Custom User Stylesheet Injection (Legacy workaround)

**Reliability: Low**
**Complexity: Low**
**Works in: Both modes**

Older CEF versions supported `user_style_sheet_location` to inject a CSS rule like
`body { background-color: #222 !important; }` as a base64 data URL. This made Blink
apply the dark background before the page's own CSS loaded.

**This approach is deprecated** in modern CEF. The `user_style_sheet_location` setting
was removed. If needed, the same effect can be achieved via `CefRequestHandler` by
injecting a `<style>` tag into the response, but this is fragile and not recommended.

---

## Recommended Fix (Combined Approach)

For the best result, combine solutions **1 + 2 + 3**:

```
Startup sequence:
1. Register WNDCLASSEX with dark hbrBackground (Solution 2A)
2. CreateWindowEx (window exists but is white in DWM)
3. Cloak the window with DWMWA_CLOAK (Solution 1)
4. ShowWindow(SW_SHOW) -- cloaked, user sees nothing
5. FillRect entire client area with dark brush (Solution 1)
6. Uncloak -- user sees dark window instantly (Solution 1)
7. CefBrowserHost::CreateBrowser with background_color=0xFF222222 (Solution 3)
8. CEF renderer initializes and paints first frame
   (brief moment where dark GDI paint is visible, then CEF content appears)
```

This eliminates the white flash from all three sources:
- **DWM layer:** Cloaking prevents the default white from ever being visible
- **Win32 layer:** Dark brush ensures WM_ERASEBKGND paints dark
- **Renderer layer:** `background_color` ensures CEF's first frame is dark

### What about CEF Views mode specifically?

In CEF Views mode, you don't directly control the Win32 window creation. However:
- `CefSettings.background_color` still applies (Solution 3)
- You can get the native HWND from `CefWindow::GetWindowHandle()` in
  `OnWindowCreated` and apply the cloaking + brush techniques at that point
- The `GetInitialShowState` returning `CEF_SHOW_STATE_HIDDEN` approach (Solution 4)
  is an alternative if cloaking is too complex

---

## CEF Version Notes

| CEF Version | background_color works? | Notes |
|-------------|------------------------|-------|
| < 3202      | Partially              | White flash common, Views layer hardcoded white |
| 3202+       | Yes                    | Fix for Views background color propagation |
| 111         | Yes                    | Confirmed working |
| 129-131     | Yes*                   | Works, but pages with own CSS background override it |
| 132+        | Yes                    | `--background-color` CLI flag confirmed working |

*The issue #3841 reports of "not working" in 129-131 were user error -- the loaded
HTML had explicit CSS background that overrode the setting.

---

## How Other Apps Handle This

| App | Technique |
|-----|-----------|
| **Chrome/Edge** | DWM cloaking (DWMWA_CLOAK) since 2025 -- paint dark while cloaked, then uncloak |
| **Electron** | `backgroundColor` option on BrowserWindow + `ready-to-show` event to delay display |
| **Spotify** | CEF-based; users report white flash still occurs on startup (not fully solved) |
| **Discord** | Electron; uses `backgroundColor: '#2f3136'` + delayed show |
| **VS Code** | Electron; `backgroundColor` from theme + splash/loading indicator |

---

## References

- [CEF Forum: Initial black browser background color](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=11044)
- [CEF Forum: Customize browser_background_color causes white flash](https://magpcss.org/ceforum/viewtopic.php?f=7&t=15337)
- [CEF Forum: Don't show window until browser loaded](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=13544)
- [CEF Forum: Create CefBrowserWindow in initial hidden state](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=17001)
- [CEF Issue #3841: background_color not working v129-131](https://github.com/chromiumembedded/cef/issues/3841)
- [CEF Issue #3610: Runtime theme switching](https://github.com/chromiumembedded/cef/issues/3610)
- [CEF Issue #3638: Window visibility during startup](https://github.com/chromiumembedded/cef/issues/3638)
- [Chromium: hwnd_message_handler.cc DWMWA_CLOAK implementation](https://chromium.googlesource.com/chromium/src/+/bde885b4e8cf11db7ba2af6c70a6580830a52e7a/ui/views/win/hwnd_message_handler.cc)
- [Microsoft fixing Chrome/Edge white flash in dark mode](https://www.windowslatest.com/2025/01/08/microsoft-finally-fixing-chrome-edges-white-flash-in-dark-mode-on-windows-11-windows-10/)
- [Electron Issue #2172: Flash of white when window shown](https://github.com/electron/electron/issues/2172)
- [CefSharp Issue #1923: Initial background of chromium browser](https://github.com/cefsharp/CefSharp/issues/1923)
- [cef_settings_t docs (Spotify CDN)](https://cef-builds.spotifycdn.com/docs/118.7/structcef__settings__t.html)
- [cef_window_info_t docs](https://cef-builds.spotifycdn.com/docs/131.0/structcef__window__info__t.html)
- [Win32: DwmSetWindowAttribute / DWMWA_CLOAK](https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute)
- [Win32: SetClassLongPtrW / GCLP_HBRBACKGROUND](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setclasslongptrw)
- [Win32: WM_ERASEBKGND handling](https://learn.microsoft.com/en-us/windows/win32/winmsg/wm-erasebkgnd)
