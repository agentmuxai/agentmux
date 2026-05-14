# SPEC: Window Title Format — `Window - Tab - AgentMux`

**Date:** 2026-05-13
**Author:** AgentX
**Status:** Draft
**Affects:** All release types — `task dev`, `task dev:standalone`, portable ZIP, installed builds, on Windows / macOS / Linux

---

## 1. Goal

Change the OS window title (the string Windows shows in the taskbar, macOS shows in the title bar, and Linux WMs show in the title bar / dock) to:

```
{Window Name} - {Tab Name} - AgentMux
```

- No version anywhere.
- Identical format across all release types and platforms.
- Most-specific identifier first so taskbar/dock truncation surfaces the useful part.

### 1.1 Examples

| Window state                                   | Title                                |
| ---------------------------------------------- | ------------------------------------ |
| Default unnamed window, "Shell" tab            | `Window 1 - Shell - AgentMux`        |
| Window with workspace "Pulse", "Logs" tab      | `Pulse - Logs - AgentMux`            |
| User-renamed instance "Debug Session", "Sysinfo" | `Debug Session - Sysinfo - AgentMux` |
| Pop-out / pool window, "Browser" tab           | `Window 2 - Browser - AgentMux`      |

---

## 2. Current Behavior (what we're replacing)

`document.title` is set in three places in `frontend/app-init.ts`:

- `app-init.ts:306` — `document.title = "AgentMux ${appVersion}"` (initial, e.g. `"AgentMux 0.33.860"`)
- `app-init.ts:485` — `document.title = "AgentMux ${appVersion} - ${initialTab.name}"` (reinitWave path)
- `app-init.ts:590` — same as 485 (initWave path)

`appVersion` is fetched once via `getApi().getAboutModalDetails().version`, which originates from `agentmux-cef/src/ipc.rs:121` (`env!("CARGO_PKG_VERSION")`).

The OS window title is then driven by CEF's `OnTitleChange` mirroring `document.title` to the native window:

- `agentmux-cef/src/client/mod.rs:107-139` — `on_title_change()`:
  - macOS / Linux: `window.set_title(title)` (CEF Views API)
  - Windows: `SetWindowTextW(hwnd, title)` (Win32) — overrides CEF's mirror with the exact `document.title`
- `agentmux-cef/src/client/handlers.rs:195-198` — DisplayHandler wrapper

**Default before document.title is written:** `index.html:7` → `<title>AgentMux</title>`.

---

## 3. Data Sources (already exist — no new schema)

The frontend already exposes the data needed for the new format. Both come from the same atoms / objects the bottom-right **InstancePanel** uses (the panel that opens when you click the version in the status bar).

### 3.1 Window Name — three-tier resolution

Lifted from `frontend/app/element/InstancePanel.tsx:142-153` (do not duplicate; extract to a shared helper):

1. **`win.meta["window:displayname"]`** — user-set instance name. Stored on the `WaveWindow` object's meta map. 64-char max (`InstancePanel.tsx:28`).

   Rename UX already exists today in the InstancePanel (the panel that opens when clicking the version chip in the status bar):
   - **Double-click** a window row → inline editor (`InstancePanel.tsx:304`)
   - **F2** while a row is focused → inline editor (`InstancePanel.tsx:316`)
   - **Enter** commits, **Esc** cancels, **blur** commits (`:349-357`)
   - Empty-after-trim silently reverts; `maxLength={64}` on the input
   - Persisted via `ObjectService.UpdateObjectMeta(makeORef("window", windowId), { "window:displayname": name })` (`:171-174`)

   This title-format change therefore inherits the existing rename UX wholesale — no new UI work for renaming.
2. **`workspace.name`** — workspace assigned to the window, when no display name is set.
3. **`"Window N"`** — 1-indexed positional fallback, where N comes from `openWindowEntriesAtom` order (`StatusBar.tsx:4`, `InstancePanel.tsx:56`). This is what unnamed windows currently show in the panel and matches user expectation.

