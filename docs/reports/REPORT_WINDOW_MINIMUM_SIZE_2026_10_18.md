# Report: The minimum window size, on Windows, Linux and macOS

**Date:** 2026-10-18
**Author:** Maricon@charlie
**Status:** implemented (the shared floor in `crates/cef/src/app/mod.rs`); checked on Linux; the macOS and Windows-floater numbers are still to measure
**Requested by:** the operator: *"on windows we have a minimum width and height for the agentmux windows ... but appears missing on linux. lets make sure linux macos and windows all honor the minimums set in windows, report back with the numbers you see."*

## 1. The numbers

| Window | Windows | Linux | macOS |
|---|---|---|---|
| Normal window, width | **380** (set in code) | was **none**: 100 accepted; now **380** (measured) | was none; now **380** (to measure) |
| Normal window, height | **39** (the OS enforces it: the code asks for 1) | was **none**: 20 accepted; now **39** (measured) | was none; now **39** (to measure) |
| Floating pane | 240 × 140 (the page's own edge-resize clamp; no OS floor; not measured on a real window) | 240 × 140 (now enforced by the window too; measured) | 240 × 140 (same; to measure) |

DIP at a 96 DPI scale (100%) throughout; Windows' 39 scales with the display (see §3).

## 2. What I measured, and how

- **Windows (Posa@Area54, over CDP, no real input):** asked `setRect("main", {width: 100, height: 20})`; `getRect` returned **380 × 39**. `GetWindowRect` on the same window gave 380×39 physical, and `innerWidth/innerHeight` and `outerWidth/outerHeight` were both 380×39. One monitor, 100% scale (AppliedDPI 96, `devicePixelRatio` 1). Other scales not tested. A real floating pane (240×140) was not measured: it needs a real tear-off, which means mouse input.
- **Linux, before (charlie, GNOME 50, native Wayland; `task dev` of main, over CDP):** `setRect("main", …)` accepted every size asked for: 300×600, 200×600, 100×600, 800×300, 800×150, 800×60, 800×20 and 100×20 all came back as asked. **No floor in either dimension.**
- **Linux, after (same dev setup, with the change):** 100×600 → **380×600**; 800×20 → **800×39**; **100×20 → 380×39**, the same answer Windows gave; 800×150 and 800×60 unchanged. A floating pane (torn off in the dev instance): 100×20 → **240×140**; 300×100 → 300×140; 240×140 stays.
- **macOS:** the code had no floor either (§3). To measure with Clare@starpower.

## 3. Why

`crates/cef/src/app/mod.rs` has one window delegate, `AgentMuxWindowDelegate`, for every normal window (main, new windows, popups). Its `minimum_size` override was compiled for Windows only (`#[cfg(target_os = "windows")]`) and returned `380 × 1`, with a comment: floor the width at 380 so the tab strip and widget bar can't be cramped, and leave the height to the OS. So:

- **Width 380** is ours. **Height 39** is not ours: Windows enforces its own minimum track size for a resizable window (39 px at 96 DPI; it grows with the display scale), because the code asks for only 1.
- Linux and macOS had no override at all, so nothing stopped a window getting as small as the compositor or the window server allowed.
- Floating panes are raw `WS_POPUP` windows on Windows, outside this delegate. The page clamps their edge-resize to `MIN_W = 240`, `MIN_H = 140` (`floating-pane-workspace.tsx`). On Linux and macOS a floating pane goes through the same delegate, so a blanket 380 would have made them stricter than on Windows.

## 4. The change

One function, `window_min_size(label)`, gives the same answer on every platform:

| Window | Minimum |
|---|---|
| a floating pane (`floating-…`) | 240 × 140 (matches the page's clamp, now enforced by the window too) |
| every other window | 380 wide; 39 tall on Linux and macOS; Windows keeps 1 and lets the OS apply its own 39 (so it scales with the display, and Windows' behaviour is unchanged) |

Two unit tests cover it (`window_min_size_tests`). The Linux result is in the PR description.

## 5. Decisions for the owner

| | Question | Recommended |
|---|---|---|
| D1 | A 39 px height floor on Linux and macOS (the number Windows shows) | yes: 39 shows the 33 px header and a sliver of content; it matches Windows |
| D2 | A larger height floor on all three (for a usable window, not just "matches Windows") | no, not now: it would change Windows; say a number if you want one |
| D3 | Windows asks for 1 and lets the OS decide, rather than stating 39 itself | leave it: that keeps the floor correct at every display scale |
