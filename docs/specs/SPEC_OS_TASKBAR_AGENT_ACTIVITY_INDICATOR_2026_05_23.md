# SPEC: OS-level activity indicator when an agent is busy

**Date:** 2026-05-23
**Author:** AgentX
**Status:** Draft

---

## TL;DR

When at least one agent is working inside AgentMux, the OS surface for the app (Windows taskbar button, macOS dock icon, Linux launcher entry) should show an **unobtrusive, ambient activity indicator** so the user — looking at another app — can tell at a glance that something is happening. The indicator clears the moment all agents go idle. A small numeric badge shows the count of busy agents when greater than one.

The bulk of the design is **platform-native indicators**, not a custom animation: Windows `ITaskbarList3` indeterminate progress + overlay icon, macOS `NSDockTile` `badgeLabel` + optional contentView animation, Linux Unity `LauncherEntry` D-Bus `progress-visible`/`count`. No frame-by-frame icon animation — the shell already animates these surfaces for free, with correct power behavior. Rotating the app logo is **not feasible on any of the three OSes** without a CPU-burning custom redraw loop, and the platform vendors actively discourage it. The shell-driven indeterminate bar (Windows) and badge dot (macOS) are what every comparable app (Slack, Discord, VS Code, JetBrains) does.

---

## 1. The behavior we want

### 1.1 Steady state — at least one agent is busy

| Platform | Indicator |
|---|---|
| **Windows** | Taskbar button shows an **indeterminate progress bar** (animated green sweep). A small **overlay icon** sits in the lower-right corner of the taskbar button — a coloured dot (Agent-X red, by convention) when N=1, the numeral when N≥2. |
| **macOS** | Dock tile shows a `badgeLabel` — empty/dot for N=1, the number for N≥2. No dock-icon bounce, no custom drawing in the steady state. |
| **Linux (Unity LauncherEntry)** | Launcher entry shows `progress-visible: true` with `progress: 0.0` (animated indeterminate sweep in DEs that support it) and `count: N`/`count-visible: true`. |

### 1.2 Edge transitions (one-shot)

| Transition | Indicator |
|---|---|
| **Last busy agent finishes while AgentMux is in the background** | Windows: `FlashWindowEx` with `FLASHW_ALL \| FLASHW_TIMERNOFG` (clean single blink under Win11). macOS: `requestUserAttention(.informationalRequest)` — single bounce. Linux: `urgent: true` (one-shot, cleared on next focus). |
| **Agent transitions to "awaiting user input"** *(future — out of scope for v1)* | Same one-shot attention surface. |

Edge attention fires **only when the AgentMux window is not foreground**. If the user is looking at AgentMux, no flash, no bounce — they can see the agent panel directly.

### 1.3 Idle state

All indicators clear within ~200 ms of the last busy agent going idle. No residual badge, no leftover overlay icon.

---

## 2. The signal source

AgentMux already has the data we need; it just isn't aggregated globally.

### 2.1 Per-pane busy signal (today)

`frontend/app/view/agent/state.ts:51` defines `turnActiveAtom: SignalPair<boolean>` — "true from the moment the user sends a message until `session_end` arrives." This is the canonical per-agent busy flag, driven by WebSocket events from `agentmux-srv` (`session_start` / `turn_start` set it true; `session_end` sets it false). Per-pane signals are instance-scoped (`state.ts:6-9` comment: "to prevent state bleeding between multiple agent widgets").

The same module exposes `streamingStateAtom` (state.ts:46, types.ts:452-457) — a finer-grained `{ active, agentId, bufferSize, lastEventTime }` for token streaming. We do **not** use `streamingStateAtom` for the OS indicator — `turnActiveAtom` is the right granularity (whole-turn semantics, not byte-level streaming).

### 2.2 What is missing — a global "any busy" derivation

There is no global atom today. Every pane's `turnActiveAtom` is private to its instance. The OS indicator needs a single boolean ("any busy") and a count ("how many busy") for the whole process.

