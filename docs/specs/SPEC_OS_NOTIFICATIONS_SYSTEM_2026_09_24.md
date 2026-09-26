# SPEC: OS notification system (native toasts) + tray re-enablement

**Date:** 2026-09-24
**Author:** Lark
**Status:** active — Windows and Linux implemented (#3645, #3650, #3653, #3654, #3662 and the Linux/quiet-hours PR); macOS toasts not started. See §11.5 for what shipped and where it deviates from this plan.
**Verified against:** `main` @ `9cc79a624` (pulled 2026-09-24). I read the code directly and did external best-practices research. I did not build a prototype.
**Builds on:**
- [`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`](SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md): the tray, background-service mode and autostart. Issue #2977.
- [`SPEC_SOUND_NOTIFICATIONS_2026_06_05.md`](SPEC_SOUND_NOTIFICATIONS_2026_06_05.md): the in-renderer sound bus. This spec sits beside it and does not replace it.
- [`SPEC_OS_TASKBAR_AGENT_ACTIVITY_INDICATOR_2026_05_23.md`](SPEC_OS_TASKBAR_AGENT_ACTIVITY_INDICATOR_2026_05_23.md): a draft that was never implemented (taskbar overlay, flash, dock badge). This spec absorbs its "attention" half as Phase 4.
- `CLAUDE.md` "Jekt security rules". Notification content derived from jekts must follow them (§9).

## User's request (verbatim)

> get latest agentmuxai/agentmux fro github main, we want to start a notionifcations system, for the OS toasts. research best practices online. THis would also tie in to the system tray wwe worked on (which doesnt appear to be enabled enaymore) write spec to file

---

## 0. TL;DR

- **Why the tray looks gone.** No commit disabled it. It has been opt-in since #2996, behind **two environment variables read by the launcher process**: `AGENTMUX_TRAY` and `AGENTMUX_BACKGROUND_SERVICE`. There is no setting and no UI to turn it on. It is also broken for autostart: `--background` sets those variables only for the **host** child, never for the launcher itself, so an auto-started AgentMux shows no tray icon (§1.1). Phase 0 fixes both and exposes the tray as a real setting.
- **OS toasts do not exist today, and are deliberately suppressed.** The host passes Chromium `--disable-notifications`, and the macOS package strips the "Helper (Alerts)" bundle (#1659, #1713). That was the right call for the *web* Notification API. This spec does **not** revive it. Toasts are sent **natively from the launcher**, the process that already owns the tray, stays alive across host restarts, and runs a Win32 message pump.
- **Architecture.** There are three roles:
  - **Sources** emit typed `NotificationEvent`s. Mostly `srv`, plus the renderer for pane-derived events.
  - A **Router** in `srv` applies policy: categories, per-agent mute, pause/DND, coalescing, rate limits, privacy redaction. It also persists history.
  - A **Presenter** in the launcher decides at the last moment whether the user is already looking. It then shows an OS toast, updates the tray badge and menu, and handles clicks by deep-linking to the pane over the existing `open_new_window` / host IPC path.
- **Per OS:**
  - **Windows:** raw WinRT `Windows.UI.Notifications` via the `windows` crate. The installer registers the AUMID under HKCU. Clicks activate through an `agentmux://` protocol handler.
  - **macOS:** `UNUserNotificationCenter` from the launcher, which is already the bundle's main executable. **Prerequisite:** stop minting a new bundle ID per version (§6.2).
  - **Linux:** freedesktop D-Bus via `notify-rust`/zbus, or the portal under Flatpak. The tray uses `ksni`.
- **The first shipped value is two categories.** "Agent needs your input" and "agent finished" (with outcome), sent only when the user is not looking at that agent. That is Phase 1, Windows first.

---

## 1. What exists today (read from code at `9cc79a624`)

### 1.1 Tray: implemented, gated off, and the autostart path never turns it on

- It lives in `agentmux-launcher/src/tray/`:
  - `mod.rs`: the shared model. `TrayAction::{OpenWindow, Quit}` forwards `open_new_window` / `quit_app` to the host's `/ipc`. The menu has "New Window"/"Start AgentMux" and "Quit AgentMux". Liveness is polled every 5 s.
  - `windows.rs`: `tray-icon 0.24` + `muda 0.19` on a dedicated thread. This is the launcher's **only** Win32 message pump (`GetMessageW`). Status changes arrive as `WM_APP+1`.
  - `macos.rs`: no thread of its own. It is serviced by the main-thread AppKit pump.
  - **Linux:** no backend. The `start_if_enabled` arm returns `"no backend on this platform yet"`, and `ksni` is not in `Cargo.toml` despite #2996's commit message.
- **The gate** is in `tray/mod.rs:247-262`. `should_enable(tray_opt_in, background_service)` requires `AGENTMUX_TRAY` **and** `AGENTMUX_BACKGROUND_SERVICE` to be present in the launcher's own environment. `settings-template.jsonc` and `schema/settings.json` contain no `tray`, `background` or `autostart` keys.
- **Autostart bug** (verified by grep, not tested live):
  - `autostart/mod.rs:89-98` `background_env_for(args)` maps `--background` to both variables.
  - It is applied only to the host `Command` (`host_spawn.rs:58, 169`).
  - The launcher never calls `set_var` for them, and `tray::start_if_enabled` runs **in the launcher** (`supervisor/windows.rs:258-271`, `unix.rs:209-210`), so it reads `std::env` there.
  - Result: `agentmux.exe --background` from the Scheduled Task, LaunchAgent or XDG autostart gets background-service semantics in the host, but **no tray icon**. That is exactly the "invisible background service" failure the tray spec's WS4 exists to prevent.
- The only removal in the tray's history is #3013, which dropped the companion-panel menu entry at the repo owner's request.

### 1.2 Notifications that exist

| Surface | State | Where |
|---|---|---|
| In-app toast bubbles | Live. About 15 callers of `pushNotification` / `pushFlashError` | `frontend/app/store/flash-notifications.ts`, `frontend/app/notification/notificationbubbles.tsx`, mounted `app.tsx:450`. Type in `custom.d.ts:710-721` |
| Sounds | Live. 6 sound IDs; suppressed while the source pane is focused | `frontend/app/notification/sound/` (`sounds.ts`, `sound-service.ts`), `frontend/app/window/window-focus.ts` |
| "Ran while you were away" | Live | `agentmux-cef/src/background_audit.rs`, `frontend/app/init/background-audit.ts` |
| OS toasts | **None. Deliberately disabled** | `agentmux-cef/src/app/mod.rs:911-918` (`disable-notifications`), `scripts/package-macos.sh:286-293` (Alerts helper stripped), `docs/retro/retro-macos-notification-double-prompt-regression-2026-06-22.md` |
| Taskbar overlay / flash / dock badge | None. The draft spec was never built | `SPEC_OS_TASKBAR_AGENT_ACTIVITY_INDICATOR_2026_05_23.md` |

Tauri-era history: `tauri-plugin-notification` and `frontend/util/notification.ts` were deleted in #665. Nothing survives.

### 1.3 Event sources available for notifications

| Event | Where it originates today | Reaches a place a Router could see? |
|---|---|---|
| Turn ended (completed / errored / stopped / interrupted) | Renderer: `AgentPaneEvent` `turn-ended` (`agent-pane-state/types.ts`). srv: `controllerstatus.turn_active` flipping false (`blockcontroller/mod.rs:1001-1030`) | srv has only the boolean, not the outcome. The outcome exists **only in the renderer** |
| Waiting for input / question | Renderer only: derived from tool nodes with `awaiting_answer` (`useAgentQuestions.ts:100-125`). No separate permission-prompt state was found | **No.** Exists only while the pane is mounted |
| Agent failure / crash | srv: `agentfailure` (`persistent/spawn.rs`, `host_spawn.rs:672`, `container_spawn.rs:612`, `resume_retry.rs:75`); `agent:process-exited` | Yes (mps broker) |
| Jekt delivered, including `ESCALATE=required` | srv: `reactive/handler.rs:867 inject_message`, `requires_stop` at 1283-1284 | **No.** Delivery writes only an audit log (`handler.rs:1717`). No broker event |
| GitHub review / CI / merge (muxbus) | srv: `muxbus/cloud_subscriber.rs` → `inject_message` | No (same as jekts) |
| Cron fired / failed to deliver | srv: `cron/mod.rs:267`; failure only goes to `tracing::warn` | `cron_changed`, with no payload |
| Work queue changes | srv: `server/work_queue.rs` `workqueue:changed` | Yes, with no payload |
| Loop iteration | `agentmux-mcp` in-memory task | No |

The srv event bus is `backend/mps.rs` (a port of Wave's `wps`). It has one fan-out client (WebSocket, `eventbus.rs`) and **no in-process subscriber hook**. **Neither the launcher nor the host subscribes to it today.** The launcher does hold everything needed to do so: `SrvSpawnResult { ws_endpoint, auth_key }` (`srv_spawner.rs:42-47`).

### 1.4 Identity and IPC facts that constrain the design

- **Windows AUMID:**
  - `AgentMuxCorp.AgentMux` is set via `SetCurrentProcessExplicitAppUserModelID` **in the host only** (`agentmux-cef/src/lib.rs:475-485`). The launcher sets none.
  - The Inno Setup shortcuts (`packaging/windows/agentmux.iss`) carry **no** `AppUserModelID:` parameter.
  - There is no HKCU `AppUserModelId` registration.
  - MSIX packages have a package identity (`AgentMux.AgentMux_vqr1k32tkfk4y!AgentMux`), so toasts work there without extra registration, but under a **different** AUMID.
- **macOS bundle ID:** `ai.agentmux.<channel>.<version>` (`scripts/package-macos.sh:163-172`), so **each version is a new bundle ID**. Notification permission and per-app notification settings are keyed by bundle ID, so every update would re-prompt and reset the user's choices. The bundle's main executable is `agentmux-launcher` (`LSUIElement=true`), which is good: the launcher is the right process to own `UNUserNotificationCenter`.
- **Linux:** `assets/linux/agentmux.desktop`, with `StartupWMClass` rewritten per channel and version.
- **Foreground signal:** the host already reports `ReportHwndForegroundChanged { hwnd }` to the launcher over the launcher pipe (`agentmux-common/src/ipc.rs:242`, handled `launcher/src/reducer/mod.rs:299`). **The launcher already knows whether an AgentMux window is foreground.** It does not know which pane or agent is focused; only the renderer knows that (`focusManager`).
- **Custom URL scheme:** none (`agentmux://` has zero hits in code or packaging).
- **Settings:** flat `namespace:key` entries in `<instance>/config/settings.json`. Template in `settings-template.jsonc:68-83` (`notify:sounds:*` already exists). Schema wildcard `"notify:*": boolean` in `schema/settings.json`. Rust in `wconfig/types.rs`. Written via the `setconfig` RPC, which broadcasts the `config` event. UI in `frontend/app/view/settings/sections/sounds-section.tsx`, already titled "Sounds & Notifications".

---

## 2. Goals and non-goals

**Goals**
- **G1:** A user who is not looking at an agent learns, via a native OS toast, when that agent needs them or has finished, and one click lands them in that agent's pane.
- **G2:** The tray becomes a user-visible, settings-driven feature that reflects notification state: an unread/attention badge, a "needs input" submenu, and a "Pause notifications" item.
- **G3:** One policy point. Categories, per-agent mute, pause/DND, coalescing and privacy are decided once in srv, not re-implemented per surface (toast, tray, sound, in-app bubble).
- **G4:** Notifications keep working in background-service mode with zero windows open, which is exactly the case where they matter most.
- **G5:** Notification surfaces cannot be used to spoof system prompts or smuggle unverified jekt content in front of the human (§9).

**Non-goals**
- Reviving Chromium's web `Notification` API or the Alerts helper. `--disable-notifications` stays.
- Push notifications from the cloud to a closed machine (no WNS/APNs). Everything here is local; muxbus events are already pulled by srv.
- Mobile.
- Approving tool permissions **from** a toast. Actions open the pane; they never approve (§9.3).
- Replacing the in-app bubbles or sound service. They become additional Presenters fed by the same Router (Phase 3), not deleted.

---

## 3. Architecture

```
            Sources                          Router (srv)                        Presenters
  ┌───────────────────────────┐    ┌──────────────────────────────┐   ┌────────────────────────────────────┐
  │ srv: agentfailure,        │    │ notify::router               │   │ launcher: NotifyPresenter          │
  │  process-exited, turn     │──▶ │  - category/agent policy     │──▶│  - "is user looking?" final gate   │
  │  end, waiting, jekt       │    │  - pause / quiet hours       │   │  - OS toast (per-OS backend)       │
  │  ESCALATE, muxbus, cron   │    │  - dedupe/coalesce/ratelimit │   │  - tray badge + menu               │
  │ renderer: turn outcome,   │──▶ │  - redaction (§9)            │   │  - click → deep link → host IPC    │
  │  waiting-for-input (RPC)  │    │  - history (persisted)       │   ├────────────────────────────────────┤
  └───────────────────────────┘    │  - retract on resolve        │──▶│ renderer: bubbles, sounds, bell    │
                                   └──────────────────────────────┘   │ host: taskbar overlay/flash (Ph 4) │
                                     publishes mps `notification`      └────────────────────────────────────┘
```

### 3.1 Why the Router lives in srv

- srv already originates most of the events (§1.3) and is the process that stays alive in background mode.
- srv owns settings (`wconfig`), so policy reads are local.
- srv owns persistence, so history can survive restarts and feed "Ran while you were away" and the tray's unread count.
- A renderer-hosted Router would fail G4: no windows means no renderer means no toasts.

### 3.2 Why the Presenter lives in the launcher, not the host

- **The tray is already there.** Badge, menu and toast share one owner and one message pump thread (`tray/windows.rs`).
- **It survives host crash/restart and runs with zero windows.** That is the tray spec's own §2 rationale, and it applies equally here.
- **Click callbacks need a live process** (Windows in-process `Activated`, Linux D-Bus `ActionInvoked`). The launcher is the longest-lived process in the tree.
- **On macOS the launcher is the bundle's main executable,** and `UNUserNotificationCenter` requires the calling process to be the bundle.
- **It already has the window-foreground signal** from the host (§1.4).
- The host keeps what only it can do: per-HWND taskbar overlay/flash (Phase 4), and resolving "which window is showing pane X" for deep links.

### 3.3 Transport

| Link | Mechanism | New? |
|---|---|---|
| srv Sources → Router | In-process Rust call: `notify::emit(NotificationEvent)` | New module, `agentmux-srv/src/backend/notify/` |
| renderer Sources → Router | New WS RPC `notify.emit`. Accepted only for event kinds the renderer is authoritative for: `turn.ended` outcome, `input.waiting`/`input.resolved` | New RPC |
| Router → Presenters | mps broker event `notification` (plus `notification:retract`, `notification:state`), persisted for replay | New event names in `mps.rs` + `mps-events.ts` |
| launcher ↔ srv | Launcher opens a WS `eventsub` to `ws_endpoint` with `auth_key` from its own `SrvSpawnResult`. It reconnects on srv restart, the same lifetime rules `auth_key` already has | **New**: the launcher's first srv subscription |
| renderer → launcher (focus detail) | Host forwards the frontend's focused `blockId` alongside the existing `ReportHwndForegroundChanged`. New `Command::ReportFocusedBlock { hwnd, block_id: Option<String> }` | New IPC command |
| Presenter click → host | Existing authenticated `POST /ipc` → new `focus_block { block_id }` (falls back to `open_new_window` + focus) | New host IPC verb |
| Presenter → Router (user feedback) | WS RPC `notify.ack { id, how: clicked\|dismissed\|action }`, `notify.pause { until }` | New RPC |

Alternative considered: have srv call `Shell_NotifyIcon`/WinRT itself. Rejected. srv has no message pump and no tray, may run in a container or remote context later, and on macOS is not the bundle executable.

### 3.4 Event model

```rust
// agentmux-common/src/notify.rs  (shared by srv + launcher + TS mirror)
pub struct NotificationEvent {
    pub id: String,                 // ULID; stable across update/retract
    pub kind: NotifyKind,           // closed enum, below
    pub agent_id: Option<String>,   // AGENTMUX_AGENT_ID of the subject agent
    pub block_id: Option<String>,   // deep-link target
    pub group_key: String,          // coalescing key, e.g. "input:<agent_id>"
    pub priority: NotifyPriority,   // Attention | Normal | Low
    pub title_template: TitleTemplate, // app-controlled; never free text (§9)
    pub body: Option<RedactableText>,  // redacted per privacy setting (§9.2)
    pub created_at_ms: i64,
    pub expires_at_ms: Option<i64>,
}

pub enum NotifyKind {
    InputWaiting,        // agent asked a question / awaits permission  → Attention
    InputResolved,       // retracts InputWaiting (never shown itself)
    TurnCompleted,       // Normal
    TurnErrored,         // Normal (Attention if the agent is now stopped)
    AgentCrashed,        // agentfailure / unexpected process-exited     → Attention
    MessageNeedsReview,  // jekt with ESCALATE=required (§9.1)           → Attention
    ReviewArrived,       // muxbus GitHub review / CI done on agent's PR  → Normal
    CronDeliveryFailed,  // Low
    BackgroundSummary,   // coalesced digest (§5.3)                      → Low
}
```

The kind is a **closed enum**. A Source cannot invent a new visual treatment. New kinds require a code change, so each gets a review against §9.

---

## 4. Tray: re-enable and extend (Phase 0 + Phase 2)

### 4.1 Phase 0: make the existing tray reachable (small; ships first)

1. **Fix the `--background` bug.**
   - In `main.rs`, before the supervisor starts, apply `background_env_for(&args)` to the launcher's **own** environment (`std::env::set_var`, single-threaded at that point), not just the host `Command`.
   - Add a test that `--background` makes `tray::should_enable(tray_opt_in_from_env(), background_service_from_env())` true.
   - Better long term: stop reading env inside `tray::` and pass a resolved `LauncherOptions { tray, background_service }` built once in `main`. Both env and settings (next item) feed it.
2. **Add a real setting.**
   - The launcher reads `<config_dir>/settings.json` directly at startup. It starts before srv, and it already knows the data dir (see `data_paths.rs`).
   - New key: `app:runinbackground` (bool, default `false`; **changed to `true` on 2026-09-25** by the repo owner, see the tray spec §6). "Keep AgentMux running in the system tray when all windows are closed."
   - It implies both flags, preserving the tray spec's deliberate pairing (§1.1 rationale at `tray/mod.rs:240-246`: a tray without background mode would lie).
   - The env vars stay as developer overrides.
   - Changing it requires a restart in v1. Surface that in the settings UI. Live toggling can follow once the launcher is subscribed to srv's `config` event (§3.3).
3. **Settings UI:** a "System tray & background" section in settings, with the toggle above plus the existing autostart (`--enable-autostart`/`--disable-autostart`) exposed as a second, **separate** toggle. The tray spec §7.4 requires those two stay separate.
4. **Windows 11 overflow:**
   - New tray icons land hidden in the overflow flyout, and there is no supported way to promote them programmatically.
   - On first enable, show a one-time in-app hint pointing at *Settings → Personalization → Taskbar → Other system tray icons*.
   - Use a stable `NIF_GUID`. The shell ties it to the exe path, so this relies on the install path being version-independent. Verify that this is true for the Inno install layout before relying on it.
5. **Linux `ksni` backend** (unchanged from the tray spec §7.6).
   - Use `tray-icon` with `default-features = false, features = ["ksni"]` (tray-icon 0.25+), or `ksni` directly.
   - Put everything in the menu: some SNI hosts drop left-click, and GNOME's extension shows no tooltips.
   - Document that GNOME needs the AppIndicator extension.

### 4.2 Phase 2: tray as the notification hub

- **Icon state variants:** `idle`, `working` (≥1 agent turn active), `attention` (≥1 unresolved `Attention` notification), `paused` (notifications paused). Ship them as pre-rendered icon sets:
  - Windows: `.ico` resources.
  - macOS: template PNGs (monochrome, so `attention` uses a dot glyph baked into the template, not color).
  - Linux: SNI `IconPixmap`, or `AttentionIconPixmap` + `Status=NeedsAttention`, which SNI provides natively.
- **Tooltip** (Windows/macOS): "AgentMux: 2 agents need input · 5 running". Never the only carrier of the information (accessibility, and GNOME has no tooltips).
- **Menu:**
  ```
  ● 2 agents need input            ▸  lark: "Which branch should I…"   (redacted per §9.2)
                                        korp: permission requested
  New Window
  ─────────────
  Pause notifications              ▸  30 minutes / 1 hour / Until tomorrow / Resume
  Notification settings…
  ─────────────
  Quit AgentMux
  ```
  Selecting an agent entry is the same action as clicking its toast (§7).
- **Left-click:**
  - Windows: unchanged (open a window). If there are unresolved `Attention` items, focus the oldest one's pane instead of opening a blank window.
  - macOS: shows the menu (HIG).
- The companion panel stays out of the menu, per #3013.

---

## 5. Router policy (srv, `backend/notify/router.rs`)

### 5.1 Decision pipeline (order matters)

1. **Kind enabled?** `notify:os:<kind>` setting (§8). A disabled kind still records to history for the in-app bell, unless `notify:history:<kind>` is also off.
2. **Agent muted?** `notify:mute:<agent_id>` (set from the pane menu or the toast's "Mute this agent" action).
3. **Paused / quiet hours?**
   - `notify:pause:until` (epoch ms, set from the tray) and optional `notify:quiethours` (`"22:00-08:00"`).
   - While paused, events go to history only. `Attention` kinds optionally break through (`notify:pause:allowattention`, default `false`).
4. **Dedupe:** same `group_key` with an unresolved notification means **update in place** (§5.3). No second toast.
5. **Delay gate (debounce against "I was about to handle it"):**
   - `InputWaiting` is held **6 s**. `TurnCompleted` is held **10 s**; the matching Claude Code defaults are ~6 s for permission and ~60 s for idle, but ours is scoped to "not looking", so it can be shorter.
   - If an `InputResolved` or a new `turn-started` arrives inside the window, drop it silently.
   - Delays are settings (`notify:delay:<kind>` in ms).
6. **Rate limit:** at most **1 toast per agent per 30 s** and **4 toasts per minute globally**. Excess events fold into a `BackgroundSummary` ("3 more agents finished") that replaces itself (same tag). Bursts from a fleet broadcast or a `FleetBulkStop` must not produce 20 toasts.
7. **Redact** (§9.2), persist, publish `notification`.

The Router **does not** decide whether the user is looking. It cannot know it in time: focus changes faster than the WS round trip. That is the Presenter's final gate (§6.0).

### 5.2 Retraction

When the condition clears, publish `notification:retract { id }`. Examples: `InputResolved`, the user opened the pane, the agent restarted after a crash, `ack` from any surface. Presenters remove the OS toast from the Action Center / Notification Center / D-Bus (`ToastNotificationHistory.Remove(tag, group)`, `removeDeliveredNotifications`, `CloseNotification`) and update the tray badge. A stale "needs input" toast for a question already answered in-app is the single most annoying failure mode seen in comparable tools; retraction is **not optional** for `InputWaiting`.

### 5.3 Coalescing, replace and update

- **Windows:** `Tag = group_key` (max 64 chars; hash if longer) and `Group = agent_id`. A new toast with the same Tag+Group replaces the old one.
- **macOS:** reuse the notification request identifier (`group_key`) to replace; `threadIdentifier = agent_id` groups per agent.
- **Linux:** keep `group_key → server id` and pass it as `replaces_id`.
- **History:** a small table in srv's existing store (`notify_history`: id, kind, agent_id, block_id, group_key, title, body_redacted, created, resolved_at, acked_how). Cap at 500 rows / 14 days. This, not the OS Action Center (~20 per app on Windows, user-clearable), is the source of truth for unread counts.

---

## 6. Presenter (launcher, `agentmux-launcher/src/notify/`)

### 6.0 Final gate: "is the user already looking?"

A toast is suppressed only if an AgentMux window is foreground **and** its focused block is the notification's `block_id`. It is still sent when the foreground window shows a different pane, and at `Attention` priority that toast should use a short-lived banner.

Then:
- An in-app bubble plus a pane tab badge is enough for `Normal`/`Low` when *any* AgentMux window is foreground. This is VS Code's `windowNotFocused` default and Warp's model: in-app attention badges when focused, OS toasts only when the app is in the background.
- If AgentMux is not foreground, or has no windows (background mode), show the OS toast.
- Setting `notify:os:when` = `"unfocused"` (default) | `"always"` | `"never"`.

Also hold back (queue, don't drop) while the OS reports a presentation or full-screen state. On Windows: `SHQueryUserNotificationState` ≠ `QUNS_ACCEPTS_NOTIFICATIONS`. Beyond that, **do not try to detect or override OS Do Not Disturb/Focus**. The OS already routes our toasts to the notification center silently; that is the correct behavior. The detection APIs are used only to stop our *own* sounds and taskbar flashing.

### 6.1 Windows backend

- **Crate:** `windows` (windows-rs) with `UI_Notifications`, `Data_Xml_Dom` and `Foundation` features, using the plain WinRT `ToastNotificationManager`. Not the Windows App SDK `AppNotificationManager`:
  - Microsoft archived the `windows-app` Rust crate.
  - `Register()` still fails (0x8007007E) for self-contained unpackaged apps on SDK 2.5.x (microsoft/WindowsAppSDK#6774).
  - It would add the Windows App Runtime as a dependency.
- Not `tauri-winrt-notification` either: it drops the `ToastNotification` handle, so `Activated` callbacks stop firing (seen in other projects). We need to own the object lifetime. `notify-rust` on Windows has no action buttons. The windows-rs code is roughly 300 lines; own it.
- **Identity:**
  - Keep AUMID `AgentMuxCorp.AgentMux` (the host's existing ID, so taskbar grouping and toasts agree).
  - The launcher also calls `SetCurrentProcessExplicitAppUserModelID` with it, and uses `CreateToastNotifierWithId(aumid)` explicitly.
  - **Installer (Inno):**
    - Write `HKCU\Software\Classes\AppUserModelId\AgentMuxCorp.AgentMux` with `DisplayName=AgentMux` and `IconUri=<install>\resources\toast-icon.png` (a real PNG, not an exe resource).
    - Add `AppUserModelID: "AgentMuxCorp.AgentMux"` to the Start-menu `[Icons]` entry.
    - Remove both on uninstall.
  - **Self-heal at launcher start:** if the key is missing (portable or dev build), write it under HKCU. No elevation is needed.
  - Warp #10187 (May 2026) is the cautionary case: no AUMID registration means toasts are silently dropped, with no error.
- **MSIX:**
  - The package identity provides an AUMID (`<PFN>!AgentMux`). The launcher must detect packaged mode (`GetCurrentPackageFullName` succeeds) and use that ID instead.
  - Declare the `windows.protocol` extension in `AppxManifest.xml.template` for `agentmux:`.
- **Activation:** `activationType="protocol"` with `launch="agentmux://notify/<id>?block=<block_id>"`.
  - Register the `agentmux` URL scheme under `HKCU\Software\Classes\agentmux` → `"<install>\agentmux.exe" "%1"`.
  - A cold launch via the URI hits the existing single-instance path (`second_instance.rs`), which forwards to the running instance. No COM activator is needed.
  - While running, the in-process `Activated` handler fires too. Keep the `ToastNotification` plus handler alive in a map keyed by id until dismissed or retracted, and dedupe against the URI path by notification id.
  - A COM `INotificationActivationCallback` activator is **deferred**. It is only needed for inline text reply, which is a non-goal.
- **XML:** a single template in code, built with `XmlDocument` API calls (never string concatenation with untrusted text).
  - `<text>` title from `TitleTemplate`.
  - `<text>` body (redacted).
  - `<text placement="attribution">` for provenance, e.g. "via ReAgent · verified" (§9.1).
  - `displayTimestamp`.
  - `<header id=agent_id title=agent_name>` groups per agent in Notification Center.
  - Buttons: `Open` (default click already does this, so it is omitted on Windows), `Mute agent 1h`, and `Dismiss` (system).
  - `InputWaiting` may use `scenario="reminder"` (stays on screen until acted on) only when `notify:os:persistentattention` is on, default off. Never `alarm`/`incomingCall`, and never `urgent`: we are not allowed to break through DND.
  - `Low` priority uses `SuppressPopup = true`, going straight to the Action Center.
- **Threading:** WinRT toast calls need an MTA/STA-initialized thread. Run them on the existing tray pump thread (`tray/windows.rs`), which has a message loop. Post requests to it the same way status updates arrive (`WM_APP+n`).

### 6.2 macOS backend

- **Prerequisite (blocking): a stable bundle ID.**
  - Today it is `ai.agentmux.<channel>.<version>`. Permission and the user's per-app notification settings (banner vs alert, sound, lock-screen previews) are keyed by bundle ID, so every update would re-prompt and silently reset the user's choices.
  - Change to `ai.agentmux.<channel>` (stable across versions).
  - Investigate first *why* the version is embedded. It is probably side-by-side installs of multiple versions, or LaunchServices caching. If that reason still holds, the notification owner can instead be a tiny, stably-identified helper app. Decide before Phase 1 macOS.
- **API:** `UNUserNotificationCenter`, via `objc2-user-notifications` (already in the objc2 family pulled in by `tray-icon`/`muda`), or `user-notify`, which has the best macOS coverage of the maintained crates (categories, thread ids, actions; LGPL-3.0, so check license fit). Do not use `mac-notification-sys` (deprecated `NSUserNotification`).
- **Delegate:** install the `UNUserNotificationCenterDelegate` at launcher start, before the pump runs, so a click that cold-launches the app still reaches `didReceive`. Install it alongside `install_reopen_handler()` in both the splash and headless pump paths (`splash_mac.rs`, `prepare_headless_app`). Implement `willPresent` to return `[]` when §6.0 says suppress.
- **Authorization:**
  - Call `requestAuthorization([.alert, .sound, .badge])` **lazily**, the first time a notification would actually be shown, or when the user enables OS notifications in settings. Never at cold launch; that was the regression in `retro-macos-notification-double-prompt-regression-2026-06-22.md`.
  - If the user denies, record it and show "Notifications are off in System Settings" in our settings page with a deep link.
- **Interruption levels:** `InputWaiting`/`AgentCrashed`/`MessageNeedsReview` → `.active` (optionally `.timeSensitive` behind `notify:os:persistentattention`, which needs the `com.apple.developer.usernotifications.time-sensitive` entitlement in `build/entitlements.mac.plist`; no Apple approval is required). `Low` → `.passive`. Never `.critical`.
- **Categories:** `agentmux.attention` (actions: Open, Mute 1h) and `agentmux.info` (Open). Use `hiddenPreviewsBodyPlaceholder = "Agent activity"`.
- **Dock:** badge = unresolved `Attention` count via `NSApp.dockTile.badgeLabel`. `LSUIElement=true` means the launcher has no Dock tile, so the badge must be set by the **host** process that owns the Dock presence. That is Phase 4, handled with the taskbar overlay.
- **Signing:** notifications require a signed bundle (ad-hoc works for dev); shipping requires Developer ID + notarization, which the tray spec already calls a prerequisite.

### 6.3 Linux backend

- **Crate:** `notify-rust` (zbus backend; `notify-rust` 4.18.0, released 2026-09-08, is actively maintained). It supports actions, `replaces_id`, `urgency` and the `desktop-entry` hint.
  - Set `desktop-entry=agentmux` so GNOME/KDE attribute it and expose per-app settings.
  - Call `GetCapabilities` once. If `body-markup` is advertised, escape the body.
- **Flatpak/Snap:** detect `/.flatpak-info` and use `org.freedesktop.portal.Notification` v2 (via `ashpd`, already in the lockfile) with `display-hint: ["hide-content-on-lock-screen"]` for anything with a body. Portal activation requires `DBusActivatable=true` in the `.desktop` file. Defer to Phase 3; AppImage is the shipping format today.
- **Activation:** the `ActionInvoked` signal goes only to a live D-Bus connection, which is fine because the launcher is alive for the session. On Wayland, use the `ActivationToken` signal (spec 1.2) to request focus via xdg-activation when raising the window.

---

## 7. Click and deep-link handling

1. The toast click, tray menu entry or `agentmux://notify/<id>?block=<block_id>` URI reaches the launcher.
2. **Validate** (§9.3): the `id` must exist in the Presenter's live map, or be fetched from srv history. The `block_id` is taken **from our own record**, never from the URI. The URI's `block` param is only a hint and is ignored if it disagrees.
3. The launcher sends `POST /ipc focus_block { block_id }` to the host.
   - The host finds the window containing the block, restores and raises it (`AllowSetForegroundWindow` handoff: the launcher is the activated process, so it holds foreground rights and passes them via `AllowSetForegroundWindow(host_pid)` before the IPC call), switches tab, and focuses the pane.
   - If no window hosts it (background mode), the host runs `open_new_window` and then loads the block's tab. `promote_pool_window`'s liveness gap (tray spec §5.1) applies here. Verify it is closed before Phase 1 ships.
4. The Presenter sends `notify.ack { id, how: clicked }`. The Router marks it resolved and publishes a retract.

---

## 8. Settings (all in `settings.json`; template section `-- Notifications --`)

| Key | Type | Default | Meaning |
|---|---|---|---|
| `app:runinbackground` | bool | `false` | Tray + background-service mode (Phase 0). Restart required in v1 |
| `notify:os:enabled` | bool | `true` once Phase 1 ships (the OS still asks for permission on macOS) | Master switch for OS toasts |
| `notify:os:when` | `"unfocused" \| "always" \| "never"` | `"unfocused"` | §6.0 |
| `notify:os:inputwaiting` / `turncompleted` / `turnerrored` / `agentcrashed` / `messageneedsreview` / `reviewarrived` / `crondeliveryfailed` | bool | `true, true, true, true, true, true, false` | Per-kind (§5.1 step 1) |
| `notify:os:preview` | `"full" \| "redacted" \| "none"` | `"redacted"` | §9.2 |
| `notify:os:persistentattention` | bool | `false` | Windows `reminder` scenario / macOS `.timeSensitive` for Attention kinds |
| `notify:os:sound` | bool | `false` | Use the OS toast sound. The renderer sound service stays separate and is suppressed when an OS toast played a sound, to avoid double dings |
| `notify:delay:<kind>` | int ms | §5.1 | Debounce |
| `notify:pause:until` | int (epoch ms) | `0` | Set from the tray |
| `notify:pause:allowattention` | bool | `false` | |
| `notify:quiethours` | string `"HH:MM-HH:MM"` | `""` | Local time |
| `notify:mute:<agent_id>` | int (epoch ms, `-1` = forever) | none | Per-agent mute |

- The `schema/settings.json` wildcard `"notify:*": boolean` must be narrowed to explicit entries, because several keys are not booleans.
- Mirror the keys in `wconfig/types.rs` and `srv-types.d.ts`.
- The settings page section `sounds-section.tsx` ("Sounds & Notifications") gets a "Desktop notifications" group, with a **"Send test notification"** button. It is the single most useful support tool: it proves AUMID, permission and DND state end to end.

---

## 9. Security and privacy (non-negotiable)

Toasts are a known phishing surface: attackers mimic OS or security prompts (ipurple.team 2026-03; Malwarebytes, Matrix Push C2, 2025-11). AgentMux adds a specific risk: agents and remote jekt senders produce text, and some of that text is unverified by construction (`CLAUDE.md` jekt rules).

### 9.1 Titles are app-controlled; provenance is shown; unverified content never reaches a toast

- `title_template` is a closed enum rendered by our code: "**lark** needs your input", "**lark** finished", "**lark** stopped with an error", "Review arrived on PR #123", "A message for **lark** needs your review". Only the agent **name** is interpolated. Agent names are user-chosen, but still length-cap them (32 chars) and strip control and bidi characters (U+202A–U+202E, U+2066–U+2069).
- **The body never contains jekt or muxbus message text.** `MessageNeedsReview` fires for exactly the `requires_stop` case (`handler.rs:1283`, i.e. `ESCALATE=required`). Its toast is fixed text only: "A message for **lark** needs your review before it acts. Open AgentMux to see the sender and trust level." The human then sees the full `[JEKT:...]` marker in-app, where the confirmation actually happens. Neither the sender, a `TRUST=` label nor a message excerpt appears on the toast. This is what makes the toast unforgeable as a *confirmation channel*: it can only say "go look", never "approve this".
- `ESCALATE=none` and ordinary `coord`/`info` jekts do **not** generate toasts. They are agent-to-agent traffic. Toasting them would train the user to ignore toasts, and the agent handles them anyway.
- `ReviewArrived` fires from the muxbus path only when reagent's `SIG=verified` (production key) passed. The attribution line reads "via ReAgent". Unverified WAN review claims produce no toast.
- **Attribution:** on Windows 11 the app name and icon in the toast header are drawn by the shell from the registered AUMID and cannot be set by content. That is the anti-spoofing anchor. Never put "Windows", "Security" or similar in our own text.

### 9.2 Lock-screen and shoulder-surfing privacy

- `notify:os:preview = "redacted"` (default):
  - `InputWaiting` shows the question's first line, truncated to 80 chars, **with credential-keyword redaction** (the same keyword list `sanitize.rs` uses for tier forcing; if it matches, the body becomes "Contains sensitive content").
  - `Turn*` shows no body beyond the outcome.
- `"none"`: title only. `"full"`: 200-char body, still keyword-redacted.
- Tool-call arguments, file paths, diffs and command lines are **never** put in a toast body at any setting.
- macOS: `hiddenPreviewsBodyPlaceholder`. Linux portal: `hide-content-on-lock-screen`.

### 9.3 Actions and activation input are untrusted

- **No toast action approves, denies or runs anything.** Available actions: open (focus pane), mute agent 1h, dismiss. Tool-permission approval happens only in the pane. This matches Apple's "no destructive actions" guidance and GNOME's "every action must also exist in-app".
- **The `agentmux://` scheme is reachable by any local process and by any web page** (browsers prompt, but users click through). The handler:
  - accepts exactly one route (`notify/<ulid>`);
  - validates the ULID syntax;
  - resolves it against srv history;
  - ignores every other parameter as an authority;
  - performs only focus/open;
  - is rate-limited;
  - never opens external URLs, and never runs anything a URI parameter names.
- Clicking a `ReviewArrived` toast opens the **agent pane**, not the GitHub URL directly. The pane can offer the link. This avoids `openExternal`-style abuse via crafted history rows.

### 9.4 Background mode audit

Every OS notification shown while no window was open is appended to the existing `background_audit` log. "Ran while you were away" then includes "3 notifications were shown", which closes part of the tray spec's still-open WS4 audit box.

---

## 10. Phased rollout

| Phase | Scope | Platforms | Exit criteria |
|---|---|---|---|
| **0: Tray reachable** | `--background` bug fix (§4.1.1), `app:runinbackground` setting + settings UI, autostart toggle surfaced, Win11 overflow hint | Win, macOS | Toggle on, restart: icon visible. Autostart login: icon visible (human-verified; tray spec §7.5 says clicks need a screen) |
| **1: Toasts MVP** | `agentmux-common::notify` types, srv Router (policy steps 1, 2, 4, 5, 7; history table), renderer `notify.emit` for `turn.ended` + `input.waiting/resolved`, launcher srv subscription, Windows Presenter (AUMID self-heal + installer, protocol activation, retract), `focus_block` host IPC, settings subset (`notify:os:enabled/when/<kind>/preview`), "Send test notification" | **Windows** | Human sees a toast with the AgentMux name/icon; click lands in the right pane; answering in-app removes the toast from the Action Center; no toast while looking at that pane |
| **2: Tray hub** | Icon state variants, attention submenu, Pause notifications, rate limit + `BackgroundSummary`, `AgentCrashed`, `MessageNeedsReview` | Win, macOS tray | 20 agents finishing within 10 s give ≤ 4 toasts + 1 summary |
| **3: macOS + Linux toasts** | Stable bundle ID (prerequisite), `UNUserNotificationCenter` Presenter, Linux `notify-rust` Presenter + `ksni` tray, `ReviewArrived`, quiet hours, per-agent mute, in-app bell/history panel reading `notify_history`, sound service consumes Router events instead of raw pane events | macOS, Linux | Same criteria as Phase 1, on each OS |
| **4: Attention surfaces** | Host: `ITaskbarList3::SetOverlayIcon` (with `pszDescription`, re-applied on `TaskbarButtonCreated`), `FlashWindowEx(FLASHW_TRAY \| FLASHW_TIMERNOFG)` for `InputWaiting` only, macOS dock badge + `requestUserAttention(.informationalRequest)`. Supersedes the 2026-05-23 draft's attention half | All | Overlay survives Explorer restart; flash stops when the window is focused |
| **5: Server-side waiting detection** | srv detects "agent awaiting answer/permission" from the provider stream (not just the mounted pane), so `InputWaiting` works in background mode with zero windows | All | Toast for a question asked by an agent whose pane was never opened this session |

Phase 5 is what fully delivers G4. Until it lands, `InputWaiting` requires the agent's pane to be mounted in some window, including a hidden one. That is a real limitation and must be stated in the settings UI copy, not hidden.

---

## 11. Testing

- **Unit (Rust, srv):** the Router pipeline is a pure function `(state, event, now, settings) → (state', actions)`, matching the reducer style used in `launcher/src/reducer` and `agent-pane-state`. Table tests cover: debounce drop on resolve, dedupe update-in-place, rate-limit fold into summary, pause and quiet-hours crossing midnight, mute expiry, redaction keyword hits, and `requires_stop` → `MessageNeedsReview` with no body.
- **Unit (launcher):** the §6.0 gate truth table; URI validation (fuzz the `agentmux://` parser: no route other than `notify/<ulid>` accepted); XML built via DOM with hostile names (`</text><action ...>`, bidi overrides).
- **Integration:** a fake Presenter backend (`NotifyBackend` trait: `show`, `update`, `retract`) records calls. A srv test harness fires `agentfailure` and asserts the recorded toast.
- **Manual, human-with-screen, per OS (required; cannot be automated in CI):** toast appears with the correct app name and icon; click deep-links; answering in-app retracts; cold-start click via URI; DND on means the toast goes quietly to the notification center; Win11 overflow hint; macOS first-run permission prompt appears exactly once, and not at launch.

---

## 11.5. Implementation status (2026-09-24)

| Phase | State | PR |
|---|---|---|
| 0: tray reachable | Done. `--background` now sets the tray env on the launcher itself; `app:runinbackground` setting; separate "Start at login" toggle via host → launcher CLI verbs | #3645 |
| 1A: Router + renderer bridge | Done | #3650 |
| 1B: Windows toasts | Done. OS acceptance verified live (notifier `Enabled`, toast in the AUMID's notification-center history); the on-screen banner was not observed, because banners were suppressed for *every* app on the test machine (a PowerShell control toast also did not show) | #3653 |
| 2: tray hub + srv sources | Done on Windows. `AgentCrashed`, `MessageNeedsReview`, pause, rate limits and summary, `notification:state` | #3654 |
| 3: macOS | **Not started**, see below | — |
| 3: Linux | Done: freedesktop notifications over zbus, `ksni` tray with the same hub menu. Compile- and unit-tested in CI only; not run on a Linux desktop | Linux/quiet-hours PR |
| 3: quiet hours | Done (`notify:quiethours`) | Linux/quiet-hours PR |
| 3: per-agent mute, in-app bell/history, ReviewArrived, sound via Router | Not started | — |
| 4: taskbar attention (Windows) | Done: overlay badge plus flash on increase | #3662 |
| 5: srv-side waiting detection | Done for persistent Claude (`can_use_tool` AskUserQuestion). Other providers have no waiting signal to hook (§1.3 research: they run with approvals bypassed) | #3662 |

**Deviations from the plan above, and why:**

- **The focus gate lives in srv, not the Presenter (§6.0).** The launcher only hears when a window *gains* foreground and has no per-pane focus information. Each frontend window instead reports `notify.focus`, and the Router gates on that, with a heartbeat so a reconnected WS re-registers.
- **No `agentmux://` protocol handler in v1 (§6.1, §7).** Clicks use the in-process WinRT `Activated` callback, and the launcher clears the app's toasts at startup and exit, so a toast never outlives the instance that could act on it. This also avoids a URI that any local process or web page could invoke. A cold-start click is therefore not supported.
- **Click → pane goes through srv and the frontend, not a host `focus_block` verb.** On a click, the launcher sends `notify.ack`, the Router publishes `notification:activate`, and the window that holds the block switches tab, focuses the pane and raises itself. If no window is open, the launcher opens one and it picks the block up via `notify.takeactivation`.
- **One Router per broker, in a registry, not an `AppState` field.** Sources that are process-global (the reactive handler's needs-review hook) are filtered per Router by block ownership.
- **History is in memory,** so there is no `notify_history` table yet. It is only needed for the in-app bell, which has not been built.
- **Linux does not use `notify-rust`.** The backend speaks `org.freedesktop.Notifications` directly over async zbus, because notify-rust's blocking handle model can't retract a notification whose click is being awaited. ksni's blocking API runs its own Tokio runtime, so the tray lives on a dedicated std thread.
- **Taskbar badges are per window,** driven by each frontend rather than by the launcher.
- **"Finished" comes from srv's controller status, not the renderer.** The first live run (2026-09-25, 0.57.2) showed "AgentX finished" toasts while AgentX was still working. srv's `turn_active` stayed true for the whole six minutes, but the renderer's reducer emitted `turn-ended (completed)` several times inside that single turn; the likely cause is `result` frames from queued messages that srv released straight into the next turn. The Router now watches `controllerstatus` through its broker observer and treats a true→false `turn_active` flip as "finished" and false→true as "resolve". The renderer only reports a user Stop, so that the turn-end it causes isn't announced.

**macOS — not started, deliberately.** It needs Objective-C FFI in the launcher (a `UNUserNotificationCenter` delegate, lazy authorization, and a Dock badge in the host). FFI mistakes there are runtime crashes of the whole app, not compile errors, and no Mac was available to run it. The §12 Q1 decision on the per-version bundle ID also affects it directly. `build-macos.yml` accepts `workflow_dispatch` with a branch ref, so a follow-up can at least be compile-checked on CI before a Mac run. Until then the macOS launcher uses the null presenter, and the menu bar keeps its original New Window / Quit menu.

## 12. Open questions (for the repo owner)

1. **macOS bundle ID.** Why is the version in the bundle ID, and can it be stable per channel? This blocks macOS toasts (§6.2).
2. **Default on or off.** Should OS toasts be on by default once Phase 1 ships (the recommendation, with only the `Attention` and `Turn*` kinds enabled), or opt-in like the tray? The tray's opt-in rationale was about a persistent background process, which doesn't apply to toasts from a foreground app.
3. **Should the tray remain coupled to background mode?** This spec keeps the coupling (one toggle). The alternative is a tray that exists only while windows are open, as a notification hub. That is cheaper but reintroduces the "icon vanishes on close" oddity the original gate was written to avoid.
4. **`notify-rust` vs `user-notify`.** The LGPL-3.0 license of `user-notify` needs a license check before it is chosen for macOS.

---

## References

**Code (at `9cc79a624`):**
- tray: `agentmux-launcher/src/tray/{mod,windows,macos}.rs`
- `--background` bug: `agentmux-launcher/src/autostart/mod.rs:89-98`, `host_spawn.rs:58,169`
- srv subscription material: `srv_spawner.rs:42-47`
- event bus: `agentmux-srv/src/backend/mps.rs`
- jekt stop: `reactive/handler.rs:1219-1284`, `sanitize.rs:327-379`
- AUMID: `agentmux-cef/src/lib.rs:475-485`
- `disable-notifications`: `agentmux-cef/src/app/mod.rs:911-918`
- bundle ID: `scripts/package-macos.sh:163-172`
- foreground report: `agentmux-common/src/ipc.rs:242`
- settings: `settings-template.jsonc:68-83`, `schema/settings.json:244-310`, `wconfig/types.rs:259-310`

**Microsoft:**
- Toast content: https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-content
- Toast UX: https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/app-notifications-ux-guidance
- Unpackaged AUMID / other apps: https://learn.microsoft.com/en-us/windows/apps/develop/notifications/app-notifications/send-local-toast-other-apps
- Progress/update: https://learn.microsoft.com/en-us/windows/apps/design/shell/tiles-and-notifications/toast-progress-bar
- History remove: https://learn.microsoft.com/en-us/uwp/api/windows.ui.notifications.toastnotificationhistory.remove
- `SHQueryUserNotificationState`: https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shqueryusernotificationstate
- Taskbar overlay: https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itaskbarlist3-setoverlayicon
- Notification area UX: https://learn.microsoft.com/en-us/windows/win32/uxguide/winenv-notification
- App SDK unpackaged `Register()` bug: https://github.com/microsoft/WindowsAppSDK/issues/6774
- archived `windows-app` crate: https://github.com/microsoft/windows-app-rs

**Apple:**
- HIG notifications: https://developer.apple.com/design/human-interface-guidelines/notifications
- Interruption levels: https://developer.apple.com/documentation/usernotifications/unnotificationinterruptionlevel
- HIG menu bar: https://developer.apple.com/design/human-interface-guidelines/the-menu-bar

**freedesktop / GNOME:**
- Notifications spec: https://specifications.freedesktop.org/notification/latest-single/
- Portal: https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.Notification.html
- GNOME HIG: https://developer.gnome.org/hig/patterns/feedback/notifications.html
- AppIndicator extension: https://extensions.gnome.org/extension/615/appindicator-support/

**Crates:**
- `notify-rust`: https://docs.rs/notify-rust/
- `tauri-winrt-notification`: https://docs.rs/tauri-winrt-notification
- `user-notify`: https://github.com/Simon-Laux/user-notify
- `tray-icon`: https://github.com/tauri-apps/tray-icon

**Comparable products:**
- VS Code chat notifications: https://code.visualstudio.com/docs/chat/chat-overview
- Warp agent notifications: https://docs.warp.dev/agent-platform/local-agents/agent-notifications
- Warp AUMID bug: https://github.com/warpdotdev/warp/issues/10187
- Claude Code notification delays: https://code.claude.com/docs/en/terminal-config
- Slack notification scheduling: https://slack.com/help/articles/201355156

**Security:**
- https://ipurple.team/2026/03/25/toast-notifications/
- https://www.malwarebytes.com/blog/news/2025/11/matrix-push-c2-abuses-browser-notifications-to-deliver-phishing-and-malware
