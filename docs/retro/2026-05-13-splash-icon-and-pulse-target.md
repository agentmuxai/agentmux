# Retro — splash icon + pulse target wrong on first ship

**Date:** 2026-05-13
**PR shipped:** #822 (merged sha squashed onto main 09:45)
**Smoke build:** 0.33.831 portable
**Reported by:** user, on first sight of the running splash

---

## What's wrong

Two issues with the splash that just shipped:

### 1. Wrong icon — low-res "stacked rectangles" instead of the brain

`agentmux-launcher/src/splash.rs:67` loads the embedded Win32 icon (resource ID 1):

```rust
let icon = LoadIconW(hinst, 1usize as *const u16);
```

That resource is `agentmux-cef/resources/win/agentmux.ico` — the stacked-rectangles app icon set up by winres at build time for the taskbar / Alt-Tab / window title. It's an `.ico` (multi-resolution but capped at the sizes winres bakes in), and it's **not the icon the user expects to see on the splash**.

The user expects the **brain logo** — the same SVG that renders as the "no panes loaded" background:
- `frontend/app/asset/logo-brain.svg`
- `assets/agentmux-logo-brain-alternate.png`
- `assets/agentmux-box-art-brain-alternate-3840x2160.png` (high-res)

Two distinct brand assets ended up at the splash by accident:
- **App icon** (`agentmux.ico`) — for OS surfaces (taskbar, Alt-Tab). Designed to read at 16px / 32px.
- **Brand logo** (brain) — for in-app surfaces. Designed to read at 200px+.

The splash is an in-app surface, not an OS chrome surface. The brain logo is what belongs there.

### 2. Pulse applied to the whole window instead of the logo

`splash.rs:144`:

```rust
SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
```

`SetLayeredWindowAttributes(_, _, _, LWA_ALPHA)` sets a global alpha for the ENTIRE layered window — background + icon together. The sine-wave envelope (lines 141–142) ramps the global alpha between ~160 and ~220, so the dark background fades in and out together with the icon.

The intended effect is the inverse: **solid background, pulsing logo**. Background should be a fixed, fully-opaque dark fill; only the logo's alpha should oscillate. The current implementation reads as "the whole window is breathing" which is visually busy and looks like a bug.

---

## Root cause

**Issue #1 — icon source confusion.** The spec / PR #822 said "shows the embedded app icon". The author reached for the resource winres exposes (the `.ico`), which is the *system app icon* — different brand from the in-app logo. Nobody on review caught the distinction. Bot reviewers (reagent / codex) don't have brand-asset context — they verified the code works, not whether the right asset was selected.

**Issue #2 — implementation took the easy path.** `SetLayeredWindowAttributes(LWA_ALPHA)` is one line and obvious. Per-element alpha (pulsing only the icon while keeping the background opaque) needs `UpdateLayeredWindow` with a pre-composited DIB section, OR per-frame `AlphaBlend` calls in WM_PAINT. Both are 30–60 lines and require a per-pixel alpha bitmap. The simpler path was taken.

In hindsight, "pulsing brain on solid background" is what the spec's intent was — but the spec said "alpha-pulsed splash" without specifying which thing pulses. The implementation chose the wrong subject.

---

## What the fix should look like

**Follow-up PR scope** (don't fold into anything else):

### Asset pipeline
- Embed `assets/agentmux-logo-brain-alternate.png` (or render the SVG at build time to a 128×128 / 256×256 PNG) as a raw resource in `agentmux-launcher`.
- Load it at splash spawn via `LoadImageW` with `IMAGE_BITMAP` (not `IMAGE_ICON`) so we get a proper bitmap with alpha channel preserved.

### Rendering
- Switch from `SetLayeredWindowAttributes(LWA_ALPHA)` for the global pulse to a **per-frame composite** approach:
  - Create a memory DC + DIB section once at spawn time
  - On each pulse tick: clear the DIB to `BG_COLOR` (solid), then `AlphaBlend` the brain bitmap on top with the current pulse alpha
  - Use `UpdateLayeredWindow` with `ULW_ALPHA` to push the composited DIB to the screen
- Background stays fully opaque (alpha 255 in the DIB everywhere), only the brain bitmap's blend factor oscillates.
- Fade-out at the end of the splash lifetime is straightforward: ramp the OVERALL `UpdateLayeredWindow` alpha (the BLENDFUNCTION parameter) — that's the one case where everything fades together, which is correct.

### Spec touch-up
Update `agentmux-launcher/src/splash.rs` module doc:
- "shows the app icon" → "shows the brain logo on a solid dark background, with the logo pulsing"
- Note that the icon is the brand logo (`logo-brain`), not the OS app icon (`agentmux.ico`).

---

## Lessons

1. **"App icon" is ambiguous in this codebase.** OS icon ≠ brand logo. Specs that say "show the icon" need to qualify *which* icon. Add a glossary entry to `CLAUDE.md` if this keeps biting.
2. **Pulse subject must be specified in the spec.** "Alpha-pulsed splash" doesn't say what pulses. Future visual specs should mock the target frames (or at least say "background X, foreground Y, pulse target = Y").
3. **Brand assets need a launcher-accessible path.** Today the brain SVG lives in `frontend/app/asset/` — `agentmux-launcher` can't reach it without copying or symlinking. A `assets/launcher/` dir checked into the repo with launcher-ready PNGs avoids the per-build asset shuffle.
4. **Bot reviewers can't see brand decisions.** Visual / branding bugs slip through reagent + codex. For PRs with visual output (splash, themes, icons), include a screenshot in the PR body so a human reviewer can sanity-check the look without building.
5. **`SetLayeredWindowAttributes(LWA_ALPHA)` is a foot-gun for partial pulses.** It's globally-applied. For element-level animation you need `UpdateLayeredWindow` + per-pixel alpha. Document this in any future Win32 layered-window work.

---

## Action items

- [ ] **Fix splash** — follow-up PR per "What the fix should look like" above. Owner: TBD. Target: next portable build cycle.
- [ ] **CLAUDE.md glossary** — add "App icon vs. brand logo" disambiguation. ~10 lines.
- [ ] **PR body template for visual PRs** — require a screenshot or animated GIF. Tracked in the PR-template repo (separate effort).
