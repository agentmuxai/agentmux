# Spec: Full Window Transparency

**Branch:** `agent2/transparency-spec`
**Status:** Draft
**Date:** 2026-03-06
**Depends on:** PR #37 (settings widget, agentx) -- settings.json external editor access

---

## Overview

Enable true window transparency across all three platforms (Windows, macOS, Linux). AgentMux currently has a **CSS-only transparency stub** -- settings exist (`window:transparent`, `window:blur`, `window:opacity`) and the frontend applies CSS opacity, but the Tauri window itself is opaque (`transparent: false` in `tauri.conf.json`). The desktop behind the app is never visible.

This spec covers the Tauri-level and platform-specific changes needed to make the desktop actually show through.

---

## Current State (What Exists)

### Settings (already defined)

| Setting | Type | Default | Location |
|---------|------|---------|----------|
| `window:transparent` | boolean | `false` | `schema/settings.json:164` |
| `window:blur` | boolean | `false` | `schema/settings.json:167` |
| `window:opacity` | number (0-1) | `0.8` | `schema/settings.json:170` |
| `window:bgcolor` | string | `#222222` | `schema/settings.json:173` |
| `term:transparency` | number (0-1) | `0.5` | `schema/settings.json:95` (per-block) |

### Frontend CSS handling (already works)

**`app.tsx:118-146`** -- `AppSettingsUpdater` component:
- Watches `window:transparent` and `window:blur` settings
- Adds `.is-transparent` class to `#main` div
- Sets `--window-opacity` CSS variable on `<body>`
- Sets `--main-bg-color` CSS variable
- Body background uses `rgb(from var(--main-bg-color) r g b / var(--window-opacity))`

**`app.scss:32-34`** -- `.is-transparent { background-color: transparent; }`

### Backend config (already defined)

**`wconfig.rs:197-207`** -- Rust `SettingsType` struct has:
- `window_transparent: bool`
- `window_blur: bool`
- `window_opacity: Option<f64>`
- `window_bg_color: String`

### What's MISSING (the gap)

1. **Tauri window `transparent: false`** in `tauri.conf.json:21` -- hardcoded opaque
2. **No Tauri-level transparency API calls** -- `window.rs` never calls `set_transparent()` or platform effects
3. **No macOS vibrancy** -- no `NSVisualEffectView` / `window_vibrancy` crate
4. **No Windows Mica/Acrylic** -- no `DwmSetWindowAttribute` / `window_vibrancy` crate
5. **No Linux compositor hints** -- no `_NET_WM_WINDOW_TYPE` or RGBA visual setup
6. **`index.html` hardcodes `background: rgb(34, 34, 34)`** -- prevents transparent startup

---

## Design

### Architecture

```
settings.json (user sets window:transparent, window:opacity, window:blur)
    |
    v
Backend reads via wconfig.rs -> sends to frontend via RPC
    |
    +--> Frontend: AppSettingsUpdater applies CSS variables (ALREADY WORKS)
    |
    +--> Backend/Tauri: NEW -- apply Tauri window transparency + platform effects
```

### Two-Layer Approach

**Layer 1: Tauri Window Transparency (required)**
Make the webview background transparent so CSS opacity actually reveals the desktop.

**Layer 2: Platform Blur Effects (enhancement)**
Add native blur/vibrancy behind the transparent window for a polished look.

---

## Implementation

### Phase 1: Basic Transparency (All Platforms)

#### 1.1 `tauri.conf.json` -- Enable transparent window

```json
"windows": [
  {
    "transparent": true,
    "backgroundColor": "#00000000"
  }
]
```

Setting `transparent: true` tells Tauri to create the window with an alpha channel. `backgroundColor` with `00` alpha ensures the webview itself is transparent.

**Platform notes:**
- **Windows:** Requires `transparent: true` in Tauri config. WebView2 supports transparency natively.
- **macOS:** Works with `transparent: true`. WebKit webview respects alpha.
- **Linux:** Requires a compositor (X11 with compositing, or Wayland). Without a compositor, falls back to opaque.

#### 1.2 `index.html` -- Transparent startup background

Change the inline startup style from:
```css
background: rgb(34, 34, 34);
```
to:
```css
background: transparent;
```

This prevents a flash of solid color during app load when transparency is enabled.

**Caveat:** Users without transparency enabled will see a brief transparent flash. To mitigate, the frontend should apply the background color immediately in `wave.ts` init based on settings.

