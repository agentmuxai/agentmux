# Floating pane tear-off — cross-platform recipes

**Date:** 2026-05-26
**Status:** Proposed
**Extends:** `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` (Windows-only; §10 explicitly defers cross-platform)
**Pairs with:** `docs/analysis/ANALYSIS_FLOATING_PANE_TEAROFF_STATE_2026-05-26.md`

---

## 1. Why this exists

The parent spec is Windows-only by design — Phase 1 shipped a
`WS_POPUP | WS_EX_TOOLWINDOW` owned-HWND primitive in
`agentmux-cef/src/floating_pane.rs`. §10 of that spec says:

> **Out of scope:** Floater on macOS / Linux. This spec is Windows-only
> initially. macOS has its own owned-window model
> (`NSWindow.addChildWindow`); Linux compositors vary. Cross-platform
> is a follow-up.

This is the follow-up. agentmux is a tri-platform app, and shipping
floating tear-off only on Windows would split the UX. The pattern is
expressible on each platform but uses different native primitives — no
shortcut.

The target behavior is the same on every platform (per the parent
spec §1):

- Floater opens at the cursor when a pane is torn out.
- **No taskbar / Dock / app-switcher entry** for the floater.
- **Minimizes / restores with the parent window.**
- **Destroyed when the parent closes.**
- Shares the parent's backend sidecar, data dir, reducer state.
- Can be re-docked into the parent's layout (Phase 4 of the parent
  spec).

What changes per platform is the *recipe* for the outer host window.
The CEF browser embedded inside it stays the same — `CefWindowInfo::SetAsChild`
on whichever native handle the platform exposes.

## 2. Tri-platform recipe table

| Property | Windows | macOS | Linux (X11 via GTK) |
|---|---|---|---|
| **Window class** | `WS_OVERLAPPEDWINDOW \| WS_POPUP` | `NSPanel` subclass | `Gtk.Window` (toplevel) |
| **Extended style / chrome** | `WS_EX_TOOLWINDOW` | `[.nonactivatingPanel, .titled, .resizable, .closable, .utilityWindow, .fullSizeContentView]` | utility type hint |
| **Owner / parent link** | 8th arg to `CreateWindowExW` (or `SetWindowLongPtr(GWLP_HWNDPARENT)`) | `parentWindow.addChildWindow(panel, ordered: .above)` | `gtk_window_set_transient_for(child, parent)` (sets `WM_TRANSIENT_FOR`) |
| **No taskbar / Dock** | `WS_EX_TOOLWINDOW` | (Inherent — Dock is per-app, not per-window. `LSUIElement`/`NSApp.setActivationPolicy(.accessory)` is *not* what we want — that'd hide the parent's Dock entry too.) | `gtk_window_set_skip_taskbar_hint(child, TRUE)` + `gtk_window_set_skip_pager_hint(child, TRUE)` |
| **No app-switcher** | `WS_EX_TOOLWINDOW` covers Alt-Tab too | `.nonactivatingPanel` keeps it out of the parent's window-cycle order | (covered by skip-taskbar + utility hint on most WMs) |
| **Min/restore with parent** | OS auto-cascade (free with owner relationship) | `addChildWindow` auto-cascades | Most WMs cascade transient windows; explicit handling via `parent.connect("window-state-event")` if a WM doesn't |
| **Destroy with parent** | OS auto-cascade | `addChildWindow` auto-destroys | `gtk_window_set_destroy_with_parent(child, TRUE)` |
| **Stays above parent** | `WS_POPUP` + owner → above by default; no `WS_EX_TOPMOST` (global topmost is user-hostile) | `level = .floating` (or implicit via `addChildWindow ordered:.above`) | `gtk_window_set_keep_above(child, TRUE)` (best-effort; WMs may ignore) |
| **CEF embed** | `CefWindowInfo::SetAsChild(hwnd, rect)` | `CefWindowInfo::SetAsChild(nsview, rect)` | `CefWindowInfo::SetAsChild(x11_window_id, rect)` |
| **Focus chain** | Existing `install_browser_pane_focus_redirect` subclass hook | First-responder chain through child NSView; no extra plumbing typically needed | X11 input focus follows the embedded window; existing CEF handlers work |
| **DPI / scaling** | `WM_DPICHANGED` per HWND | `viewDidChangeBackingProperties` per NSView; `screen.backingScaleFactor` | `gdk_monitor_get_scale_factor`; floater receives `notify::scale-factor` when crossing displays |
| **Drag from titlebar** | `WM_NCHITTEST` returning `HTCAPTION` over draggable region (already used by main window) | Custom title bar with `mouseDown` → `[window performWindowDragWithEvent:]` | `gtk_window_begin_move_drag()` on button-press in titlebar region |

