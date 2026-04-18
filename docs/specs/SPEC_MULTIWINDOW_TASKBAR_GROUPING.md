# SPEC: Multi-Window Taskbar Behaviour — Full Instances + Sub-Windows

**Status:** Decision made — two distinct window types.
**Date:** 2026-04-17
**Owner:** AgentA
**Files:** `agentmux-cef/src/commands/window.rs`, `agentmux-cef/src/ui_tasks.rs`,
`agentmux-cef/src/client.rs`, `agentmux-cef/src/main.rs`,
`agentmux-cef/src/state.rs`, `frontend/app/status-bar/`

---

## 0. Decision (read this first)

AgentMux supports **two window types** with explicit, distinct taskbar behaviour:

| Type | Who opens it | Taskbar entry | Alt-Tab | Pinning | Close cascade |
|------|--------------|---------------|---------|---------|---------------|
| **Full instance** (default, all user-facing paths) | second `agentmux.exe` launch, status-bar version click, Ctrl+Shift+N, any future "New Window"/"New Instance" menu item | own entry, groups with siblings | yes | yes | independent |
| **Sub-window** (agent / internal only, not exposed in user UI) | Rust/backend API `open_subwindow(...)` invoked by agent operations — e.g. auxiliary view surfaced by a tool, transient panel attached to an agent task | **none** (hidden from taskbar) | yes | n/a | tied to its parent instance |

Mechanism:

1. **Process AUMID** (`AgentMuxCorp.AgentMux`) set once in `main.rs` before CEF `initialize()` — groups *full instances* under one pinned identity when the user's OS grouping setting permits.
2. **Full instances** do nothing beyond the shared AUMID. They show in the taskbar exactly like today. Every user-driven "open in new window" flow — including the status-bar version click in the bottom-right — resolves to `NewWindowMode::FullInstance`.
3. **Sub-windows** (agent-only) get `ITaskbarList::DeleteTab(hwnd)` applied after creation. They are fully functional top-level HWNDs — Alt-Tab works, they take focus, they render normally — but the shell never paints a taskbar button for them regardless of "Combine taskbar buttons" setting.
4. Sub-windows track their parent's `parent_instance_id`. When the parent closes, its sub-windows close with it.
5. There is **no user-visible UI for creating a sub-window.** The in-app "Windows (N)" switcher and `Ctrl+\`` cycle from the previous spec revision are dropped — sub-windows are not a first-class user concept. If we later need to expose sub-windows, we reintroduce the switcher at that point.

Why the asymmetry: users reason about new windows as "another AgentMux" — Chrome/VS Code semantics, own taskbar button, Alt-Tab parity. They don't need or want a hidden second window. Agents, on the other hand, may legitimately want to surface transient UI (a diff view, a confirmation prompt, a debug panel) that shouldn't pollute the taskbar and whose lifetime is tied to the agent task. Sub-windows serve that internal use case only.

---

## 1. Current State

`commands::window::open_new_window` (`window.rs:~424`) generates
`window-<uuid>`, posts `CreateWindowTask` to the CEF UI thread
(`ui_tasks.rs:160-212`). `CreateWindowTask::execute()` calls
`window_create_top_level(...)` which `CreateWindowEx`-es a standalone
top-level HWND. No AUMID, no parent relationship, no taskbar treatment
→ Explorer paints one button per HWND with whatever grouping the OS
decides.

There is currently **no distinction** between "full instance" and
"sub-window" at the API or state level. Both concepts collapse into a
single `open_new_window` command today.

---

## 2. Window Types — Full Specification

### 2.1 Full instance

- Independent conceptual scope. Own layout tree, own workspace, own
  agent identity (future). Survives other instances' closure.
- Opened by **every user-visible path**:
  - second `agentmux.exe` launch (forwarded via the existing single-instance IPC in `main.rs`)
  - status-bar version click (bottom-right)
  - `Ctrl+Shift+N`
  - any future "New Window" / "New Instance" menu item