#### 1.3 `wave.ts` -- Apply background on init

During initialization, read `window:transparent` from settings. If false, immediately set `document.body.style.background = '#222222'` to prevent transparent flash for non-transparency users.

#### 1.4 `app.scss` -- Ensure all layers are transparent-capable

Verify no intermediate DOM elements have opaque backgrounds that would block transparency. Key elements to check:
- `html`, `body` -- controlled by CSS variables (ok)
- `#main` -- `.is-transparent` class handles this (ok)
- `.window-header`, `.tab-bar`, `.status-bar` -- may need `background: inherit` or explicit transparency
- Block/pane backgrounds -- should remain opaque by default (only the chrome around them goes transparent)

### Phase 2: Platform-Specific Blur Effects

#### 2.1 Add `window-vibrancy` crate

```toml
# src-tauri/Cargo.toml
[dependencies]
window-vibrancy = "0.5"
```

The `window-vibrancy` crate provides cross-platform blur effects via a single API.

#### 2.2 `src-tauri/src/commands/window.rs` -- New Tauri command

```rust
use window_vibrancy::{apply_vibrancy, apply_blur, apply_acrylic, apply_mica};

#[tauri::command]
pub fn set_window_transparency(
    window: tauri::WebviewWindow,
    transparent: bool,
    blur: bool,
    opacity: f64,
) {
    if !transparent && !blur {
        // Clear any applied effects
        clear_vibrancy_effects(&window);
        return;
    }

    #[cfg(target_os = "macos")]
    if blur {
        // NSVisualEffectMaterial::HudWindow for dark blur
        apply_vibrancy(&window, NSVisualEffectMaterial::HudWindow, None, None)
            .expect("Failed to apply vibrancy");
    }

    #[cfg(target_os = "windows")]
    if blur {
        // Try Mica first (Windows 11), fall back to Acrylic (Windows 10)
        if apply_mica(&window, Some(true)).is_err() {
            apply_acrylic(&window, Some((0, 0, 0, (opacity * 255.0) as u8)))
                .expect("Failed to apply acrylic");
        }
    }

    #[cfg(target_os = "linux")]
    if blur {
        // Linux blur is compositor-dependent; CSS fallback is primary
        // Some compositors support _KDE_NET_WM_BLUR_BEHIND_REGION
    }
}
```

#### 2.3 Frontend -- Call Tauri command on settings change

In `AppSettingsUpdater` (`app.tsx`), after applying CSS, also call the Tauri command:

```typescript
// After existing CSS handling
if (isTransparentOrBlur) {
    getApi().setWindowTransparency(true, windowSettings?.["window:blur"] ?? false, opacity);
} else {
    getApi().setWindowTransparency(false, false, 1.0);
}
```

#### 2.4 `tauri-api.ts` -- Add bridge function

```typescript
setWindowTransparency: (transparent: boolean, blur: boolean, opacity: number) => {
    invoke("set_window_transparency", { transparent, blur, opacity }).catch(console.error);
},
```

#### 2.5 `custom.d.ts` -- Add type declaration

```typescript
setWindowTransparency(transparent: boolean, blur: boolean, opacity: number): void;
```

#### 2.6 Capabilities -- Add permission (if needed)

The `set_window_transparency` command is a custom Tauri command, so it only needs to be registered in `lib.rs` (no capability permission needed for custom commands).

### Phase 3: New Window Support

#### 3.1 `window.rs` -- Apply transparency to new windows

When creating new windows (`create_window` command), read the current settings and apply transparency:

```rust
// After window creation
let config = config_watcher.get_settings();
if config.window_transparent || config.window_blur {
    set_window_transparency(new_window, true, config.window_blur, config.window_opacity.unwrap_or(0.8));
}
```

---

## Platform Matrix

| Feature | Windows | macOS | Linux |
|---------|---------|-------|-------|
| Basic transparency | WebView2 alpha channel | WebKit alpha | Requires compositor |
| `window:opacity` | CSS + webview bg alpha | CSS + webview bg alpha | CSS + webview bg alpha |
| `window:blur` | Mica (Win11) / Acrylic (Win10) | NSVisualEffectView vibrancy | Compositor-dependent (best-effort) |
| `window:bgcolor` | CSS variable | CSS variable | CSS variable |
| Fallback (no compositor) | N/A (always composited) | N/A (always composited) | Opaque window, CSS opacity only |