This spec adds a tiny store module that subscribes to every agent pane's `turnActiveAtom`, computes `(anyBusy: boolean, busyCount: number)`, and pushes changes to the host via IPC. No backend change is required.

---

## 3. The OS surfaces

### 3.1 Windows — `ITaskbarList3` (Shell COM)

Two surfaces, both per-HWND, both already supported by features that are **already enabled** in `agentmux-cef/Cargo.toml` (`Win32_UI_Shell`, `Win32_System_Com`).

- **`SetProgressState(hwnd, TBPF_INDETERMINATE)`** — shell-animated green sweep on the taskbar button. Free animation (compositor-driven, zero per-frame cost in our process). Clear with `SetProgressState(hwnd, TBPF_NOPROGRESS)`. `TBPF_PAUSED` (yellow) and `TBPF_ERROR` (red) are reserved for v2 use cases (awaiting input, agent error).
- **`SetOverlayIcon(hwnd, hicon, description)`** — 16×16 (HiDPI-scaled) icon overlay in the lower-right of the taskbar button. Clear with `SetOverlayIcon(hwnd, NULL, NULL)`. The `description` string feeds screen readers and **must** be set (e.g. `"1 agent working"`, `"3 agents working"`).

**Required gate before any call.** `ITaskbarList3` methods silently fail until the shell has created the taskbar button. We must call `RegisterWindowMessage(L"TaskbarButtonCreated")` at window creation and only invoke `ITaskbarList3` after we receive that registered message — this is non-negotiable per Microsoft Learn and is the most common reason for silent breakage.

**HWND source.** Already wired: `BrowserHost::window_handle()` (`agentmux-cef/src/window/window.rs:92`) returns the top-level HWND for a labelled window. The main window's label is `"main"`.

**No rotation API.** Windows offers no way to rotate the taskbar icon. Frame-cycled overlay icons would cost CPU and prevent the system from idling — explicitly *not* done. The indeterminate progress bar is the animation; the overlay icon is steady.

**Flash for edge transitions.** `FlashWindowEx` with `{ dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG, uCount: 3, dwTimeout: 0 }` blinks until the window comes foreground, then auto-clears. Per Microsoft, reserve for "the window requires attention but does not currently have keyboard focus" — i.e. fire only when the foreground window is not ours.

### 3.2 macOS — `NSDockTile`

- **`NSApp.dockTile.badgeLabel = "•"` / `"3"` / `nil`.** Short string in the red oval; `nil` clears it. The empty-dot convention for N=1 is `"●"` (U+25CF) or a small UI-string per HIG.
- **`NSApp.dockTile.contentView = customView` + `dockTile.display()`** is the *only* way to animate the dock icon — set a `NSView` subclass that draws the app icon plus a spinning ring overlay, and `display()` per frame. **We do not do this in v1**: the cost (CPU + GPU per frame, blocks idle) is high relative to the value, and Apple's HIG steers apps toward badge labels for ambient activity. The hook is left available for v2.
- **`NSApp.requestUserAttention(.informationalRequest)`** — single dock-icon bounce. Per the AppKit header, this is a no-op when the app is already active, which matches our "fire only when backgrounded" requirement automatically.

**No rotation API** — same as Windows. The "spinning logo" idea would have to be a custom-drawn `contentView`, which we decline in v1.

### 3.3 Linux — Unity `LauncherEntry` D-Bus

Emit a D-Bus `Update` signal on a per-app object path with signature `sa{sv}`, properties:

- `app_uri`: `"application://agentmux.desktop"` (must match the installed `.desktop` file name, which already ships with AgentMux on Linux).
- `count` (int64) + `count-visible` (bool) — number badge.
- `progress` (double 0.0–1.0) + `progress-visible` (bool) — animated progress bar. With `progress: 0.0` and `progress-visible: true`, common DEs (Ubuntu Dock, KDE Plasma) animate; vanilla GNOME Shell ignores it (limitation, documented in §7).
- `urgent` (bool) — one-shot attention flag for edge transitions.