### 3.2 Tab Name

`tab.name` — `Tab` WaveObj field (already wired through to the active-tab atoms used by `app-init.ts:485,590`). Same source the existing title uses.

### 3.3 Static suffix

Literal `"AgentMux"`. No build flavor / dev / portable suffix (per request — uniform across release types).

---

## 4. Implementation

### 4.1 New helper: `frontend/util/window-title.ts`

Extract the three-tier resolution from `InstancePanel.tsx` into a shared module so both the status-bar panel and the title computation use one implementation.

```ts
// Pseudocode shape
export function resolveWindowName(
    win: WaveWindow | undefined,
    workspace: Workspace | undefined,
    indexInOpenWindows: number,
): string {
    const display = (win?.meta?.["window:displayname"] ?? "").trim();
    if (display) return display;
    if (workspace?.name) return workspace.name;
    return `Window ${indexInOpenWindows + 1}`;
}

export function formatWindowTitle(windowName: string, tabName: string | undefined): string {
    // Tab name omitted if empty so we don't render "Foo -  - AgentMux".
    return tabName?.trim()
        ? `${windowName} - ${tabName} - AgentMux`
        : `${windowName} - AgentMux`;
}
```

Then refactor `InstancePanel.tsx:142-153` to call `resolveWindowName` so the panel and the OS title can never disagree.

### 4.2 Replace the three `document.title` writes in `app-init.ts`

Remove the version concatenation. Compute the new title from the same atoms `InstancePanel` reads (`openWindowEntriesAtom`, the per-window `WaveWindow` via `getObjectValue`, the active workspace, the active `tab.name`).

| Line | Before                                                          | After                                                |
| ---- | --------------------------------------------------------------- | ---------------------------------------------------- |
| 306  | `document.title = "AgentMux ${appVersion}"`                     | `document.title = formatWindowTitle(windowName, undefined)` (yields `Window N - AgentMux`) |
| 485  | `document.title = "AgentMux ${appVersion} - ${initialTab.name}"` | `document.title = formatWindowTitle(windowName, initialTab.name)` (yields `Window N - Tab - AgentMux`) |
| 590  | (duplicate of 485)                                              | same                                                 |

`appVersion` lookup at `app-init.ts:305` becomes dead code for title purposes — keep only if used elsewhere; otherwise drop.

### 4.3 Make the title reactive

Today the title is set once at init. The new format must update when:

- Active tab changes (`tab.name` changes, or user switches tabs)
- Window display name is renamed (`win.meta["window:displayname"]` changes)
- Workspace assigned to the window changes (affects fallback)
- Window's position in `openWindowEntriesAtom` changes (affects "Window N" fallback)

Wire a SolidJS `createEffect` (or jotai subscription, matching the existing pattern around `app-init.ts:485-590`) that recomputes and writes `document.title` whenever any of the inputs change. Unsubscribe on window close.

The reactive write replaces all three current writes — those three sites only existed because the title needed to be re-set on each init path.

### 4.4 Native side — no change

`agentmux-cef/src/client/mod.rs:107-139` already mirrors `document.title` verbatim to the OS window via `SetWindowTextW` (Windows) and `window.set_title` (macOS/Linux). Once the frontend writes the right string, the OS title is correct on every platform with no Rust change.

### 4.5 `index.html` default — change

`index.html:7`: `<title>AgentMux</title>` is shown only during the brief window before the first `document.title` write. Leave as-is — `"AgentMux"` is a reasonable transitional value and matches the suffix in the new format.

---

## 5. Release-Type × Platform Coverage

The fix is **frontend-only** and the frontend bundle is identical across every release type — so the format applies uniformly with no per-build gating, no feature flag, no platform conditional.

### 5.1 Release-type matrix

Every release path in `Taskfile.yml` is exercised:

