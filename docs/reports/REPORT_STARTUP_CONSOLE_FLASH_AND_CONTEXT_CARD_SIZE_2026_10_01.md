# Report: startup console flash, and oversized "Given to the agent" card

Date: 2026-10-01 · Author: Loap · Base: `main` @ `48f511ca9` (one commit past v0.59.4)
Status: analysis only. No source files were changed. Neither issue was reproduced by running the app.

---

## 1. Console window flashing during the splash

### Symptom
On startup, while the splash is visible, a window that looks like a command terminal appears briefly and disappears.

### Mechanism
- The launcher and CEF host are GUI-subsystem binaries in release (`crates/launcher/src/main.rs:22-25`, `crates/cef/src/main.rs:6-9`). Debug launcher builds are console-subsystem.
- **srv is a console-subsystem binary.** It has no `windows_subsystem` attribute, and `crates/srv/build.rs` only sets version info. The launcher hides it by spawning it with `CREATE_NO_WINDOW` (`crates/launcher/src/srv_spawner.rs:248`, `:463-464`).
- Windows allocates a fresh console for any console-subsystem process started from a console-less parent without `CREATE_NO_WINDOW`. Redirecting stdio does not prevent it.
- Almost every spawn site already sets the flag, via `agentmux_common::win32::CREATE_NO_WINDOW` (`crates/common/src/win32.rs:27`) and the helpers in `crates/common/src/cli.rs:24-37` and `crates/srv/src/util.rs:31-36`. The flash comes from the sites that bypass it.

### Culprit 1 (most likely, about 75%): launcher `schtasks.exe` at every startup
Chain, all before the host window exists:
1. `crates/launcher/src/supervisor/windows.rs:456` calls `notify::start(...)` once srv is ready.
2. srv sends an initial `config` event on WebSocket connect (`crates/srv/src/server/websocket.rs:100-113`).
3. The launcher parses it as `Frame::StartAtLogin` (`crates/launcher/src/notify/mod.rs:317-319`) and calls `start_at_login::observe`. The first frame always passes the dedupe check, so it spawns the thread `reconcile_latest`.
4. `reconcile_latest` (`crates/launcher/src/start_at_login.rs`) calls `autostart::read_entry()` unconditionally, which runs `schtasks /Query /TN <id> /XML` (`crates/launcher/src/autostart/mod.rs:612`).

Verified: none of the four `schtasks` spawns in `autostart/mod.rs` set `creation_flags` (lines 416, 453, 501, 612). Only the query at 612 runs on every startup. The others run only when the plan is Write or Remove.