## 3. Per-platform implementation notes

### 3.1 Windows — shipped (Phase 1, reference)

See parent spec §3.1. Already implemented in
`agentmux-cef/src/floating_pane.rs::create_floating_pane_window`.
Inputs: parent HWND, geometry. Output: owned HWND, embedded CEF
browser, focus-redirect subclass installed.

Reuse this code path as the **reference implementation** the other
platforms structurally mirror.

### 3.2 macOS — `NSPanel` + `addChildWindow:ordered:`

The canonical pattern for a floating panel that's owned by a parent
window, kept out of the Dock, and bound to the parent's lifetime.

**Window creation pseudo-Swift:**

```swift
let panel = NSPanel(
    contentRect: NSRect(x: x, y: y, width: w, height: h),
    styleMask: [
        .titled,            // we draw our own title bar inside; needed for resize handles
        .closable,
        .resizable,
        .utilityWindow,     // narrower titlebar look; standard "tool window" affordance
        .nonactivatingPanel,// CRITICAL: clicking the panel doesn't take key from parent
        .fullSizeContentView
    ],
    backing: .buffered,
    defer: false
)
panel.isFloatingPanel = true
panel.collectionBehavior.insert(.fullScreenAuxiliary)
panel.titleVisibility = .hidden
panel.titlebarAppearsTransparent = true
panel.isMovableByWindowBackground = false  // we want titlebar drag, not background drag

// CEF browser into the panel's contentView
let info = CefWindowInfo()
info.setAsChild(panel.contentView, CefRect(0, 0, w, h))
CefBrowserHost.createBrowser(info, client, frontendUrl, settings)

// Establish ownership relationship
parentWindow.addChildWindow(panel, ordered: .above)
```

**Key properties of this setup**

- `addChildWindow(_:ordered:)` does the heavy lifting:
  follows-parent-on-drag, minimize/restore with parent, destroy when
  parent closes. We get the entire lifetime relationship "for free."
- `.nonactivatingPanel` is the single most important style flag — it
  keeps the parent NSWindow as `key` even when the panel is clicked.
  Without it macOS's "one key window per app" model fights us.
- Dock invisibility is **inherent** at the window level on macOS — the
  Dock shows apps, not windows. The parent's Dock entry stays;
  the panel doesn't add a second one. *Don't* try to hide the Dock
  entry app-wide via `LSUIElement` / `setActivationPolicy(.accessory)`
  — that'd hide the parent too.
- App-switcher (⌘-Tab) invisibility follows the same principle —
  ⌘-Tab cycles apps, not windows; the panel inherits the parent
  app's slot.