- HWND treatment: shared process AUMID, otherwise untouched. Windows
  paints a taskbar button. Grouping follows the user's OS setting.
- Data: full `AppState.browsers` entry with `label = "main"` for the
  first instance, `label = "main-<uuid>"` for subsequent ones.
- On close: no cascade.

### 2.2 Sub-window (internal / agent-only)

- Belongs to a specific *parent instance*. Independent top-level HWND
  for OS purposes (alt-tab, focus, paint) but scoped to the parent at
  the app level.
- **Not exposed in user UI.** Only reachable via the Rust/backend API
  `open_subwindow(parent_instance_id, url, geom)`. Agent-driven tool
  calls are the intended callers (e.g. an agent pops an auxiliary view
  while a task runs).
- HWND treatment: shared process AUMID **and** `ITaskbarList::DeleteTab(hwnd)` right after `set_window_icon` in `on_after_created`.
- Data: `AppState.browsers` entry with `label = "sub-<uuid>"` and a `parent_instance_id` pointing to the owning full instance's label.
- Close cascade: when the parent full instance closes, cascade-close its sub-windows. Sub-window closed by user/agent → no cascade.
- Discovery: Alt-Tab still exposes them at OS level. **No in-app switcher in v1** — revisit if user-visible sub-windows become a product need.

### 2.3 Promotion (edge case)

If the user closes a full instance while it has sub-windows, two sane options — spec picks (a):

- **(a) Cascade close** sub-windows with the parent. Matches Electron's `parent:` option semantics. Simplest for the user. **Ship this.**
- **(b) Promote** the oldest sub-window to a full instance (`ITaskbarList::AddTab(hwnd)`, relabel, reparent others to it). More forgiving of accidental parent close but introduces taskbar pop-in UX jank.

Option (b) can be revisited if telemetry shows accidental closes are common.

---

## 3. API Design

Rename `open_new_window` to make the mode explicit.

```rust
// agentmux-cef/src/commands/window.rs
pub enum NewWindowMode {
    /// Independent AgentMux instance — own taskbar entry, own lifecycle.
    FullInstance,
    /// Sub-window of an existing instance — hidden from taskbar, dies with parent.
    Subwindow { parent_instance_id: String },
}

pub fn open_new_window(
    state: &Arc<AppState>,
    mode: NewWindowMode,
    url: &str,
    geom: Option<(i32, i32, i32, i32)>,
) -> Result<String, String>;
```

Add a new field on `AppState.browsers` entry (or a sibling map) to
record the window type + parent link:

```rust
// agentmux-cef/src/state.rs
pub enum WindowType { FullInstance, Subwindow }

pub struct WindowMeta {
    pub label: String,
    pub kind: WindowType,
    pub parent_instance_id: Option<String>, // Some only for Subwindow
    pub created_at: SystemTime,
}
pub browsers: HashMap<String, (Browser, WindowMeta)>;
```

Frontend IPC:

- `open_new_window({ url })` → maps to `NewWindowMode::FullInstance`. All user-facing triggers use this. The status-bar version-number click, `Ctrl+Shift+N`, second `agentmux.exe` launch — all resolve here.
- `open_subwindow({ url, parent_instance_id })` → `NewWindowMode::Subwindow`. **Not wired to any UI element.** Reserved for agent-tool / backend callers. If the frontend ever needs it, it goes through an agent-mediated path, not a user button.

---

## 4. Implementation

### 4.1 Process AUMID — `main.rs`, before CEF `initialize()`

```rust
#[cfg(target_os = "windows")]
unsafe {
    use windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    let aumid: Vec<u16> = "AgentMuxCorp.AgentMux\0".encode_utf16().collect();
    let _ = SetCurrentProcessExplicitAppUserModelID(aumid.as_ptr());
}
```

Cargo: `windows-sys = { ..., features = [..., "Win32_UI_Shell", "Win32_System_Com"] }`. Use a **version-stable** AUMID (no patch number) so pinning survives updates. Pin the constant in `agentmux-common`.