| Release type                       | Taskfile target                | Bundle source                | Covered? |
| ---------------------------------- | ------------------------------ | ---------------------------- | -------- |
| Dev (launcher in loop, Windows)    | `task dev`                     | Vite HMR (`frontend/`)       | Yes      |
| Dev (host direct, all OS)          | `task dev:standalone`          | Vite HMR (`frontend/`)       | Yes      |
| Portable ZIP (Windows)             | `task package`                 | `frontend/dist/` (bundled)   | Yes      |
| AppImage (Linux)                   | `task package:linux`           | `frontend/dist/` (bundled)   | Yes      |
| MSIX (Windows Store)               | `task package:msix` *(TODO)*   | `frontend/dist/` (bundled)   | Auto, when implemented |
| macOS .app / .dmg                  | `task package:macos` *(TODO)*  | `frontend/dist/` (bundled)   | Auto, when implemented |
| Installed via `install-linux-desktop.sh` | (launches existing portable / AppImage build) | n/a — same bundled frontend | Yes |

The two TODO packages need no spec follow-up — they bundle the same `frontend/dist/` and route titles through the same CEF host, so they pick up the new format the day they're implemented.

### 5.2 Per-platform native title sink

`document.title` reaches the OS via different APIs on each platform. All three are already wired in `agentmux-cef/src/client/mod.rs:107-139`:

| OS              | API used                                                               | Source line               |
| --------------- | ---------------------------------------------------------------------- | ------------------------- |
| Windows         | `SetWindowTextW(hwnd, ...)` (Win32) — overrides CEF's mirror           | `client/mod.rs:130`       |
| macOS           | `window.set_title(...)` → CEF Views → AppKit `[NSWindow setTitle:]`    | `client/mod.rs:114`       |
| Linux X11       | `window.set_title(...)` → CEF Views → `_NET_WM_NAME` / `XStoreName`    | `client/mod.rs:114`       |
| Linux Wayland   | `window.set_title(...)` → CEF Views → `xdg_toplevel.set_title(...)`    | `client/mod.rs:114`       |

Whatever string `document.title` holds, all four sinks render it verbatim. **No Rust change needed.**

### 5.3 Wayland `app_id` is unrelated — leave alone

On Wayland, the *icon-matching identifier* (`xdg_toplevel.set_app_id`) is separate from the window title. AgentMux already overrides `app_id` to the literal `"agentmux"` (so GNOME/KWin/sway match the window to `agentmux.desktop`):

- `agentmux-cef/src/app.rs:175,195,215` — `install_linux_window_properties_override` writes `wayland_app_id = "agentmux"` and X11 `wm_class = "agentmux"`

This is the **icon binding**, not the user-facing title, and it must remain `"agentmux"` regardless of the title format. Confirm during testing that `xdg_toplevel.set_app_id("agentmux")` still emits — the title-format change doesn't go near this code path, but a regression check is cheap.

### 5.4 Build-flavor distinction in title — explicitly out of scope

The format is identical for dev / portable / installed builds. If a user runs `task dev` and a portable ZIP simultaneously, both windows show the same `Window N - Tab - AgentMux`. The version-button in the bottom-right status bar (`StatusBar.tsx:84`) remains the way to tell them apart.

---

## 6. Where the version still appears (out of scope, intentionally kept)

The version remains visible in places that aren't the OS window title:

- **Status bar bottom-right** — `StatusBar.tsx:84` (`v{version}` button — the entry point to InstancePanel).
- **About modal** — wherever `getAboutModalDetails().version` is rendered.
- **Per-window splash / launcher logs** — `agentmux-launcher.log` includes version stamps.

Leave these alone — they're how the user discovers the running version. The change is specifically the OS window title.

---

## 7. Test Plan

### 7.1 Manual

**Behavior (run on each release type at least once):**

