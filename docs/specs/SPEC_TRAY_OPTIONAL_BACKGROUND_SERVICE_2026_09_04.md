# Spec: optional system-tray + persistent background service, cross-platform

**Status:** proposed — design only, nothing implemented.
**Author:** Agent5
**Tracking issue:** [agentmuxai/agentmux#2977](https://github.com/agentmuxai/agentmux/issues/2977)
**Verified against:** `main` @ (2026-09-04 pull), codebase research + external best-practices research, no live prototype built.

## User's request (verbatim, for traceability)

> we want to mimic some of the features of an OS-like bot (like grok bot) with a systray icon. this will move agentmux to also a system service as optional. even if the user isnt using the frontend, the backend and systray stays opens, to do tasks, and there is a simplier frontend panel at the systray (across 3 platforms) .. research best practices online, we want the best seamless performance, robust solution

## 1. What exists today (confirmed by direct codebase reading)

**Three-process model, siblings not a chain**: `agentmux-launcher` spawns `agentmux-srv` (backend) and `agentmux-cef` (CEF window host) as siblings under one job-object/process-group, not a parent→child chain. `host` does not spawn `srv`; neither spawns the other.

**srv already survives a `host` *crash*, but not a clean exit.** The supervisor's host-crash-restart arm only ever touches `host`, never `srv` (`agentmux-launcher/src/supervisor/windows.rs:863-937`, `unix.rs:654-686`). But a **clean** host exit (`exit code == 0`) — which is exactly what happens today when the user closes the last visible window — breaks the entire supervisor loop and tears everything down: Windows via `KILL_ON_JOB_CLOSE` on the shared job object (`windows.rs:1108-1121`), Unix via an explicit epilogue that terminates both children (`unix.rs:741-758`). **This is the one load-bearing behavior change this whole feature requires**: closing the last window must stop meaning "shut down the tree."

**No headless mode exists, but a directly-reusable hidden-window primitive does.** `agentmux-cef/src/commands/window_pool.rs`'s pool-window mechanism already creates a real, rendering, off-screen CEF top-level window, hidden from the taskbar/Alt+Tab (`WS_EX_TOOLWINDOW` + `SW_HIDE`), and later promotes it back to a normal visible window (`promote_pool_window`). This is the exact mechanical shape "start hidden to tray, show later" needs — today it exists as a warm-pool performance optimization, not wired to any user-facing lifecycle.

**Single-instance + reattach already works, and a tray "Open" action can reuse it as-is.** The launcher's named-pipe (Windows) / Unix-socket (Linux/macOS) bind IS the single-instance gate. A second launch that loses the bind forwards `{"cmd":"open_new_window"}` to the already-running instance (`agentmux-launcher/src/second_instance.rs::forward_open_new_window`), which then runs the same promote-pool-window path above instead of spawning a duplicate backend. This is precisely the "new window attaches to already-running backend" pattern the tray's "Open AgentMux" action needs — no new plumbing required for the happy path.

**Known, still-open reliability gap in that exact reattach path**: `docs/retro/retro-fresh-vm-suspend-orphaned-frontend-2026-09-03.md` documents `promote_pool_window`'s liveness check (Windows `IsWindow()`) only confirming the OS hasn't destroyed the handle — not that the renderer/connection behind it is actually alive. The retro's own root cause turned out to be an unrelated Session-0 zombie-process issue (not proven to be this gap in practice), but the code-level finding stands on its own and is unfixed. **A tray feature would make "backend alive, frontend maybe-stale" a designed-for, everyday state instead of a rare accident — this gap should be closed before shipping, not treated as a parallel nice-to-have.**

**No install/auto-start surface exists at all.** `install_handlers.rs`/`system_install_handlers.rs` are about installing *agent CLI toolchains* (npm packages, git/Node/Python), not AgentMux itself. Grepped the whole repo for Registry `Run` keys, `LaunchAgent`, `systemd` service units: zero hits related to app self-installation. This is genuinely new surface on all three platforms.

**No tray/GUI-abstraction crate exists.** The only real GUI engine linked anywhere is `cef = "148"`. Windows GUI code is raw `windows-sys` (which already enables the `Win32_UI_Shell` feature namespace `Shell_NotifyIconW` lives in — likely no new Windows-side crate feature flag needed). macOS is raw `objc_msgSend` FFI. Linux has `x11rb` + `ash`, no GTK binding beyond what the `rfd` file-picker crate pulls in incidentally. Zero hits for `tray_icon`/`ksni`/`NSStatusItem`/`Shell_NotifyIcon` anywhere in the repo.

## 2. Cross-platform tray implementation — recommended crates

- **Windows + macOS: `tray-icon` (tauri-apps) + `muda`** for the menu. Actively maintained (`tray-icon` v0.24.2, 2026-07; `muda` v0.19.3, 2026-06; tens of millions of downloads each). Windows needs *any* win32 message pump running on the tray-creation thread — not winit/tao-specific — which a CEF host already has. macOS needs a Cocoa run loop **already pumping** (not merely instantiated) on the main thread before tray creation; CEF's own main-thread loop is plausibly compatible but this is the single highest-value thing to verify with a throwaway prototype before committing design details further, since **no public report was found of anyone pairing `tray-icon` with a CEF (as opposed to winit/tao) host app** — architecturally plausible, not proven.
- **Linux: `ksni` directly, not `tray-icon`'s GTK path.** `tray-icon` on Linux requires a running **GTK3 main loop** — a second toolkit's event loop alongside CEF's own GLib-based one, which multiple sources describe as "janky" in practice, and which Tauri itself has an open (stalled, unmerged) PR to replace with `ksni` for exactly this reason. `ksni` (v0.3.6, actively maintained) talks pure D-Bus StatusNotifierItem with no GTK requirement and is runtime-agnostic (works fine alongside Tokio, which `agentmux-srv` already uses). **Recommendation: `tray-icon`+`muda` for Windows/macOS, `ksni` for Linux** — a genuine per-platform split, not one crate for all three.
- **Unavoidable Linux caveat, regardless of crate**: GNOME dropped native tray support in 3.26 and never implemented StatusNotifierItem; users need a third-party extension (`AppIndicator Support` or the newer `Status Tray`, which explicitly handles Chromium/Electron-style apps). This must be a documented user-facing caveat, not something fixable in-app.
- Where should the tray icon live — `host` (CEF process) or `launcher`? **Recommend `launcher`.** The launcher already owns a lightweight native window independent of CEF (the splash screen, raw GDI on Windows) and is the one process guaranteed to survive `host` exiting/restarting. A tray icon owned by `host` would need to be recreated every time `host` restarts (crash-recovery, or a future auto-update swap); one owned by `launcher` persists across that transparently and is a smaller, more stable process to keep resident.

## 3. Comparable-product architecture — what to imitate, what to avoid

**Raycast is the closest documented analog** (official technical write-up): a four-layer split — native host (window/hotkey/tray owner) + webview frontend + a **persistent Node.js backend** (the actual engine) + a Rust core — with the tray-triggered items themselves being **ephemeral** (loaded on demand, not resident), distinct in lifecycle from both the always-on backend and on-demand windows. That three-tier shape (ephemeral tray affordance / on-demand full window / persistent engine) maps directly onto AgentMux's own `launcher` (tray + lifecycle owner) / `host` (on-demand full window) / `srv` (persistent engine) split — this design should lean into that existing shape rather than inventing a new one.

**GitHub Copilot's session model is a useful naming precedent**: it distinguishes local sessions (tied to app lifetime) from a "Background Agent" mode that "runs locally but outside the client process; survives client restarts" — directly analogous to what `srv` decoupled from `host`'s lifetime would become.

**Avoid Docker Desktop's shape as a cautionary example**: heavy idle VM footprint, marketed as acceptable because Docker's value proposition tolerates it — AgentMux's shouldn't resemble this. Avoid Slack/ChatGPT desktop's shape too: both are conventional single-process Electron apps with the window merely hidden, not a real daemon/UI split — ChatGPT desktop currently has a live, unintentional bug on Windows where background processes have no window/taskbar entry at all, which is closer to an accident than a design worth copying.

**Tray-triggered panel**: given the app is already CEF-based (a full web-rendering engine, not a lightweight native toolkit), the natural choice is a **small, separate, always-on-top CEF window** showing a simplified view — not a native OS menu (too limited for "chat with an agent") and not a full secondary process (unnecessary weight given CEF is already resident). This can reuse the exact same pool-window hide/show/promote mechanism already in `window_pool.rs`, sized and positioned differently (near the tray icon rather than centered), rather than building a fourth rendering surface.

## 4. OS-native background/auto-start — recommended mechanism per platform

- **Windows: Scheduled Task with `LogonType=InteractiveToken`, not a Windows Service.** A true Windows Service requires admin elevation to register at all and hits Session-0 isolation (services cannot interact with the interactive desktop — directly disqualifying for anything that needs to show a tray icon or spawn visible windows later). A Scheduled Task set to run at logon under the user's own token needs no elevation and no stored password, with real restart-on-failure options — the practical middle ground between a bare Registry `Run` key (zero crash supervision) and a full Service (needs admin, wrong session).
- **macOS: `SMAppService` (macOS 13+), registering a `LaunchAgent`.** This is unambiguously current best practice — Apple's own framework, correct "regardless of how the app was installed," and the only path that surfaces correctly in System Settings → Login Items & Extensions. Do not use legacy `launchctl load`/`unload` for new code. Registering a background item triggers a **system-level user consent notification** the first time — a real, structural checkpoint, not just a written guideline, and it should be treated as an actual feature (a moment to explain what's being enabled), not an obstacle to route around.
- **Linux: default to an XDG autostart `.desktop` file** (`~/.config/autostart/`) for broad desktop-environment compatibility; **offer a `systemd --user` unit as an optional, more-robust alternative** for users on systemd-based distros who want survive-logout behavior via `loginctl enable-linger` — as an explicit, separately-reviewed opt-in, never silently enabled by an installer. Detect environment at install time rather than hard-picking one mechanism at build time; a stable minority of non-systemd distros (Alpine, Gentoo, Void, Devuan, Artix, Slackware) makes autostart `.desktop` the safer universal default.
- **Code-signing is now a functional requirement, not just risk mitigation**, on both platforms with an auto-starting background component: Windows' Smart App Control (on by default on clean Windows 11 installs) can block unsigned/unknown apps outright with no override, and EV certificates no longer bypass SmartScreen reputation checks at all as of an August 2024 Microsoft change — OV and EV now start from zero reputation identically to unsigned. macOS Sequoia removed the Control-click Gatekeeper override entirely for unnotarized apps. **This feature should not ship without the release pipeline already producing signed, notarized (macOS) builds** — worth confirming as a prerequisite, not assuming.