Plus a `Query` method on the same object path returning the current state, so launchers that restart can resync.

**Crate choice:** `zbus` (pure-Rust, async, already a common dep in Rust desktop stacks). No FFI/C wrapper required.

**X11 fallback (`XUrgencyHint`).** When running under X11/XWayland *and* the desktop environment does not honour Unity LauncherEntry, set `WM_HINTS` `XUrgencyHint` for edge transitions. Use `x11rb` for this — feature-gated behind `target_os = "linux"` *and* `cfg(x11)`.

**Wayland note.** Pure-Wayland sessions without Unity-LauncherEntry support have **no equivalent** to "urgent" — `xdg-activation-v1` is a focus-passing protocol, not an attention protocol. We document this and degrade gracefully.

---

## 4. Why these indicators, why not others

| Approach | Why no |
|---|---|
| **Rotating the app logo** | No OS offers an API for this. Implementing it means a CPU/GPU-driven per-frame redraw of the taskbar/dock icon — measurable battery cost on laptops, and Apple/Microsoft HIG both explicitly counsel against it. Every comparable app (VS Code, JetBrains, Slack, Discord, Cursor) uses a badge or shell-driven progress instead. |
| **Continuous `FlashWindowEx` / `requestUserAttention(.critical)`** | These are "attention demanded" surfaces, not "background activity" surfaces. Continuous flashing/bouncing is the most disruptive thing a tray app can do. Reserve strictly for one-shot edge transitions. |
| **Custom-drawn `NSDockTile.contentView` spinner in v1** | Possible, but costs a redraw loop and ships *one* platform out of three. Defer to v2 if we decide the badge is insufficient. |
| **`TBPF_NORMAL` progress with a fake fraction** | We don't have a real fraction — agent turns aren't progress-quantifiable. `TBPF_INDETERMINATE` is semantically correct and looks exactly the same to the user. |
| **System tray icon animation** | Different surface; AgentMux doesn't ship a tray icon today and adding one is out of scope. The taskbar/dock/launcher is what every user already sees. |

---

## 5. Architecture

### 5.1 Frontend: global busy aggregator

New module: `frontend/app/store/agentActivity.ts`.

```ts
// Subscribes to every agent pane's turnActiveAtom and emits aggregate state.
export const agentActivityStore = {
    busyCount: createSignal(0),  // SolidJS signal
    anyBusy: createMemo(() => agentActivityStore.busyCount[0]() > 0),
};

// Register/unregister called by each agent pane on mount/unmount.
export function registerAgentPane(turnActiveAtom: SignalPair<boolean>): () => void { ... }
```

Each agent pane (`frontend/app/view/agent/agent-view.tsx`) calls `registerAgentPane(model.turnActiveAtom)` on mount and the returned cleanup on unmount. The store maintains a `Set<SignalPair>` and a derived count via a single `createEffect` per registration.

A second effect debounces the count signal (200 ms, trailing edge) and pushes changes to the host via the IPC defined in §5.2. Debouncing avoids flicker when a fast tool call completes between two queued turns.

### 5.2 Host IPC command

Mirror the style of existing commands (`zoom_factor`, `close_window` in `agentmux-cef/src/window/window.rs:16-100`):

```rust
// agentmux-cef/src/commands/os_indicator.rs
#[derive(Deserialize)]
pub struct SetAgentActivityArgs {
    pub window_label: String,    // typically "main"
    pub busy_count: u32,          // 0 = idle
}

pub async fn set_agent_activity(state: AppState, args: SetAgentActivityArgs) -> Result<()> { ... }
```

Frontend call (existing pattern):

```ts
await invokeCommand("set_agent_activity", { window_label: "main", busy_count });
```

One command, three platform implementations dispatched at compile time via `#[cfg(target_os = "...")]`. No new IPC events from host → frontend are required (the frontend is the single source of truth for activity state).

### 5.3 Windows implementation

`agentmux-cef/src/os_indicator/windows.rs`:

- One-time init: `CoCreateInstance(CLSID_TaskbarList, ITaskbarList3)`, store in a `OnceCell<ITaskbarList3>` behind a `Mutex` (it's apartment-threaded — pin all calls to the UI thread that owns the HWND, which is the pattern existing host code already uses).
- Register `TaskbarButtonCreated` window message at host-window creation; gate the first call until that message arrives (track a per-HWND `taskbar_ready: AtomicBool`).
- `set_agent_activity(hwnd, busy_count)`:
  - `busy_count == 0` → `SetProgressState(hwnd, TBPF_NOPROGRESS)` + `SetOverlayIcon(hwnd, NULL, NULL)`.
  - `busy_count >= 1` → `SetProgressState(hwnd, TBPF_INDETERMINATE)` + `SetOverlayIcon(hwnd, build_overlay_icon(busy_count), L"<N> agent(s) working")`.
- `build_overlay_icon(n)`: composes a 16×16 / 32×32 (HiDPI) `HICON` from a baked-in template (a coloured circle for `n==1`; the digit overlaid for `n in 2..=9`; `9+` for `n>=10`). Use `CreateIconFromResourceEx` with PNG bytes — the icons live in `agentmux-cef/resources/win/activity/*.png`. Cache `HICON`s per `n` for the session.
- `flash_if_needed(hwnd, going_idle: bool)`: if `going_idle && GetForegroundWindow() != hwnd`, call `FlashWindowEx` once. Tracked by host state; not driven by the frontend.

### 5.4 macOS implementation

`agentmux-cef/src/os_indicator/macos.rs`:

- Hold a reference to `NSApp.dockTile` via `objc2-app-kit` (already in the dependency graph for windowing).
- `set_agent_activity(_label, busy_count)`:
  - `busy_count == 0` → `dock_tile.setBadgeLabel(None)`.
  - `busy_count == 1` → `dock_tile.setBadgeLabel(Some("•"))`.
  - `busy_count >= 2` → `dock_tile.setBadgeLabel(Some(&busy_count.to_string()))`.
- `flash_if_needed(going_idle: bool)`: if `going_idle && !NSApp.isActive()`, call `requestUserAttention(.informationalRequest)`. The OS skips the bounce if the app is foreground; we still gate to avoid stale request IDs.
- The custom `contentView` spinner path is **not** wired in v1.

### 5.5 Linux implementation

`agentmux-cef/src/os_indicator/linux.rs`:

- On startup, register a `com.canonical.Unity.LauncherEntry` D-Bus name and export the object path `/com/agentmux/AgentMux` with the `Update` signal and `Query` method (per the Ubuntu LauncherAPI spec).
- `set_agent_activity(_label, busy_count)`:
  - Emit `Update` with `app_uri: "application://agentmux.desktop"`, `count: busy_count as i64`, `count-visible: busy_count >= 2`, `progress: 0.0`, `progress-visible: busy_count >= 1`, `urgent: false`.
- `flash_if_needed(going_idle: bool)`: emit a one-shot `Update` with `urgent: true` (Unity LauncherEntry edge case — the launcher clears it on next focus). On X11 sessions, also call `x11rb` to set `XUrgencyHint` on the window's `WM_HINTS`.
- Wayland-only sessions without LauncherEntry support: no-op for edge attention. Document in §7.

---

## 6. Animation and power policy

- **Steady busy state** uses only **shell-driven animation** (Windows `TBPF_INDETERMINATE`, Unity progress sweep). Zero per-frame CPU in our process.
- **macOS** has no shell-animated equivalent on the dock tile. We do **not** spin our own animation in v1 — the badge label is the indicator.
- **Edge attention** fires at most once per transition. Not a continuous loop.
- **On battery**: no behavioural change — none of the above costs meaningful power.
- **Update rate**: frontend debounce of 200 ms on busy-count changes. We never send the same `busy_count` twice in a row (the store dedupes).

---

## 7. Out of scope

- **System tray icon.** AgentMux ships no tray icon today. Adding one is a separate feature; if it lands later, it should subscribe to the same `agentActivityStore`.
- **Per-window indicators.** v1 is process-global. If AgentMux later opens multiple top-level windows with disjoint agent sets, this spec needs a small extension (route the IPC to the right `window_label`).
- **macOS animated dock contentView.** Hook is documented (§3.2); implementation is v2.
- **Wayland-without-LauncherEntry attention surface.** No protocol exists; degrade silently.
- **`TBPF_PAUSED` / `TBPF_ERROR` states.** Reserved for v2 ("agent awaiting input" → paused; "agent errored" → error). The architecture admits it; v1 only uses `INDETERMINATE` / `NOPROGRESS`.
- **Browser pane progress.** This spec is about *agent* activity. Browser pane network progress is handled separately by CEF's own UI.
- **Customisable indicator behaviour.** No setting to disable in v1. If users complain (unlikely — these are the quietest possible surfaces), add a `settings.json` opt-out later.

---

## 8. Tests

### L1 — Rust unit (`agentmux-cef/src/os_indicator/`)

- `set_agent_activity(hwnd, 0)` issues the correct `TBPF_NOPROGRESS` + `SetOverlayIcon(NULL)` sequence (mock the `ITaskbarList3` trait).
- `set_agent_activity(hwnd, 1)` issues `TBPF_INDETERMINATE` + `SetOverlayIcon(dot, "1 agent working")`.
- `set_agent_activity(hwnd, 3)` issues `TBPF_INDETERMINATE` + `SetOverlayIcon(badge_3, "3 agents working")`.
- Calls made before `TaskbarButtonCreated` is received are buffered and replayed on receipt.
- `build_overlay_icon(n)` is cached per `n` (second call hits cache).
- macOS: badge label transitions are `None` → `Some("•")` → `Some("3")` → `None`.

### L2 — Frontend unit

- `agentActivityStore.busyCount` returns the correct count across mount/unmount of multiple agent panes.
- The debouncer collapses two updates within 200 ms into one IPC call.
- The store dedupes identical successive counts.

### L3 — Integration / manual

- Spawn 1 agent → busy → Windows shows indeterminate bar + dot overlay → idle → both clear within ~200 ms.
- Spawn 3 agents simultaneously → overlay shows "3" → finish two → overlay shows "1" → finish last → all clear.
- Send the user to a different app, complete an agent turn → taskbar button blinks once (Win11 single blink) and does not continue blinking after the user focuses AgentMux.
- macOS: same flow shows dock badge `•` → `3` → `1` → clear, and a single bounce when finishing in background.
- Linux (Ubuntu, KDE Plasma): launcher shows progress sweep + count badge.
- Linux (vanilla GNOME): badge not visible (documented limitation); functionality is no-op; no crash.

### L4 — Smoke

- HiDPI Windows (200% scale): overlay icon renders crisp (32×32 source picked).
- Windows 10 and Windows 11 both render correctly (W11 flash is the cleaned-up single blink).
- Theme: switch between Windows light and dark theme — overlay icon legible on both.

---

## 9. Order of delivery

Branch: `agentx/os-taskbar-agent-activity`.

1. **Frontend store** — `frontend/app/store/agentActivity.ts` with subscribe/unsubscribe API. Per-pane `registerAgentPane` wired in `agent-view.tsx`. No IPC yet; expose the count via a debug `console.log` so we can verify it. Ship behind a `task dev` console check.
2. **Host IPC scaffold** — `set_agent_activity` command, dispatch to a per-platform stub that logs only. Verify the IPC roundtrip.
3. **Windows implementation** — `ITaskbarList3` init, `TaskbarButtonCreated` gate, `SetProgressState` + `SetOverlayIcon` calls, overlay-icon compositing. Add the three baked overlay PNGs to `agentmux-cef/resources/win/activity/`.
4. **macOS implementation** — `NSDockTile` `badgeLabel` calls via `objc2-app-kit`.
5. **Linux implementation** — `zbus` Unity LauncherEntry signal, X11 urgency-hint fallback.
6. **Edge attention** — `FlashWindowEx` / `requestUserAttention` / `urgent:true` on backgrounded → idle transitions.
7. **Tests** (§8 L1, L2) folded into the relevant commits.
8. **Manual L3/L4 verification** against `task dev` on Windows; `task package` smoke on macOS/Linux where reachable.

Each step is independently revertible. Steps 4 and 5 can land in either order; step 3 is the most user-visible win and is the priority.

---

## 10. References

### Codebase

- `frontend/app/view/agent/state.ts:46-51` — `turnActiveAtom`, `streamingStateAtom`.
- `frontend/app/view/agent/types.ts:452-457` — `streamingStateAtom` shape.
- `agentmux-cef/src/window/window.rs:16-100` — existing IPC command pattern (`zoom_factor`, `close_window`).
- `agentmux-cef/src/window/window.rs:92` — `BrowserHost::window_handle()` (HWND source).
- `agentmux-cef/Cargo.toml:54-82` — `windows` crate features already include `Win32_UI_Shell`, `Win32_System_Com`.
- `agentmux-cef/build.rs` — embeds the app `.ico` into the PE resource; no linker change needed for shell32.
- `docs/specs/SPEC_MULTIWINDOW_TASKBAR_GROUPING.md` — adjacent (taskbar grouping via AUMID and `DeleteTab`). Same HWND, same COM stack; this spec extends it.
- `assets/linux/icons/hicolor/*/apps/agentmux.png` — Linux icon set.
- `build/icons/{16,32,48,256}x{...}.png` — overlay-icon source material.

### Primary platform docs

- Microsoft Learn — [`ITaskbarList3`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-itaskbarlist3), [`SetProgressState`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itaskbarlist3-setprogressstate), [`SetProgressValue`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itaskbarlist3-setprogressvalue), [`SetOverlayIcon`](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itaskbarlist3-setoverlayicon), [`FlashWindowEx`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-flashwindowex), [Taskbar Peripheral Status sample](https://learn.microsoft.com/en-us/windows/win32/shell/samples-taskbarperipheralstatus).
- Apple Developer — [`NSDockTile`](https://developer.apple.com/documentation/appkit/nsdocktile), [`badgeLabel`](https://developer.apple.com/documentation/appkit/nsdocktile/badgelabel), [`contentView`](https://developer.apple.com/documentation/appkit/nsdocktile/contentview), [`NSApplication.requestUserAttention`](https://developer.apple.com/documentation/appkit/nsapplication/1428358-requestuserattention).
- Ubuntu Wiki — [Unity LauncherAPI](https://wiki.ubuntu.com/Unity/LauncherAPI).
- freedesktop.org — [Desktop Notifications Specification](https://specifications.freedesktop.org/notification/latest-single/).
- Wayland — [`xdg-activation-v1`](https://wayland.app/protocols/xdg-activation-v1) (focus-passing only; no urgency surface).

### Rust crates

- `windows` (Microsoft) — `Win32::UI::Shell::ITaskbarList3`, `Win32::UI::WindowsAndMessaging::FlashWindowEx`.
- `objc2-app-kit` — `NSApp.dockTile`, `requestUserAttention`.
- `zbus` — D-Bus signal/method for Unity LauncherEntry.
- `x11rb` — X11 `WM_HINTS` urgency fallback.

### Prior art

- Electron `BrowserWindow.setProgressBar(0..1)` + `app.setBadgeCount` — combined cross-platform shim. Confirms the surface choice.
- Tauri v2 `Window.setProgressBar` (issue [#4386](https://github.com/tauri-apps/tauri/issues/4386)); badging tracked at [#4489](https://github.com/tauri-apps/tauri/issues/4489).
- Slack / Discord / VS Code / JetBrains IDEs — all use shell-driven indicators (progress + badge) and never custom logo animation.