- [ ] Default fresh window: title is `Window 1 - Shell - AgentMux`
- [ ] Open a 2nd window: title of 2nd is `Window 2 - <tab> - AgentMux`
- [ ] Switch tabs: title updates immediately to new `tab.name`
- [ ] Open InstancePanel (click version chip in status bar) → double-click a window row → type "Debug Session" → Enter: OS title becomes `Debug Session - <tab> - AgentMux` immediately
- [ ] Same flow but press Esc instead of Enter: OS title unchanged
- [ ] Rename to whitespace-only string: silently reverts; OS title unchanged
- [ ] Rename to a 64-char string: title shows full 64-char window name (no truncation in `document.title`; OS may visually truncate in taskbar)
- [ ] Assign a workspace whose `name` is "Pulse": title becomes `Pulse - <tab> - AgentMux`
- [ ] Tear off / pop-out a pane: pop-out window also follows the format

**Platform sweep (each renders title via a different OS API — §5.2):**

- [ ] Windows — taskbar entry shows `Window N - Tab - AgentMux` (verifies `SetWindowTextW`)
- [ ] macOS — title bar + Dock label show the same string (verifies CEF Views → AppKit)
- [ ] Linux X11 — title bar + taskbar entry show the same (verifies `_NET_WM_NAME`)
- [ ] Linux Wayland (GNOME or KWin) — title bar shows the same string AND `xdg_toplevel.set_app_id("agentmux")` is still emitted (regression check on §5.3 — icon must still bind to `agentmux.desktop`)

**Release-type sweep:**

- [ ] `task dev` (Windows) — launcher path
- [ ] `task dev:standalone` (Windows or Linux)
- [ ] `task package` → portable ZIP, extract, launch — title identical to dev (no version)
- [ ] `task package:linux` → AppImage, launch — title identical to dev (no version)

### 7.2 Automated

- [ ] Unit test for `formatWindowTitle` (omit tab when empty; otherwise three-part join)
- [ ] Unit test for `resolveWindowName` (display name → workspace → "Window N" fallback chain)
- [ ] Refactor `InstancePanel.tsx` to call `resolveWindowName`; existing visual behavior unchanged
- [ ] No existing test asserts on `document.title` format (`frontend/app/block/autotitle.test.ts` is about block titles, not window titles) — no test breakage expected

---

## 8. Open Questions

1. **What if `tab.name` is empty/whitespace?** Spec currently says fall back to `Window - AgentMux` (drop the empty middle slot). Confirm this is preferred over `Window - (untitled) - AgentMux`.

   Note on separator collisions: tab names or window names that themselves contain ` - ` will produce visually ambiguous titles (e.g. a tab literally named "Foo - Bar" yields `Window - Foo - Bar - AgentMux`). Acceptable trade-off — `|` would dodge this but the user chose `-`. If it becomes a problem, switch to ` — ` (em dash) or escape inner hyphens.
2. **Active vs window-scoped tab name.** A window has multiple tabs but only one is active at a time. The title should reflect the *active* tab in that window — confirm there's an existing per-window active-tab atom (likely yes, since tabs render correctly today).
3. **Truncation.** Windows taskbar truncates long titles aggressively. With `Window Name - Tab Name - AgentMux`, the suffix may get cut first — that's the intended priority (lose `AgentMux` before losing `Window`/`Tab`). No code action needed; just noting.

---

## 9. Files Changed (estimate)

| File                                                | Change                                                                  |
| --------------------------------------------------- | ----------------------------------------------------------------------- |
| `frontend/util/window-title.ts`                 | **NEW** — `resolveWindowName`, `formatWindowTitle`                      |
| `frontend/app-init.ts`                              | Remove 3 title writes (lines 306, 485, 590); replace with reactive effect |
| `frontend/app/element/InstancePanel.tsx`            | Refactor name resolution to call `resolveWindowName`                    |
| `frontend/util/window-title.test.ts`            | **NEW** — unit tests for the helper                                     |
| `agentmux-cef/src/client/mod.rs`                    | None                                                                    |
| `agentmux-cef/src/client/handlers.rs`               | None                                                                    |
| `agentmux-cef/src/ipc.rs`                           | None (`getAboutModalDetails().version` still serves StatusBar / About) |

Estimated diff size: ~150 lines net (most of it the new helper + tests).