### 4.2 `skip_taskbar` helper — `client.rs`

Place next to `set_window_icon`:

```rust
#[cfg(target_os = "windows")]
unsafe fn skip_taskbar(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows_sys::Win32::UI::Shell::{ITaskbarList, TaskbarList};
    use windows_sys::core::{GUID, Interface};

    let mut tbl: *mut ITaskbarList = std::ptr::null_mut();
    let hr = CoCreateInstance(
        &TaskbarList as *const GUID,
        std::ptr::null_mut(),
        CLSCTX_INPROC_SERVER,
        &<ITaskbarList as Interface>::IID,
        &mut tbl as *mut _ as *mut _,
    );
    if hr < 0 || tbl.is_null() { return; }
    ((*(*tbl).lpVtbl).HrInit)(tbl);
    ((*(*tbl).lpVtbl).DeleteTab)(tbl, hwnd);
    ((*(*tbl).lpVtbl).Release)(tbl as *mut _);
}
```

### 4.3 Conditional dispatch in `on_after_created`

After `set_window_icon`:

```rust
let meta = self.state.browsers_meta.lock().get(&label).cloned();
if matches!(meta.map(|m| m.kind), Some(WindowType::Subwindow)) {
    unsafe { skip_taskbar(hwnd); }
}
```

Full instances skip this branch and appear in the taskbar normally.

### 4.4 Parent-close cascade

In the main handler's `on_before_close`, after unregistration logic:

```rust
if matches!(closing_meta.kind, WindowType::FullInstance) {
    // Close every sub-window tied to this parent.
    let children: Vec<String> = self.state.browsers_meta.lock()
        .values()
        .filter(|m| m.parent_instance_id.as_deref() == Some(&closing_meta.label))
        .map(|m| m.label.clone())
        .collect();
    for lbl in children {
        post_close_window(&self.state, &lbl);
    }
}
```

### 4.5 `TaskbarCreated` broadcast — Explorer restart survival

Listen for `RegisterWindowMessageW("TaskbarCreated")` in the top-level
window's subclassed WndProc (same trick Electron uses in
`native_window_views.cc:~335`). On receipt, re-apply `skip_taskbar` to
every HWND whose meta says `WindowType::Subwindow`.

### 4.6 Installer AUMID stamp

`.lnk` shortcuts pinned to the taskbar must carry
`System.AppUserModel.ID = AgentMuxCorp.AgentMux` in their property
store, or pinning forks. Update the Inno Setup / MSI script to stamp
this; document for manual-install flows.

---

## 5. Production Reference

| App | Strategy | Maps to our model |
|-----|----------|-------------------|
| VS Code | shared AUMID, all windows in taskbar | "new window" is always a full instance |
| Chrome | per-profile AUMID, every window in taskbar | full instance per profile; no sub-window concept |
| Slack / Discord (Electron) | shared AUMID + `setSkipTaskbar` on small overlays (mini-player, HUDs) | sub-window pattern |
| Obsidian | no AUMID set → inconsistent grouping ([forum complaints][obs]) | don't |
| Electron `BrowserWindow({ parent, skipTaskbar })` | built-in primitive for our sub-window concept | exact semantic match |

Key code:

Electron `shell/browser/native_window_views.cc:~1970`:

```cpp
void NativeWindowViews::SetSkipTaskbar(bool skip) {
  Microsoft::WRL::ComPtr<ITaskbarList> taskbar;
  if (FAILED(::CoCreateInstance(CLSID_TaskbarList, nullptr,
                                CLSCTX_INPROC_SERVER,
                                IID_PPV_ARGS(&taskbar))) ||
      FAILED(taskbar->HrInit())) return;
  if (skip) taskbar->DeleteTab(GetAcceleratedWidget());
  else      taskbar->AddTab(GetAcceleratedWidget());
}
```

Electron `shell/common/application_info_win.cc`:

