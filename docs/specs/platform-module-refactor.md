# Spec: Platform Module Refactor

**Date:** 2026-03-18
**Issue:** #164
**Status:** Ready to implement
**Priority:** High — macOS traffic lights regressed again from PR #161

---

## Problem

Platform-specific code is scattered across 10+ files in both the Rust backend and TypeScript frontend. Every window creation, drag handler, and UI component must independently remember platform quirks. This causes recurring regressions:

- **PR #145:** Fixed macOS resize handles by switching NSWindow to Titled, but reintroduced traffic light buttons
- **PR #161:** Removed `data-tauri-drag-region="false"` from system-status, inadvertently affected macOS traffic light positioning
- **Ongoing:** Windows custom buttons (✕ □ ─) render on macOS where they shouldn't; Linux drag handler conflicts with Tauri drag attributes

### Current Platform Check Locations

**Rust (src-tauri/):**
| File | `#[cfg]` blocks | Purpose |
|------|----------------|---------|
| `lib.rs` | 4 | NSWindow styleMask, Windows console, Linux GTK |
| `commands/window.rs` | 8 | New window creation per platform |
| `commands/drag.rs` | 4 | Tear-off window, Linux GTK drag |
| `commands/contextmenu.rs` | 3 | Linux native menu workaround |
| `commands/platform.rs` | 5 | Platform detection, macOS fullscreen |
| `commands/providers.rs` | 7 | CLI detection (where vs which) |
| `commands/file_ops.rs` | 5 | Path handling |
| `commands/cli_installer.rs` | 6 | npm/install paths |
| `sidecar.rs` | implicit | Windows Job Object, Unix orphan cleanup |
| `drag.rs` | implicit | Linux-only GTK drag module |

**TypeScript (frontend/):**
| File | Checks | Purpose |
|------|--------|---------|
| `hook/useWindowDrag.ts` | `isLinux()` | Disable drag attrs on Linux |
| `app-bg.tsx` | `PLATFORM !== PlatformMacOS` | Background rendering |
| `view/term/termwrap.ts` | 3 platform checks | Canvas renderer (Linux), scrollbar (macOS) |
| `element/quicktips.tsx` | `PlatformMacOS` | Cmd vs Alt labels |
| `window/system-status.tsx` | none (BUG) | Should hide custom buttons on macOS |

---

## Proposed Architecture

### Rust: `platform/` Module

```
src-tauri/src/
├── platform/
│   ├── mod.rs          // pub trait + dispatch
│   ├── macos.rs        // NSWindow, traffic lights, resize
│   ├── linux.rs        // GTK drag, centering, show window
│   └── windows.rs      // Job Object, console alloc
```

#### `platform/mod.rs`

```rust
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

/// Called once per window after creation.
pub fn setup_window(window: &tauri::WebviewWindow) {
    #[cfg(target_os = "macos")]
    macos::setup_window(window);
    #[cfg(target_os = "linux")]
    linux::setup_window(window);
    #[cfg(target_os = "windows")]
    windows::setup_window(window);
}

/// Called on app startup (before any windows).
pub fn setup_app(app: &tauri::App) {
    #[cfg(target_os = "macos")]
    macos::setup_app(app);
    #[cfg(target_os = "linux")]
    linux::setup_app(app);
    #[cfg(target_os = "windows")]
    windows::setup_app(app);
}
```

#### `platform/macos.rs`

```rust
use objc2_app_kit::*;
use objc2_foundation::*;

pub fn setup_window(window: &tauri::WebviewWindow) {
    let ns_window = window.ns_window().unwrap() as cocoa::id;

    // Titled + FullSizeContentView for native resize handles
    ns_window.setStyleMask_(
        NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable
        | NSWindowStyleMask::FullSizeContentView
    );

    // Hide title bar chrome
    ns_window.setTitlebarAppearsTransparent_(true);
    ns_window.setTitleVisibility_(NSWindowTitleVisibility::Hidden);

    // Hide traffic light buttons
    for button_type in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = ns_window.standardWindowButton_(button_type) {
            button.setHidden_(true);
        }
    }

    // Allow dragging by background
    ns_window.setMovableByWindowBackground_(true);
}

pub fn setup_app(_app: &tauri::App) {
    // Nothing needed at app level for macOS
}
```

