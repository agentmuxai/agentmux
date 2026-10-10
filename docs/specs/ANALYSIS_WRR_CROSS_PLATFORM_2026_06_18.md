# Analysis: Does WRR need a macOS/Linux equivalent?

- **Date:** 2026-06-18
- **Author:** AgentO (Masty)
- **Question:** Should Window Reality Reconciliation (WRR) — today Windows-only — be extended to macOS/Linux?
- **TL;DR:** **No, not a wholesale port.** The identity layer WRR sits on is already cross-platform; WRR's observation drift-classes are mostly Windows-specific defenses. There is exactly **one** cross-platform gap worth closing (crash-orphan detection), and it's far cheaper to fix it directly than to replicate WRR's native-event machinery (which is infeasible on Wayland anyway).

---

## 1. What WRR actually is

An **observation-only** drift sensor (Phase B.9.1; see `docs/retro/wrr-design-2026-04-28.md`). The host turns Win32 window events (`SetWinEventHook` + `WM_WINDOWPOSCHANGED`/`WM_DISPLAYCHANGE`) into `Command::ReportHwnd*`, and the launcher reducer classifies drift between CEF identity and Win32 reality (`HiddenSinceOpen`, `OffMonitor`/`BoundsDiverged`, `HwndWithoutBrowser`, `OrphanDestroy`, `LingeringHwnd`). Enforcement (Phase F) never really shipped — the only corrective action is a Windows-only `SetWindowPos` move (`agentmux-cef/src/ui_tasks.rs:937`, an explicit no-op elsewhere). It was motivated by one concrete Windows bug: a stray taskbar entry from an off-screen `open_new_window`.

The module is gated Windows-only and stubbed elsewhere (`agentmux-cef/src/wrr/mod.rs:34` — `install_hooks` is a no-op on non-Windows, with the comment *"WRR is Windows-only — Phase 7 will revisit"*).

## 2. What's already cross-platform (the part that matters for correctness)

- **Window identity.** CEF `on_after_created` → `report_window_opened` and `on_before_close` → `report_window_closed` are wired on **all** platforms and feed the same launcher reducer. This is the canonical "what windows exist" layer; WRR only adds a Win32-reality projection on top.
- **Orphan reconciler.** `agentmux-cef/src/commands/orphan_reconcile.rs` (`plan_reconcile`) is structurally cross-platform and closes orphaned pool browsers via platform-neutral `close_browser`.
- **Renderer-crash signal.** `on_render_process_terminated` is wired cross-platform (`agentmux-cef/src/client/handlers.rs:396`, `client/mod.rs:1491`).

## 3. What's genuinely missing on macOS/Linux

The OS-reality **feed**. The host only emits `ReportHwnd*` on Windows (`agentmux-cef/src/client/mod.rs:573`), so the launcher's `wrr` reducer arm **compiles but is dead code** on macOS/Linux packaged builds. The single consequence that bites:

- **Crash-orphan detection.** The reconciler's per-browser liveness check is hard-coded to `HwndStatus::Live` on non-Windows (`orphan_reconcile.rs:432`); only the Windows arm does a real `IsWindow` probe. So a renderer/window that **dies without firing `on_before_close`** (the canonical WRR `OrphanDestroy` case — renderer crash takes the window) is **undetectable** on macOS/Linux: the reconciler can't tell a zombie from a live browser, and the last-window-closed quit cascade that Windows derives from `WM_DESTROY` has no native equivalent.

The other drift-classes (`OffMonitor`, `HiddenSinceOpen`, `LingeringHwnd`) are also unobserved off Windows, but they were observation-only warnings, and macOS's window server / typical X11 WMs largely prevent the stranded-window cases on their own. Low value.

## 4. Why a port is the wrong shape

"Extend WRR" really means **three separate native observation backends** feeding the existing reducer:

- **macOS:** an `NSWindowDelegate` swizzle observing `windowWillClose`/`windowDidMove`/`windowDidChangeScreen` + `NSApplicationDidChangeScreenParameters`, and a `[NSApp windows]`→label registry.
- **Linux/X11:** `StructureNotify` (`MapNotify`/`UnmapNotify`) + XRandR `RRScreenChangeNotify`, plus `XQueryTree` enumeration.
- **Linux/Wayland:** **no portable API** for per-window close/hide or monitor topology. Compositor-specific. Effectively not implementable as a library.

That's a large, three-platform native surface (with an unsolvable Wayland leg) to resurrect mostly-Windows-specific defenses. Not worth it.

## 5. The real macOS/Linux window bugs are NOT WRR-shaped

The open native-window bugs need native window APIs, not a reality sensor — a WRR port wouldn't touch them:
- macOS tear-off ghost-pane crash (needs `NSDraggingDestination`).
- macOS floating-pane redock hit-test (needs `cef::Window::bounds()` / `[NSApp orderedWindows]`).
- Wayland visible pool windows (the `-32000` off-screen hack is ignored; needs a real hide mechanism).

## 6. Recommendation

1. **Do not replicate the Win32 event-hook layer on macOS/Linux.** Keep WRR Windows-only by design.
2. **Close the one cross-platform gap — targeted, not a port.** Make `orphan_reconcile.rs`'s liveness check real on macOS/Linux (query `cef::Window`/`BrowserHost` validity instead of hard-coding `Live`) and trigger reconciliation off the already-cross-platform `on_render_process_terminated`. This gives the crash-orphan safety net WRR provides on Windows, in ~tens of lines, with no native-observation machinery. **Filed as [#1569](https://github.com/agentmuxai/agentmux/issues/1569).**
3. **Track the native window bugs separately** (§5) — they're per-bug native work.
4. **Doc hygiene:** correct the "macOS/Linux equivalents on the roadmap" framing in the WRR docs to "Windows-only by design; the cross-platform safety net is the orphan reconciler," so nobody assumes parity. (`agentmux-cef/src/wrr/mod.rs:34` comment + `agentmux-docs` `internals/wrr.md`.)

## 7. Cost summary

| Option | Effort | Value |
|---|---|---|
| Full WRR port (3 native backends) | Very high; Wayland leg infeasible | Low — resurrects Windows-specific defenses |
| **Targeted crash-orphan fix (#2 above)** | **Low (~tens of LoC, cross-platform)** | **Closes the only real gap** |
| Native window bugs (§5) | Per-bug native work | High, but orthogonal to WRR |