## 5. Reconnection / single-instance design

The existing named-pipe/Unix-socket bind-as-lock + `forward_open_new_window` pattern (§1) is already the right shape and needs no redesign — it matches the strongest documented precedent found (VS Code: attempt to connect to the expected pipe/socket path first; success means "already running, forward and exit"; failure means "bind it, become the instance"). Two concrete gaps to close, not architectural changes:

1. **Close the `promote_pool_window` liveness gap** (§1, the VM-suspend retro's still-valid code-level finding) before this feature ships — verify the reattached window's actual connection health, not just OS handle existence, and fall back to spawning a genuinely fresh window on a timeout. This gap is latent today (rarely hit) but becomes a routine, everyday path once "reattach to a backend that's been running unattended for hours" is the normal flow rather than an edge case.
2. **Prefer OS-native service-manager restart policies over a bespoke watchdog where available** (systemd `Restart=on-failure`, launchd `KeepAlive`, Windows Scheduled Task's own retry settings) for the persistent-service half, and have the tray/UI process's own reconnect-on-timer already be its recovery mechanism for the "service restarted, am I still attached" question — decoupling "is the service alive" from "is it literally the same OS process," per the researched update-handoff failure mode (a resident process that never fully exits can block its own update from ever applying — confirmed as a real Squirrel.Windows failure class).

## 6. Security / consent design — non-negotiable defaults

Synthesized from OWASP's Agentic Security Initiative guidance plus three cautionary precedents researched (Zoom 2019's persistent-hidden-local-server debacle, the 2025 Cluely breach, Microsoft Recall's 2024 forced opt-out-to-opt-in reversal) — all three failure shapes are directly on-point given this feature is specifically "a persistent background service that can execute shell commands":

- **Opt-in and disabled by default.** Never silently enabled by an installer or an update.
- **The tray icon itself is the transparency mechanism** — it must be a reliable, always-accurate indicator of "the background service is running," not just a launcher shortcut. (A real user report exists of exactly the failure to avoid: a well-known password manager's tray icon disappearing while its background agent kept running unlocked, with no visible indicator at all.)
- **One complete, actually-tested disable/uninstall path** that removes the LaunchAgent/Scheduled-Task/autostart-file artifact itself, not just closes the window — Zoom's 2019 incident is the canonical failure of this exact property (a hidden background service that survived "uninstall" and could silently reinstall the full app).
- **No admin/root elevation required anywhere in the design** (already satisfied by the Scheduled-Task-not-Service / LaunchAgent-not-Daemon / systemd-user-not-system choices above).
- **Per-action confirmation tiers for destructive operations even while unattended** — a background agent silently running shell commands with no one watching is precisely the scenario this repo's own jekt trust-layer work (see `CLAUDE.md`'s Jekt security rules) already reasons carefully about for agent-to-agent messages; the same "when in doubt, escalate to a human" discipline applies here for agent-to-shell actions taken with no window open to show the human anything in real time.
- **Immutable audit logging of what the background service did while unattended**, surfaced the next time a window (or the tray panel) opens — the direct answer to "there was no one watching."

## 7. Phased rollout (recommended sequencing)

1. **Prerequisite, ship first, independent of tray work**: fix the `promote_pool_window` liveness gap (§5.1) and decouple `host`'s clean-exit path from srv/launcher teardown (§1) — both are required underpinnings, neither requires a tray icon to exist yet, and both are independently testable today.
2. **Throwaway prototype, not a shipped feature**: verify `tray-icon`+`muda` actually coexists with `agentmux-launcher`'s own event loop on Windows and macOS specifically — NOT `host`'s CEF loop, since §2 places tray ownership in `launcher` precisely so the icon persists across `host` crashes/restarts. (Codex P2 on this PR's own review caught this document's Phase-2 wording pointing at the wrong process — the coexistence risk that actually matters is with whatever message pump `launcher` already runs for its splash window, not CEF's.) This is the one genuinely unverified architectural assumption in this whole design; don't commit further engineering to the approach before confirming it.
3. **Windows first** (most mature crate support, code-signing pipeline most likely already needed regardless), then macOS (notarization prerequisite, `SMAppService` consent UX), then Linux (`ksni`, with the GNOME-extension caveat documented up front).
4. **Auto-start registration itself ships last, opt-in, and off by default even after the tray icon exists** — a user can have "close-to-tray keeps it running for this session" without "starts automatically at every login" being bundled into the same toggle; keep those two decisions separate in the UI.

## 8. What this spec does not decide

- Does not pick a final visual design for the simplified tray panel — only that it should be a small CEF window reusing the pool-window mechanism, not a native menu or a separate process.
- Does not resolve whether the tray-triggered panel and the full main window share one `srv` connection or the panel gets its own lightweight one — needs the Windows/macOS event-loop prototype (§7.2) done first to know what's actually feasible.
- Does not commit to a specific update/auto-update mechanism for the now-persistent background service — flagged as a real open question (§5.2) but not designed here.

## References

| Topic | Source |
|---|---|
| Process model / job objects | `agentmux-launcher/src/supervisor/windows.rs`, `unix.rs` |
| Single-instance + reattach | `agentmux-launcher/src/ipc/mod.rs`, `second_instance.rs` |
| Hidden-window primitive | `agentmux-cef/src/commands/window_pool.rs` |
| Reattach liveness gap | `docs/retro/retro-fresh-vm-suspend-orphaned-frontend-2026-09-03.md` |
| `tray-icon`/`muda` | https://github.com/tauri-apps/tray-icon, https://github.com/tauri-apps/muda |
| `ksni` | https://crates.io/crates/ksni |
| GNOME tray extensions | https://extensions.gnome.org/extension/615/appindicator-support/, https://extensions.gnome.org/extension/9164/status-tray/ |
| Raycast architecture | https://www.raycast.com/blog/a-technical-deep-dive-into-the-new-raycast |
| GitHub Copilot session model | https://docs.github.com/en/copilot/how-tos/copilot-on-github/use-copilot-agents/manage-and-track-agents |
| VS Code single-instance | https://deepwiki.com/microsoft/vscode/1.1-application-startup-and-process-architecture |
| `SMAppService` / Login Items | https://support.apple.com/guide/deployment/manage-login-items-background-tasks-mac-depdca572563/web |
| SmartScreen/SAC reputation changes | https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation |
| OWASP AI Agent Security | https://cheatsheetseries.owasp.org/cheatsheets/AI_Agent_Security_Cheat_Sheet.html |
| Zoom 2019 hidden-server incident | https://www.securityweek.com/mac-zoom-web-server-allows-remote-code-execution/ |