`schtasks.exe` is a console program started from a GUI process, so it gets a console that lives only as long as the query, which matches "runs quickly and disappears". The code arrived in `5f955e4e5` (#3788, 2026-09-25, start-at-login), so it is a recent regression. The earlier flash fixes (#2171, #2173, #2174) predate it.

### Culprit 2 (real gap, about 35% as the visible flash): srv `--crash-monitor` self-spawn
- `crates/srv/src/crash_monitor.rs:173-178` runs `Command::new(current_exe).arg("--crash-monitor")` with null stdin/stdout and no creation flags.
- It is called from `install_crash_guard()` (`crates/srv/src/bootstrap/watchers.rs:235`), essentially the first thing srv does, on Windows only.
- Observed on this machine: the running monitor child has a `conhost.exe` child created in the same second, so a console was allocated for it.
- It is ranked lower because the monitor lives for srv's whole lifetime, so a visible window would persist rather than flash, and no top-level window owned by either PID was found. It may be hidden by Windows Terminal's default-terminal setting, or only visible for the moment the console is created. This code dates from #246, so it is old.

### Ruled out
These all set `CREATE_NO_WINDOW`: `launcher/src/splash_info.rs:106`, `cef/src/sidecar.rs:310`, `cef/src/commands/backend.rs:269`, `cef/src/commands/autostart.rs:42`, `common/src/process.rs:168`, `common/src/runtime_mode.rs:431`, and the srv tool-store, MCP probe, CLI handler, PTY, LSP, ACP and subprocess spawns.

Not on the startup path: `launcher/src/host_hang_dump.rs:243` (only on a host hang), `launcher/src/upgrade.rs:205,251` (only during upgrade), and `cef/src/commands/platform.rs` and `cli_login.rs` (on user action; `cli_login.rs:1331` uses `CREATE_NEW_CONSOLE` on purpose).

### Recommended fix
1. Add `CREATE_NO_WINDOW` to all four `schtasks` spawns in `crates/launcher/src/autostart/mod.rs`, ideally through one small helper so the next `schtasks` call cannot forget it.
2. Add it to the `--crash-monitor` spawn in `crates/srv/src/crash_monitor.rs`.
3. Add a regression guard, such as a test or lint that greps for `Command::new` in the Windows-reachable crates without a creation flag or an allowlist entry, because this class has recurred at least three times (#2171, #2173, #3788).

### Not verified
- That the splash is still showing when the `schtasks` call runs. The code order says yes.
- That srv's initial `config` event reaches the launcher quickly.
- Whether the crash monitor's console is ever visible.

To confirm before or after the fix, capture process creation during startup. Process Monitor with a filter on `schtasks.exe` or `conhost.exe`, or `Get-CimInstance Win32_ProcessStartTrace`, will show which one fires during the splash.

---

## 2. "Given to the agent…" card is too large

### What renders it
- Title string: `frontend/app/view/agent/context-delivery.ts:67`, "Given to the agent · session continued · N items" on a resume. Reason labels are at `:38-43`.
- Component: `frontend/app/view/agent/components/ContextDeliveryCard.tsx`. Title span at `:152-153`, item rows (`ItemHead`) at `:85-134`, expanded `<pre>` body at `:202`.
- Mounted by `frontend/app/view/agent/virtualization/DocumentRow.tsx:257-263`.

### Cause
Styles are in `frontend/app/view/agent/styles/_document-nodes.scss:1170-1293`. Verified: these selectors set no `font-size`, so they inherit the agent pane base of 15px (`--font-agent-size`, `agent-view.scss:420`, `theme.scss:390`):

| Selector | Currently |
|---|---|
| `.agent-context-delivery-title` | 15px, weight 600, `--main-text-color` |
| `.agent-context-delivery-item-name` | 15px |
| `.agent-context-delivery-body` (`<pre>`) | 15px (inherited) |
| title and item icon glyphs | unsized |

The neighbours around them are 11-12px: chips, the cut mark, item size and card size are 11px, and the excerpt and advice are 12px. So the card mixes a 15px bold, full-contrast title with 11px badges. The block's own comment says it should be "quieter than a jekt: it's context, not a message", but the title is the loudest line in the row.

Comparison with sibling meta rows:
- Dividers and notices (session outcome, CLI notice, context compacted, memory reinjection, history link) use 11px labels in `--secondary-text-color`.
- Markdown-canceled header is 11px mono. Tool previews are 11-12px.
- Thinking blocks are muted and italic. User-message notes use `0.85em`.

It is not markdown heading inflation (the body is a plain `<pre>` with `LinkifiedText`), and it is not the pane zoom (a single CSS `zoom` on `.agent-view-zoomed`).

### Proposed change (CSS in `_document-nodes.scss`, `.agent-context-delivery` block)
- `.agent-context-delivery-title`: `font-size: 12px` (or `var(--text-sm)`), `font-weight: 500`, and consider `--secondary-text-color`, with main-text on hover or when expanded.
- `.agent-context-delivery-item-name`: `font-size: 11px` or `12px`, `color: var(--secondary-text-color)`.
- `.agent-context-delivery-body`: `font-size: 11px` or `12px`, `line-height: 1.35`, matching the jekt body and tool previews.
- Title and item icons: `font-size: 11px; flex-shrink: 0`.
- Keep the 2px left border, the 3% background, and the existing chevron mixin.

Conventions:
- Use CSS vars only (`--secondary-text-color`, `--main-text-color`, `--border-color`, `--warning-color`, `color-mix(...)`). `eslint.theme-colors.config.js` bans Tailwind palette utilities in class strings, so add none in the TSX.

### Must change together: virtualiser height estimates
`frontend/app/view/agent/virtualization/renderers.ts:169-172` hardcodes `CONTEXT_DELIVERY_TITLE_PX = 30`, `EXCERPT_PX = 22`, `ITEM_HEAD_PX = 24` and `ADVICE_PX = 40`. Shrinking the fonts without lowering these leaves the row estimates too tall until measured. Suggested starting values are about 24 for the title and about 20 for the item head. Check them in the UI.

### Tests
- No test asserts font size or computed style, and there is no visual-regression convention here, so a CSS-only change breaks nothing.
- Keep these class names and strings: `DocumentRow.test.tsx` (about `:508-680`) queries `.agent-context-delivery-body` and `-chevron` and checks title text, including "Given to the agent · new session · 4 items". `context-delivery.test.ts:141-165` asserts the exact titles.
- If the `renderers.ts` constants change, check for any test that asserts the estimates. This was not searched.

### Not verified
- Which part the user finds largest (title, item names or body). The 15px bold title and the item names are the likeliest.
- The `<pre>` body's real computed size. Its monospace default can differ slightly from the inherited 15px, and there is no global `pre` reset. Compare it against `.agent-jekt-body`.
- The look across themes, and the effect on virtualised row heights.

---

## Suggested order
1. Fix the `schtasks` and crash-monitor flags (small, Rust-only, user-visible on every launch).
2. Resize the context card and update the `renderers.ts` constants in the same PR.
3. Check both visually with `UIScreenshot` or `CaptureWindow` on a running dev build.