- Window cycling within the app (⌘-`) does not include
  `.nonactivatingPanel` panels.

**Gotchas**

- The Cocoa CEF integration historically had cross-process child-NSWindow
  issues (see [magpcss.org forum thread](https://magpcss.org/ceforum/viewtopic.php?f=6&t=19593)).
  Phase 1 macOS work must verify in-process embedding works — CEF's browser
  view becoming a subview of `panel.contentView` is the supported path.
  Multi-process variants are out of scope.
- `addChildWindow` doesn't establish input-event parent-child — it
  manages window order and lifecycle. Input goes to whichever window
  the user clicks (as usual).
- Resize during a child-attached state can be flaky; `removeChildWindow`
  + reattach when toggling float ↔ docked is the safe pattern (Phase 4
  of the parent spec implements this on re-dock).

**File where this lands:** `agentmux-cef/src/floating_pane_macos.rs`
(or `mod macos` inside the existing module — TBD).

### 3.3 Linux X11 (via GTK + Ozone-X11) — transient + utility + skip-taskbar

agentmux uses CEF with Ozone-X11 backend on Linux (Wayland is
deferred — see §4). The GTK surface for the outer window is what we
configure with X11 EWMH hints.

**Recipe (pseudo-C):**

```c
GtkWindow *floater = GTK_WINDOW(gtk_window_new(GTK_WINDOW_TOPLEVEL));
gtk_window_set_default_size(floater, w, h);
gtk_window_move(floater, x, y);

// Ownership relationship (sets WM_TRANSIENT_FOR atom)
gtk_window_set_transient_for(floater, parent_window);

// Tells the WM to treat us as a utility / tool window
gtk_window_set_type_hint(floater, GDK_WINDOW_TYPE_HINT_UTILITY);

// Stay out of the taskbar / pager (alt-tab / workspace switcher)
gtk_window_set_skip_taskbar_hint(floater, TRUE);
gtk_window_set_skip_pager_hint(floater, TRUE);

// Lifetime cascade: floater destroyed when parent destroys
gtk_window_set_destroy_with_parent(floater, TRUE);

// Best-effort topmost-relative-to-parent (WMs may ignore)
gtk_window_set_keep_above(floater, TRUE);

gtk_widget_show_all(GTK_WIDGET(floater));

// CEF browser embed using the realized X11 window id of the floater's
// GdkWindow. (Same pattern as the main window does today.)
unsigned long xid = GDK_WINDOW_XID(gtk_widget_get_window(GTK_WIDGET(floater)));
CefWindowInfo info;
info.SetAsChild(xid, CefRect{0, 0, w, h});
CefBrowserHost::CreateBrowser(info, clientHandler, frontendUrl, settings, ...);
```

**Why each flag**

- `set_transient_for` writes the `WM_TRANSIENT_FOR` ICCCM atom on the
  X11 window. Window managers use it to associate the floater with
  its parent — stacking, focus passing, and (on most WMs)
  minimize/restore cascade.
- `GDK_WINDOW_TYPE_HINT_UTILITY` maps to
  `_NET_WM_WINDOW_TYPE_UTILITY` (EWMH). WMs typically draw this with
  a narrower titlebar and float-not-tile semantics (relevant on tiling
  WMs like i3, sway, wmii).
- `skip_taskbar_hint` and `skip_pager_hint` are EWMH atoms
  (`_NET_WM_STATE_SKIP_TASKBAR`, `_NET_WM_STATE_SKIP_PAGER`) that ask
  the desktop environment to omit the window from the taskbar and the
  workspace switcher. **All mainstream WMs respect these** (GNOME,
  KDE, Xfce, Cinnamon, MATE, i3, sway, awesome, etc.).
- `set_destroy_with_parent` covers the "close cascade" cleanly within
  GTK — when the parent's GtkWindow is destroyed, the floater is too.
  Out of an abundance of caution, also listen for the parent's
  `delete-event` and close floaters explicitly.

**Gotchas**

- Min/restore cascade is NOT universally automatic on Linux — Mutter
  (GNOME) and KWin (KDE) handle it for `WM_TRANSIENT_FOR` windows;
  i3/sway/Hyprland may not. If the user reports a missed cascade,
  wire an explicit listener on the parent's `window-state-event`
  signal and call `gtk_window_iconify` / `gtk_window_deiconify` on
  floaters. Acceptable trade-off — the worst case is the floater
  stays visible when the parent minimizes, which is annoying but not
  broken.
- `keep_above` is advisory on X11 (WM may override). Don't rely on
  it for correctness; it's a UX nicety.
- Multi-monitor: GDK gives us per-monitor scale via
  `gdk_monitor_get_scale_factor()`. When the floater crosses a DPI
  boundary, we get `notify::scale-factor` on the floater's GdkWindow
  and forward to CEF (same pattern as the main window).

**File where this lands:** `agentmux-cef/src/floating_pane_linux.rs`
(GTK + GDK FFI; gtk-rs crate already in use for the main window).

### 3.4 Linux Wayland — deferred

Wayland is intentionally out of scope for this spec. The reasoning:

- CEF Wayland support is still maturing. CEF Ozone-Wayland is usable
  only in **views mode** today — the embedded-native-window path
  (`SetAsChild` on a Wayland `wl_surface`) is not yet upstream. See
  [chromiumembedded/cef#2804](https://github.com/chromiumembedded/cef/issues/2804).
- agentmux runs CEF Ozone-X11 on Linux today (`agentmux-cef`
  bootstraps with `--use-gl=desktop` + X11 backend), so Wayland
  sessions get XWayland — which gives us X11 semantics anyway and the
  recipe in §3.3 still works.
- The "right" Wayland pattern when CEF support catches up is
  `xdg_toplevel.set_parent()` plus a not-yet-standardized way to set
  no-taskbar — closest is the `xdg_decoration` server-side decoration
  protocol plus a per-compositor convention. Out of scope until CEF
  ships embedded Wayland.

Track this in a follow-up once CEF Ozone-Wayland gains
`SetAsChild` parity. Until then, X11 (under native X11 or XWayland)
covers all Linux users.

## 4. Shared cross-platform code structure

Recommend factoring the existing Windows code in
`agentmux-cef/src/floating_pane.rs` so the public API is platform-neutral
and the platform-specific guts are behind a `cfg`-gated module:

```rust
// agentmux-cef/src/floating_pane/mod.rs
pub struct FloatingPaneHandle { /* opaque handle */ }

pub fn create_floating_pane_window(
    parent_window_label: &str,
    pane_id: &str,
    rect: Rect,
) -> Result<FloatingPaneHandle, Error> {
    #[cfg(windows)]
    return platform::windows::create(parent_window_label, pane_id, rect);
    #[cfg(target_os = "macos")]
    return platform::macos::create(parent_window_label, pane_id, rect);
    #[cfg(target_os = "linux")]
    return platform::linux::create(parent_window_label, pane_id, rect);
}

mod platform {
    #[cfg(windows)] pub mod windows;
    #[cfg(target_os = "macos")] pub mod macos;
    #[cfg(target_os = "linux")] pub mod linux;
}
```

The IPC command (`open_floating_pane_window(paneId, x, y, w, h)`)
exposed to the frontend stays the same on every platform — only the
internals change.

## 5. Implementation phases (extends the parent spec)

Numbering continues from the parent spec — its Phase 1 (Windows)
shipped May 11.

| Phase | Scope | LOC est. | Risk | Notes |
|---|---|---|---|---|
| **2** (existing) | Floating-pane shell with real Block renderer (Windows-side first, but the shell is platform-neutral TS/SCSS) | ~300 | Low | No platform branching needed at the frontend level |
| **3** (existing) | Tear-off routing + `MarkPaneFloating` reducer command | ~200 | Medium | Frontend; platform-neutral |
| **C1 (new)** | macOS host primitive: `NSPanel` + `addChildWindow:ordered:` + CEF embed + focus chain | ~400 | Medium | macOS-only; mirrors Windows Phase 1 structure |
| **C2 (new)** | Linux X11 host primitive: GTK toplevel with transient + utility hint + skip-taskbar + CEF embed | ~300 | Medium | Linux-only; mirrors Windows Phase 1 structure |
| **4** (existing) | Re-dock (drag floater back into parent layout) | ~300 | Medium | Frontend; platform-neutral once Phases 2/3 land |
| **5** (existing) | Geometry persistence | ~150 | Low | Frontend; platform-neutral |
| **6** (existing) | Polish (per-pane title, escape behavior, keyboard shortcuts) | ~150 | Low | Frontend; platform-neutral |
| **W (deferred)** | Linux Wayland native embed | ~? | High | Blocked on CEF Ozone-Wayland upstreaming |

**Recommended shipping order**

1. Phases 2 + 3 (gets Windows-only MVP working as the user described).
2. C1 (macOS) — same Phase 2/3 frontend code; only the host primitive
   changes. Same code path lights up on Mac.
3. C2 (Linux X11) — same again.
4. Phases 4 + 5 + 6 — cross-platform by construction once host
   primitives exist on all three.

## 6. Acceptance criteria (additive to parent spec §11)

- [ ] Tear a pane off on **macOS** → floating panel appears at cursor,
  no Dock entry, no ⌘-Tab entry; ⌘-` does not cycle to it.
- [ ] Minimize parent NSWindow on macOS → floater hides; restore →
  floater reappears.
- [ ] Close parent NSWindow on macOS → floater destroyed.
- [ ] Tear a pane off on **Linux** (X11, GNOME / KDE / Xfce) →
  floating window appears at cursor, no taskbar entry, not in pager.
- [ ] Minimize parent GtkWindow on Linux → floater follows (best
  effort per WM — GNOME / KDE definitely; document any tiling-WM
  caveats).
- [ ] Close parent GtkWindow on Linux → floater destroyed.
- [ ] Cross-platform: dragging a floater's title bar moves it on each
  platform via the native drag primitive (`WM_NCHITTEST`,
  `performWindowDragWithEvent:`, `gtk_window_begin_move_drag`).

## 7. References

### Phase 1 + parent spec
- `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` — Windows-only base
  spec; §10 defers cross-platform to this doc.
- `agentmux-cef/src/floating_pane.rs` — Phase 1 Win32 primitive.
- `frontend/app/floating-pane/floating-pane-shell.tsx` — placeholder
  shell (Phase 2 replaces).

### Platform API references
- **Windows**: [MSDN — Window Features (owned windows)](https://learn.microsoft.com/windows/win32/winmsg/window-features),
  [WS_EX_TOOLWINDOW](https://learn.microsoft.com/windows/win32/winmsg/extended-window-styles).
- **macOS**: [`NSWindow.addChildWindow(_:ordered:)`](https://developer.apple.com/documentation/appkit/nswindow/1419152-addchildwindow),
  [NSPanel + `.nonactivatingPanel`](https://developer.apple.com/documentation/appkit/nspanel),
  [Cindori — floating panel in SwiftUI](https://cindori.com/developer/floating-panel),
  [philz.blog — NSDrawer, Child Windows, and Modern macOS Apps](https://philz.blog/nsdrawer-child-windows-and-modern-macos-applications/).
- **Linux X11 / GTK**: [`gtk_window_set_transient_for`](https://docs.gtk.org/gtk3/method.Window.set_transient_for.html),
  [`gtk_window_set_type_hint`](https://docs.gtk.org/gtk3/method.Window.set_type_hint.html),
  [`gtk_window_set_skip_taskbar_hint`](https://docs.gtk.org/gtk3/method.Window.set_skip_taskbar_hint.html),
  [EWMH `_NET_WM_WINDOW_TYPE_UTILITY`](https://specifications.freedesktop.org/wm-spec/wm-spec-1.5.html#idm45070414296368).
- **CEF cross-platform embedding**: [`CefWindowInfo::SetAsChild`](https://magpcss.org/ceforum/apidocs3/projects/(default)/CefWindowInfo.html#SetAsChild(CefWindowHandle,constCefRect%26)),
  [CEF#2804 — Wayland embedding tracking issue](https://github.com/chromiumembedded/cef/issues/2804),
  [CEF#3294 — Embedded non-Views windows](https://github.com/chromiumembedded/cef/issues/3294).

### Precedent in similar apps
- **VS Code** — Move Editor into New Window (auxiliary window, Electron),
  [microsoft/vscode#10121](https://github.com/microsoft/vscode/issues/10121).
- **Photoshop / After Effects** — palette/tool windows owned by document window.
- **Browser DevTools** — undock to floating window, owned by source browser window.