#### `platform/linux.rs`

```rust
pub fn setup_window(window: &tauri::WebviewWindow) {
    // Register GTK drag handler
    register_gtk_drag(window);
    // Center if no saved position
    center_if_needed(window);
}
```

#### `platform/windows.rs`

```rust
pub fn setup_window(_window: &tauri::WebviewWindow) {
    // Nothing extra for Windows window setup currently
}

pub fn setup_app(_app: &tauri::App) {
    #[cfg(debug_assertions)]
    unsafe { windows::Win32::System::Console::AllocConsole(); }
}
```

### Frontend: Minimal — No Refactor

The frontend only has 2-3 platform checks. Not worth abstracting. Just add one guard:

#### `window/system-status.tsx`

Hide custom window buttons on macOS (macOS uses native keyboard shortcuts / app menu for close/minimize/maximize):

```tsx
<Show when={PLATFORM !== PlatformMacOS}>
    <WindowActionButtons />
</Show>
```

That's the only frontend change needed.

---

## Immediate Fix (Before Refactor)

The macOS traffic lights regression needs a hotfix NOW:

1. **Hide traffic lights in Rust:** Add `standardWindowButton().setHidden(true)` for close/minimize/zoom in the window setup code (`lib.rs` or `commands/window.rs`)
2. **Hide custom buttons on macOS in frontend:** `<Show when={PLATFORM !== PlatformMacOS}>` around `<WindowActionButtons />`

---

## Implementation Plan

### Phase 1: Create `platform/` module + immediate fix
1. Create `src-tauri/src/platform/{mod,macos,linux,windows}.rs`
2. Move window setup code from `lib.rs` into platform modules
3. Add `setup_window()` call to all window creation sites
4. Hide macOS traffic lights in `macos.rs`
5. Hide custom buttons on macOS in frontend (one `<Show>` guard)

### Phase 2: Consolidate remaining Rust `#[cfg]` blocks
1. Move window creation from `commands/window.rs` into `platform/`
2. Move tear-off window setup from `commands/drag.rs` into `platform/`
3. Move sidecar platform code (Job Object, orphan cleanup) into `platform/`
4. Move context menu workaround into `platform/`

Frontend stays as-is — not worth refactoring 2-3 checks.

---

## Files to Create/Modify

| File | Action |
|------|--------|
| `src-tauri/src/platform/mod.rs` | **Create** — dispatch to platform impl |
| `src-tauri/src/platform/macos.rs` | **Create** — NSWindow setup, hide traffic lights |
| `src-tauri/src/platform/linux.rs` | **Create** — GTK drag, centering |
| `src-tauri/src/platform/windows.rs` | **Create** — console alloc, future needs |
| `src-tauri/src/lib.rs` | **Modify** — replace inline `#[cfg]` with `platform::setup_window()` |
| `src-tauri/src/commands/window.rs` | **Modify** — call `platform::setup_window()` |
| `src-tauri/src/commands/drag.rs` | **Modify** — call `platform::setup_window()` for tear-off |
| `frontend/app/window/system-status.tsx` | **Modify** — hide WindowActionButtons on macOS |

---

## Testing Matrix

| Test | Windows | macOS | Linux |
|------|---------|-------|-------|
| Custom buttons (✕ □ ─) visible | ✓ | ✗ hidden | ✓ |
| Traffic lights visible | N/A | ✗ hidden | N/A |
| Window drag from header | ✓ data-attr | ✓ movableByBackground | ✓ GTK native |
| Window resize from edges | ✓ native | ✓ Titled style | ✓ native |
| Double-click header → maximize | ✓ | ✓ | ✓ |
| Keyboard close (Cmd-W / Alt-F4) | ✓ | ✓ | ✓ |
