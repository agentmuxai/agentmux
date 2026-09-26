# SPEC: Start with OS — AgentMux starts quietly at login, on every platform, and keeps working after updates

**Date:** 2026-09-25
**Status:** active — Phases 0, 1 (#3788), 1b and 2 are built; Phases 3 and 4 are not started. See §6.
**Author:** Maricon
**Trigger:** Repo owner: *"we also want to design a robust 'start with OS' feature."*
**Builds on:** `SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` §4 (mechanism choice per platform, WS2)
and §6 (consent rules); `SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md` §4.1 (the Settings toggle).
**Related:** PR #3785 (tray on by default; a tray that cannot start drops background mode).
**Owner decisions (2026-09-25):** start at login is **off by default**, with no first-run prompt. It is
controlled from a Settings toggle **and** a check item in the tray's right-click menu, and both control
the **same reactive property** (§3.9).

---

## 1. What "robust" means here

A user who turns on **Start at login** should get, on every login, on every platform and package
format:

1. **It starts.** The login entry launches a binary that exists and can load its libraries, including
   after an update, a reinstall to the same place, or a new AppImage download.
2. **It starts quietly.** A tray icon appears and no window opens. The first window opens when the user
   clicks the icon, or when they launch AgentMux themselves.
3. **It is never invisible.** If there is no tray to show (e.g. Linux without a StatusNotifier host), it
   opens a normal window instead of running hidden. This is the WS4 rule: a resident process always has
   a visible indicator.
4. **It starts once.** A login entry firing while AgentMux is already running does nothing: no second
   instance and no extra window.
5. **It can be seen, repaired and removed.** Settings shows whether the entry is present **and valid**
   (target exists), where it points, and which build owns it, with Repair and Disable. Uninstalling the
   app leaves no entry that errors at every login.
6. **No elevation, no silent enabling.** Per-user mechanisms only. Off by default (§4).
7. **One switch, shown twice.** The Settings toggle and the tray menu's check item are the same
   property, so they can never disagree (§3.9).

## 2. What existed before this spec

`agentmux-launcher/src/autostart/mod.rs` registers an entry with `current_exe()` and `--background`.
It can be driven through `--enable-autostart`, `--disable-autostart` and `--autostart-status`, and the
Settings toggle drives it too (`agentmux-cef/src/commands/autostart.rs` shells out to the launcher).

| Platform | Artifact | Location |
|---|---|---|
| Windows | Scheduled Task `AgentMux`, `InteractiveToken`, `LeastPrivilege`, 30 s delay, battery-safe | `autostart/mod.rs:125-176` |
| macOS | LaunchAgent `com.agentmux.launcher`, `RunAtLoad`, `KeepAlive=false` | `autostart/mod.rs:185-211` |
| Linux | XDG `~/.config/autostart/agentmux.desktop` | `autostart/mod.rs:218-230` |

Artifact generation is pure and unit-tested; `disable()` is idempotent. Those parts are sound and are
kept.

### 2.1 Gaps, by severity

| # | Gap | Effect | Where |
|---|---|---|---|
| G1 | **Linux target is the raw launcher.** For AppImage, `current_exe()` is the versioned extract cache `~/.local/share/agentmux/extracted/<ver>/usr/bin/agentmux-launcher`, and AppRun prunes all but the two newest caches. For deb/rpm/tarball it bypasses the wrapper that sets `LD_LIBRARY_PATH`, and the launcher has no RPATH. | AppImage: the entry dies after two updates. deb/rpm/tarball: fails to load `libcef.so`, likely at every login. | `autostart/mod.rs:471`; `scripts/linux-apprun.sh:114-145`; `scripts/build-deb-linux.sh:28-31` |
| G2 | **`--background` still opens a window.** The host creates the first window unconditionally. | "Starts quietly" is false; the UI copy "tray only, no window" is wrong. | `agentmux-cef/src/app/mod.rs:1026` (`window_create_top_level` about 1132) |
| G3 | **No login readiness.** On Linux the StatusNotifier watcher may register after the autostart entry runs, so `ksni` fails and the tray is reported unavailable. After #3785 that means background mode is dropped. | A login start comes up with no tray, and closing its window quits. | `tray/linux.rs:165-205` (5 s bounded start, no retry) |
| G4 | **Nothing re-points the entry.** It is written once, at enable time. | Any path change (portable zip moved, new AppImage, macOS app moved) silently breaks it. | `autostart/mod.rs:349-351` |
| G5 | **One identity for every channel.** `AUTOSTART_ID = "AgentMux"` and the macOS label are channel-independent, so enabling from a dev build overwrites the release entry, and disabling from any build removes it. | The owner, who runs `local-*` builds daily, can point login at a build that is later pruned. | `autostart/mod.rs:104-107` |
| G6 | **Uninstall leaves entries behind.** Inno has no `[UninstallRun] --disable-autostart`; MSIX has no `StartupTask`; deb/rpm cannot clean per-user files; an AppImage has no uninstall. | An error or a no-op at every login, forever. Zoom-2019-shaped (tray spec §6). | `packaging/windows/agentmux.iss`; `packaging/msix/AppxManifest.xml.template:42` |
| G7 | **macOS uses a raw LaunchAgent, not `SMAppService`.** Bundle ids are per version (`ai.agentmux.<channel>.<ver>`). | The entry may not show in System Settings → Login Items, where users look; a per-version id cannot be a stable login item. | `autostart/mod.rs:22-36`; `scripts/package-macos.sh:172` |
| G8 | **A second launch with `--background` opens a window** (it forwards `open_new_window`). | A login entry firing while AgentMux is already running pops a window. | `supervisor/windows.rs:157-190`; `second_instance.rs` about 325 |
| G9 | **Status is "file exists".** `is_enabled()` does not check the target. | Settings says "on" for an entry that cannot start anything. | `autostart/mod.rs` `is_enabled` |

G1 is a present-day correctness bug on the platform the owner uses. G2 and G8 make the feature noisy
everywhere.

## 3. Design

### 3.1 A stable launch target per package format

The entry must point at something that survives updates, and the launcher must not guess it from its
own path. The **wrapper or AppRun that started it says so**: each sets `AGENTMUX_STABLE_EXE` to its
own path right before `exec`. The launcher reads it once at startup and **removes it from its env**
(`autostart::take_stable_exe_env`), so no child (host, srv, an agent shell, a dev build started from a
pane) inherits a path that belongs to this launch. `$APPIMAGE` is deliberately not read directly: the
AppImage runtime's copy leaks into every agent shell, so a dev build run from a pane would register the
installed AppImage. `autostart::stable_target()` falls back to `current_exe()` when the variable is
absent or names a missing file.

| Kind | Stable target the entry runs | Who provides it |
|---|---|---|
| AppImage | The AppImage file itself (`$APPIMAGE`, e.g. `~/Desktop/AgentMux_<ver>.AppImage`) | AppRun exports it (`scripts/linux-apprun.sh`). A newer AppImage re-points the entry on its first start (§3.9), so deleting the old file after that is safe. |
| deb / rpm | `/usr/bin/agentmux` (the wrapper that sets `LD_LIBRARY_PATH`) | The wrapper exports it |
| tarball | `<extract dir>/agentmux.sh` | The script exports it |
| macOS | `current_exe()` inside the `.app` for now; `SMAppService` later (§3.7) | — |
| Windows (Inno) | `current_exe()`, i.e. `{autopf}\AgentMux\agentmux.exe` (stable per-user install path) | — |
| Windows (MSIX) | The package's `StartupTask` (§3.7, not built) | The manifest |
| Windows portable / dev | `current_exe()`; a move is fixed on the next start (§3.9) | — |

**Linux `TryExec=`.** The XDG entry gains `TryExec=<stable target>`. Per the Desktop Entry spec the
session ignores an entry whose `TryExec` is missing or not executable, so removing the app (or deleting
the AppImage it names) makes the entry **inert instead of erroring at every login**. This
closes G6 on Linux without root touching per-user files.

### 3.2 Self-heal on every launch

Self-heal is the reconcile of §3.9 running on every launcher start. When srv sends the settings on
connect, the launcher compares the entry with `stable_target()` and this channel. It rewrites an entry
that points at an old version or a moved file. It rewrites only while the setting is **on**, so it can
never enable anything by itself. A target that no longer exists is handled by `TryExec` on Linux; a
Broken/Repair status in Settings is Phase 3.

### 3.3 Start quietly: "main" loads hidden when the tray is up (built)

The launcher sets host env `AGENTMUX_START_HIDDEN` only when **all** of these hold
(`host_spawn::start_hidden_decision`):

- it was started with `--background` (a login start, never a manual one);
- the tray started (`!tray::unavailable()`, from #3785), so there is always an icon to reach it from;
- this is the **first** host it spawns. A host restarted after a crash comes back visible, because the
  user may already have been using it.

**The "main" window is still created and loaded**; only its first reveal is held
(`agentmux-cef/src/start_hidden.rs`, hooked into `reveal_top_level_window`, the path every first reveal
and its retries take). The first design here, skipping `window_create_top_level` entirely, was
rejected after mapping what depends on "main":

- the frontend's session restore and the host's multi-window reproject run only from "main";
- agents start only when a pane mounts in a window (`ControllerResync`), so with no window no agent
  would run at login, which defeats the point;
- the window pool starts when "main" registers;
- `open_new_window` with zero live browsers cannot create a window (it clones its CEF client from an
  existing one), and the first window would have been force-labelled "main" with a mismatched queue
  entry.

A hidden "main" avoids all of these: the app is exactly as it would be after a normal start, just not
on screen.

**The first request for a window shows "main"** instead of opening a second, blank one:
`open_new_window` (tray *New Window*, a relaunch, the macOS Dock with no visible window) and focusing
"main" (a notification click, the Dock). If "main" has not finished loading yet, the hold is released
and its normal load path shows it. Later requests open windows as usual.

**Audit (WS4):** the held period is recorded in the background audit log like a period with every
window closed (`WentUnattended` on the first held reveal, `Observed` when it is shown), under the same
lock as the reducer's own transitions.

**Splash:** a login start skips the splash (`splash_config::splash_disabled`); on macOS that also
routes the start through the headless AppKit pump the menu-bar item needs. **No tray means a window**,
never a hidden process (§1 item 3).

### 3.4 Login readiness

- **Linux tray (built):** a login start retries the `ksni` tray start once a second for up to **45 s**
  (`tray::LOGIN_TRAY_WAIT`) while no StatusNotifier host is registered; any other start fails fast
  (5 s). The host spawn waits for that outcome, since it needs `tray::unavailable()` to choose between
  hidden and windowed (§3.3). Nobody is looking at a login start, and a late-but-correct tray beats a
  window nobody asked for.
- **XDG entry (built):** `X-GNOME-Autostart-Delay=10`, the Linux equivalent of the Windows 30 s delay.
- **Windows:** keep the Task's 30 s delay. Re-adding the icon when Explorer restarts is already done by
  `tray-icon` (0.24), which re-registers on the `TaskbarCreated` broadcast; nothing to add.
- **macOS:** the menu bar is up before login items run; no wait needed.

### 3.5 One owner, recorded in the entry

Channels are separate installs that share one login, so one entry, with the owner written into it:

- The artifact carries the owning channel and the target (Linux: `X-AgentMux-Channel=`; macOS: a
  `AgentMuxChannel` key in the plist or the `SMAppService` registration; Windows: the Task's
  `<Description>`).
- Enabling from a build **takes ownership** (last enable wins). Settings in every other build shows
  "Start at login is on for *<channel>*", with a button to switch it to this build.
- Turning the setting **off** removes only an entry this channel owns; another build's entry is left
  alone (built: `autostart::plan`).
- Not built yet: showing the owning channel in Settings, and asking before a non-owning build turns it
  off.

This keeps the existing single artifact name, so `disable()` still finds it, while fixing G5.

### 3.6 A login start never opens a second instance or a window (built)

When a launcher started with `--background` loses the single-instance race, it **exits 0 without
forwarding** `open_new_window`. A manual second launch keeps forwarding, as today. Same change in
`supervisor/windows.rs` and `second_instance.rs` (`forward_for_second_instance`), driven by
`autostart::second_instance_opens_window(args)`. The macOS reopen handler, which is a user asking for a
window, is unchanged.

### 3.7 Platform-native registration where one exists

- **macOS: `SMAppService.mainApp`.** It is the only path that shows in Login Items and respects the
  user's approval there. It needs a **stable bundle id for the release channel** (drop the version from
  `CFBundleIdentifier`; keep it in `CFBundleVersion`), objc2 bindings, and a signed bundle. When the
  status is `requiresApproval`, Settings says "Allow AgentMux in System Settings → Login Items" and
  offers `SMAppService.openSystemSettingsLoginItems()`. The LaunchAgent stays as the fallback for
  unsigned and dev builds, with a stable path per §3.1.
- **Windows MSIX: `windows.startupTask`** in the manifest (`Enabled="false"`, toggled through
  `StartupTask.RequestEnableAsync`). Packaged apps cannot rely on a Scheduled Task pointing into their
  versioned install path. Removing the package removes the task.
- **Windows Inno:** `[UninstallRun] Filename: "{app}\agentmux.exe"; Parameters: "--disable-autostart"`.

### 3.8 Status the user can trust

`--autostart-status` gains `--json` and returns
`{ state: "off" | "on" | "broken" | "needs_approval", target, owner_channel, this_channel, movable }`,
where `broken` means the entry exists but its target does not. Settings will render that status next
to the switch, with **Repair** (re-apply the setting). Until then, Settings reads the plain
`--autostart-status` after each change, and says so when the setting is on but nothing is registered.

### 3.9 One property, two switches, one login entry (built)

`app:startatlogin` (bool, **default `false`**) is the single source of truth. srv stores and
broadcasts it like every other setting.

```
 Settings toggle ── setconfig ──┐                        ┌─> Settings toggle (reactive)
                                ├─> srv (settings.json) ─┼─> launcher: tray check item
 tray check item ── setconfig ──┘   broadcast "config"   └─> launcher: reconcile the OS login entry
   (via the launcher's srv session)
```

- **Settings:** the toggle reads and writes `app:startatlogin`, the same way the tray toggle above it
  works (`notifications-section.tsx`). It no longer asks the launcher to enable anything, and the
  host's `set_autostart` command is removed. `autostart_status` stays, read-only.
- **Tray:** a **Start at login** check item between *New Window* and *Quit* on Windows, macOS and Linux
  (`tray::menu_model`). Clicking it sends `setconfig {"app:startatlogin": !current}` over the launcher's
  existing srv session (`notify::request_start_at_login`), the same path the tray's "pause
  notifications" uses. The item shows the value srv last broadcast and is disabled until the first one
  arrives, so it never guesses. It does not flip itself: the check mark changes when srv's broadcast
  comes back.
- **The launcher applies it** (`start_at_login::observe`). srv sends the full config on connect and
  after every change. The launcher acts only when this key changes: it redraws the tray and reconciles
  the login entry on a worker thread, using `autostart::plan` (pure, unit-tested):

| `app:startatlogin` | Entry on the machine | Action |
|---|---|---|
| on | this channel's, running `stable_target()` | nothing |
| on | absent, another channel's, or an old or moved target | write it for this channel (takes ownership, re-points) |
| off | this channel's | remove it |
| off | another channel's, or one from before ownership was recorded | nothing |
| never set | one from before ownership was recorded | **adopt**: set `app:startatlogin = true`, since the user turned it on with the old toggle |
| never set | anything else | nothing (off by default) |

- **Where it runs:** the launcher's srv session now starts on macOS too, with the no-op notification
  presenter, because it carries this setting. Nothing reconciles while AgentMux is not running; the
  next start applies whatever the setting says.
- **CLI verbs** (`--enable-autostart`, `--disable-autostart`, `--autostart-status`) stay as low-level
  tools for uninstallers and debugging. `--enable-autostart` now writes the stable target and this
  channel. The setting wins at the next start: an entry this channel owns but that the setting says is
  off gets removed then.

## 4. Consent and defaults

- Start at login is **off by default**, with **no first-run prompt** (owner decision, 2026-09-25). It is
  separate from the tray, which #3785 turns on by default.
- Nothing enables it implicitly: not an installer, not an update, not self-heal. The one exception is
  adopting an entry the user already created with the old toggle (§3.9), which keeps their choice
  rather than making a new one.
- Either switch turns it off, and `--disable-autostart` removes the entry. On Linux the entry becomes
  inert when the app is gone (`TryExec`).

## 5. Testing

- **Pure, in CI (built):**
  - every artifact reads back its own target and channel;
  - `TryExec`, the delay key and the channel key are present and cannot be injected;
  - `stable_target` prefers the wrapper path and skips a missing one;
  - `plan` covers the whole §3.9 table;
  - the tray's check item mirrors the setting and is disabled while the value is unknown;
  - the launcher parses `app:startatlogin` out of srv's config broadcast;
  - the Settings toggle is off by default, writes the key, and reports a failed registration.
- **Pure, in CI (later phases):** the second-instance decision (§3.6) for background vs manual launch;
  the start-hidden decision (§3.3) over `{--background, tray up}`.
- **Launcher integration:** a fake StatusNotifier watcher that appears after a delay (a
  `dbus-daemon --session` in the test), asserting the tray starts and the host gets
  `AGENTMUX_START_HIDDEN`; and that one that never appears yields a window and no background mode.
- **Manual login matrix** before each phase ships (log out and back in, then update and log in again):
  Ubuntu GNOME with and without the AppIndicator extension; KDE; AppImage and deb; Windows 11 with Inno
  and MSIX; macOS signed. The pass criteria are §1's six points.

## 6. Phases

| Phase | Content | State | Fixes |
|---|---|---|---|
| 0 | One property, two switches (§3.9): `app:startatlogin` off by default, Settings toggle, tray check item on all three platforms, launcher reconcile with channel ownership | **Built** (PR #3788) | G5, G9 (partly) |
| 1 | Linux stable target via `AGENTMUX_STABLE_EXE` from AppRun and the wrappers; `TryExec` and the delay key; self-heal (§3.2) | **Built** (PR #3788) | G1, G4, G6 (Linux) |
| 1b | `--background` second instance exits quietly (§3.6) | **Built** (Phase 2 PR) | G8 |
| 2 | Start hidden when the tray is up (§3.3); the Linux watcher wait (§3.4); Explorer-restart icon re-add (already in `tray-icon`) | **Built** (Phase 2 PR) | G2, G3 |
| 3 | Status JSON; Settings status with owner and Repair (§3.8) | Not started | G9 |
| 4 | macOS `SMAppService` with a stable bundle id; MSIX `StartupTask`; Inno uninstall hook (§3.7) | Not started | G6 (Windows), G7 |

Phase 4 needs signing and packaging work that cannot be verified on the Linux dev host. With Phase 2
in, the Settings copy says again that a login start opens in the tray, without a window.

## 7. Open questions for the owner

| # | Question | Recommendation |
|---|---|---|
| 1 | Keep start-at-login off by default, or offer it once on first run? | **Decided 2026-09-25: off by default, no prompt.** A Settings toggle and a tray check item on one property (§3.9). |
| 2 | Should `local-*` dev builds be allowed to own start-at-login? | Yes, but with the owner shown (§3.5), since the owner runs dev builds as their main app. |
| 3 | Linux: add a `systemd --user` option for restart-on-crash? | Not now. The launcher already supervises its children, and lingering past logout is a separate decision (tray spec §4). |
| 4 | macOS: is dropping the version from the release bundle id acceptable? It is required for `SMAppService` and also changes how macOS treats updates (one app, not many). | Yes, for the release channel only. |

## Appendix — file references

| Concern | File |
|---|---|
| Artifacts, enable/disable/status, CLI verbs | `agentmux-launcher/src/autostart/mod.rs` |
| `--background` → host env | `agentmux-launcher/src/host_spawn.rs` (`background_env_for`) |
| Tray start, failure fallback | `agentmux-launcher/src/tray/mod.rs` (`start_if_enabled`, `unavailable`) |
| Linux tray start bound | `agentmux-launcher/src/tray/linux.rs` |
| First window creation | `agentmux-cef/src/app/mod.rs` (`on_context_initialized`) |
| Single instance | `agentmux-launcher/src/supervisor/windows.rs`, `agentmux-launcher/src/second_instance.rs` |
| AppImage re-exec and cache pruning | `scripts/linux-apprun.sh` |
| App-menu entry writer | `scripts/install-linux-desktop.sh` |
| deb/rpm/tarball wrappers | `scripts/build-deb-linux.sh`, `scripts/build-rpm-linux.sh`, `scripts/build-tarball-linux.sh` |
| macOS bundle id | `scripts/package-macos.sh` |
| Windows installers | `packaging/windows/agentmux.iss`, `packaging/msix/AppxManifest.xml.template` |
| Settings toggle | `frontend/app/view/settings/sections/notifications-section.tsx`, `agentmux-cef/src/commands/autostart.rs` (status only) |
| The property, tray redraw, reconcile | `agentmux-launcher/src/start_at_login.rs`, `autostart::plan` |
| Tray check item | `agentmux-launcher/src/tray/mod.rs` (`menu_model`), `tray/{linux,windows,macos}.rs` |
| srv session (config in, setconfig out) | `agentmux-launcher/src/notify/mod.rs` |
| Setting declaration | `schema/settings.json`, `settings-template.jsonc`, `frontend/types/srv-types.d.ts` |
