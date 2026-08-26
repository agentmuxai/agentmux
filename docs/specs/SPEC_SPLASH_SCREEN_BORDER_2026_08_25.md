# SPEC — Splash screen: add a darkened 2px border, across all 3 platforms

**Date:** 2026-08-25
**Author:** AgentY
**Status:** Draft
**Scope:** the launcher's native splash window only — `agentmux-launcher/src/splash.rs` (Windows), `splash_mac.rs` (macOS), `splash_linux/{mod,x11,wayland}.rs` (Linux). No other window (main app, modals) is in scope.
**Related:** `docs/specs/SPEC_LINUX_SPLASH_POLISH_2026_06_20.md` (added Linux's rounded corners + fade — no border), `docs/specs/SPEC_LINUX_SPLASH_SESSION_AWARE_2026_06_20.md`.

---

## 1. Current state (audited against source)

The splash is **not** a shared web view — each platform draws it natively, with its own separate implementation:

| Platform | File | Mechanism |
|---|---|---|
| Windows | `splash.rs` | `WS_POPUP` + `WS_EX_LAYERED` GDI layered window; pixels composed into a top-down DIB (`CreateDIBSection`) every frame |
| macOS | `splash_mac.rs` | Borderless `NSWindow` (`styleMask 0`) with a layer-backed `NSView` backdrop |
| Linux | `splash_linux/mod.rs` (+ `x11.rs`/`wayland.rs`) | `override_redirect` X11 window or `xdg_toplevel` Wayland surface; backdrop composited per-pixel into an ARGB8888 buffer (`render_frame`) |

All three agree on one dark backdrop, `#1A1A1F` (`BG_R/G/B`), no light-mode variant. **None of the three currently draws a window border/stroke:**

- **Windows** (`splash.rs`): flat opaque DIB fill, square corners. Only a 1px *internal* separator line exists (`SEP_R/G/B = 0x36`, between the stage list and footer) — not a window edge.
- **macOS** (`splash_mac.rs`): `CORNER_RADIUS = 16.0` via `layer.setCornerRadius:`, plus `setHasShadow:1` (a drop shadow, not a border). No `borderWidth`/`borderColor` set anywhere.
- **Linux** (`splash_linux/mod.rs`): `CORNER_RADIUS_PX = 16.0` (comment: "Matches splash_mac.rs"), rounded via alpha-masked `corner_coverage()` in `render_frame`. Same story — rounding only, no stroke.

**Pre-existing asymmetry, unrelated to this ask but worth flagging:** Windows has square corners while macOS/Linux have 16px rounded corners. This spec does not propose fixing that — see §5.

## 2. Desired behavior (repo-owner-specified)

> "lets add a darkened border 2 PX thick on the border on the splash screen, across 3 platforms"

A 2px stroke, in a color darker than the `#1A1A1F` backdrop, drawn at the window's edge (following whatever corner treatment that platform already has — square on Windows, 16px-radius on macOS/Linux) — consistently on Windows, macOS, and Linux.

**Color:** no existing token for "darker than `#1A1A1F`" — propose `#0D0D10` (roughly half the backdrop's RGB values, a straightforward "darkened" reading) as a literal constant defined once and referenced by name on all three platforms (e.g. `BORDER_R/G/B` alongside the existing `BG_R/G/B` constants), so a future palette change only needs updating in one place per platform rather than three independently-tuned hex triples. Confirm the exact shade live (`task dev`/packaged build) before finalizing — this is a starting proposal, not a final color decided by comparison.

## 3. Proposed design, per platform

### 3.1 Windows (`splash.rs`)

The DIB is filled manually per-pixel (no native window-chrome border primitive available for a layered `WS_POPUP` window). Add a border pass after the background fill and before the rounded-corner logic doesn't apply here (Windows has square corners) — for each pixel, if it falls within 2px of any edge (`x < 2 || x >= W-2 || y < 2 || y >= H-2`), write `BORDER_R/G/B` instead of the background/content color. Must run **after** all other content is drawn (stage list, separator, footer) so the border isn't overdrawn by them, or equivalently, inset all existing content drawing by 2px so it never reaches the border region in the first place — the exact ordering depends on how `splash.rs`'s draw loop is structured; the implementer should read the current fill/content sequence before choosing.

### 3.2 macOS (`splash_mac.rs`)

Trivial — `CALayer` has native border support. Alongside the existing `layer.setCornerRadius:` call (~line 1038-1039), add:
```
layer.setBorderWidth: 2.0
layer.setBorderColor: <CGColor for BORDER_R/G/B>
```
Both are single `objc_msgSend` calls, matching the pattern already used for `setCornerRadius:`/`setHasShadow:` in this file. The border draws inset from the layer's rounded-rect path automatically — no extra corner math needed.

### 3.3 Linux (`splash_linux/mod.rs`, `x11.rs`, `wayland.rs`)

No native layer-border primitive here (raw ARGB8888 buffer composited manually) — needs the same "ring" approach as Windows, adapted to the existing rounded-corner alpha-masking. `corner_coverage()` already computes, per-pixel, how much of a pixel near a corner is inside vs. outside the rounded rect (for antialiasing the mask). Add a second coverage band: pixels whose distance from the rounded-rect boundary falls within the 2px border width get `BORDER_R/G/B` (blended via the same coverage-based antialiasing already used for the corner mask, for a clean edge instead of a jagged one), pixels further inside get the normal background/content. `x11.rs`/`wayland.rs` don't need their own changes — both already just pass `radius`/`window_alpha` through to `render_frame`, which is where the actual pixel logic lives.

## 4. Verification

- **No unit-testable surface** — this is raw pixel-drawing code on all three platforms; there's nothing here that a Rust test asserts against today (confirmed: none of the three splash files has a test file in this repo).
- **Visual verification is mandatory and cannot be skipped** — per this repo's own established caveat for UI changes (see e.g. `SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md` §4's identical note), a sandboxed dev environment has no display. Confirm on a real Windows machine via `task dev` or a packaged build (`task package`) — the launcher's splash only renders during actual startup, not inside the CEF-hosted app itself, so `task dev`'s hot reload doesn't cover it; a full relaunch is needed to see a splash-code change. macOS/Linux verification needs those platforms' own build environments — this repo runs on Windows per this session's own machine, so cross-platform verification is a real gap to flag to whoever picks this up, not something this spec can close on its own.
- Confirm the border reads as "darkened," not as a lighter mis-tint, against the actual `#1A1A1F` backdrop on a real display (color perception on different panels can shift a marginal proposal like `#0D0D10` — verify live per §2).

## 5. Non-goals

- **Not** unifying Windows' square corners with macOS/Linux's 16px rounding — a real, pre-existing asymmetry, but a separate, larger visual decision than "add a border," and changing it isn't necessary to satisfy this request (a square-cornered 2px border on Windows and a rounded 2px border on macOS/Linux both read as "the splash now has a darkened border" — they just differ in corner treatment exactly as the rest of the window already does).
- **Not** adding a light-mode splash variant — none exists today, out of scope here.
- **Not** touching the internal 1px separator line inside the Windows splash (`SEP_R/G/B`) — a different, already-existing internal-content divider, unrelated to a window-edge border.