### Platform-Specific Notes

**Windows:**
- WebView2 supports `COREWEBVIEW2_COLOR` with alpha for background transparency
- Mica requires Windows 11 Build 22000+
- Acrylic works on Windows 10 1803+
- Tauri's `transparent: true` handles the WebView2 setup

**macOS:**
- `NSVisualEffectView` provides native blur behind the window
- Materials: `HudWindow` (dark), `Sidebar` (lighter), `FullScreenUI`
- Works on all supported macOS versions (10.14+)

**Linux:**
- Requires a running compositor (Picom, KWin, Mutter, Sway)
- X11: Needs RGBA visual and `_NET_WM_WINDOW_OPACITY` hint
- Wayland: Compositor handles transparency natively (if supported)
- Current code forces `GDK_BACKEND=x11` on Wayland (in `main.rs:11-17`) -- transparency may need adjustment here
- Blur is compositor-specific and not reliably available -- CSS `backdrop-filter: blur()` is the fallback

---

## Settings Integration

**Depends on PR #37** (AgentX) which adds the settings widget for opening `settings.json` in an external editor. Once merged, users can toggle transparency via:

```json
{
  "window:transparent": true,
  "window:opacity": 0.85,
  "window:blur": true,
  "window:bgcolor": "#1a1a2e"
}
```

Settings are hot-reloaded by the backend config watcher -- no restart required.

---

## Files Changed

| File | Phase | Change |
|------|-------|--------|
| `src-tauri/tauri.conf.json` | 1 | Set `transparent: true`, `backgroundColor: "#00000000"` |
| `frontend/index.html` | 1 | Transparent startup background |
| `frontend/wave.ts` | 1 | Apply solid bg for non-transparent users on init |
| `frontend/app/app.scss` | 1 | Audit intermediate elements for opaque backgrounds |
| `src-tauri/Cargo.toml` | 2 | Add `window-vibrancy` dependency |
| `src-tauri/src/commands/window.rs` | 2 | New `set_window_transparency` command |
| `src-tauri/src/lib.rs` | 2 | Register new command |
| `frontend/util/tauri-api.ts` | 2 | Add `setWindowTransparency` bridge |
| `frontend/types/custom.d.ts` | 2 | Add type declaration |
| `frontend/app/app.tsx` | 2 | Call Tauri command from `AppSettingsUpdater` |

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Transparent flash on startup for non-transparent users | `wave.ts` applies solid bg immediately from cached settings |
| Linux without compositor shows broken rendering | Detect compositor, fall back to opaque. Log warning. |
| Performance impact of blur effects | Blur is opt-in (`window:blur`). Basic transparency has minimal cost. |
| `window-vibrancy` crate compatibility with Tauri v2 | Verify crate version supports Tauri v2 before integrating |
| Wayland `GDK_BACKEND=x11` override conflicts | Test transparency under both X11 and Wayland backends |
| WebView2 transparency edge cases on older Windows | Require Windows 10 1903+ (already a Tauri requirement) |

---

## Test Plan

### Phase 1 (Basic Transparency)
- [ ] `window:transparent: true` makes desktop visible through the app on Windows
- [ ] Same on macOS
- [ ] Same on Linux (with compositor)
- [ ] `window:opacity: 0.5` shows desktop at 50% through the app
- [ ] `window:transparent: false` (default) shows solid opaque window
- [ ] No transparent flash on startup when transparency is disabled
- [ ] Terminal text remains readable at `window:opacity: 0.8`
- [ ] Block content remains on opaque backgrounds (only chrome is transparent)

### Phase 2 (Platform Blur)
- [ ] `window:blur: true` on macOS shows native vibrancy
- [ ] `window:blur: true` on Windows 11 shows Mica effect
- [ ] `window:blur: true` on Windows 10 shows Acrylic effect
- [ ] `window:blur: true` on Linux degrades gracefully (CSS blur or opaque)
- [ ] Toggling blur on/off via settings.json takes effect without restart

### Phase 3 (New Windows)
- [ ] New windows inherit transparency settings from current config
- [ ] Changing settings applies to all open windows

### Build Verification
- [ ] `npx vite build --config vite.config.tauri.ts` succeeds
- [ ] `cargo check` in `src-tauri/` succeeds
- [ ] `task package` produces working installers on all platforms