```cpp
void SetAppUserModelID(const std::wstring& name) {
  SetCurrentProcessExplicitAppUserModelID(name.c_str());
}
```

VS Code `src/vs/code/electron-main/app.ts`:

```ts
if (isWindows && win32AppUserModelId) {
  app.setAppUserModelId(win32AppUserModelId);
}
```

---

## 6. Verification

1. Second `agentmux.exe` launch → **two** AgentMux taskbar buttons. With OS grouping enabled they collapse under one icon (flyout shows both); with "Never combine" they appear as two adjacent buttons — matches Chrome/VS Code behaviour, acceptable for full instances.
2. Click status-bar version (bottom-right) → third full instance opens; third taskbar button appears (or joins the grouped flyout); Alt-Tab shows 3 AgentMux windows.
3. Pin main AgentMux to taskbar, restart OS → pin survives and launches AgentMux with the shared AUMID.
4. Invoke `open_subwindow(...)` from a backend test harness or agent → no new taskbar button appears; Alt-Tab shows the sub-window; closing the parent full instance closes the sub-window.
5. Flip Settings → Personalization → Taskbar → Combine taskbar buttons between Always / When taskbar is full / Never. Full-instance count follows the setting. Sub-windows never appear in any mode.
6. Kill Explorer (`taskkill /f /im explorer.exe` + relaunch) → sub-windows remain hidden after taskbar restart (via `TaskbarCreated` re-apply).

---

## 7. Risks & Mitigations

- **Sub-window without parent.** Can't happen at the API level — `NewWindowMode::Subwindow` requires a `parent_instance_id`. Reject the IPC if the label isn't in `browsers_meta` or isn't a `FullInstance`.
- **Agent leaves sub-windows orphaned.** Mitigation: parent-close cascade closes them. Also expose a `close_all_subwindows(parent_instance_id)` internal API for agent cleanup.
- **Accessibility for sub-windows.** Screen readers enumerate taskbar buttons, so sub-windows are invisible to AT via that path. Since sub-windows are agent-driven transient UI, the agent should surface whatever content they contain through the normal AgentMux chat/log stream in parallel. Alt-Tab still exposes them at OS level.
- **Explorer restart.** `TaskbarCreated` broadcast handler re-applies `DeleteTab` to every HWND whose meta says `WindowType::Subwindow`.
- **Future user-visible sub-windows.** If product direction changes and sub-windows need to be user-created, reintroduce the status-bar "Windows (N)" switcher + `Ctrl+\`` cycle from the previous revision. The underlying mechanism is identical; only the UI surface changes.

---

## 8. References

- [Application User Model IDs][appids] — MSDN grouping semantics
- [`SetCurrentProcessExplicitAppUserModelID`][spec-mupcai]
- [`ITaskbarList` interface][itbl] — `AddTab` / `DeleteTab`
- [Windows 11 "Never combine" Q&A][ms-qa]
- Electron `shell/browser/native_window_views.cc` — `SetSkipTaskbar`
- Electron `shell/common/application_info_win.cc` — `SetAppUserModelID`
- Chromium `ui/base/win/shell.cc` — `SetAppIdForWindow`
- VS Code `src/vs/code/electron-main/app.ts` — `app.setAppUserModelId`
- AgentMux hooks: `agentmux-cef/src/client.rs:97` (`on_after_created`), `agentmux-cef/src/commands/window.rs:239` (`find_own_top_level_window`), `agentmux-cef/src/commands/window.rs:424` (`open_new_window`).

[appids]: https://learn.microsoft.com/en-us/windows/win32/shell/appids
[spec-mupcai]: https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-setcurrentprocessexplicitappusermodelid
[itbl]: https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-itaskbarlist
[ms-qa]: https://learn.microsoft.com/en-us/answers/questions/5617750/windows-11-taskbar-behaviour-never-combine
[obs]: https://forum.obsidian.md/t/opening-multiple-vaults-creates-multiple-taskbar-icons-is-this-intended-windows-11/55346
