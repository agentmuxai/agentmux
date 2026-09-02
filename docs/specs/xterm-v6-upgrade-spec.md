# xterm.js v6.0.0 Upgrade Spec

**Date:** 2026-03-28
**Author:** Agent1
**Status:** Approved — implementing
**Risk Level:** High — renderer pipeline change + private API usage

---

## Current State

### Versions (pinned in package-lock.json)

| Package | Current | Latest Stable | Delta |
|---------|---------|---------------|-------|
| `@xterm/xterm` | 5.5.0 | **6.0.0** | Major |
| `@xterm/addon-canvas` | 0.7.0 | **0.7.0 (DEPRECATED)** | Removed in v6 |
| `@xterm/addon-fit` | 0.11.0 | 0.11.0 | Custom fork |
| `@xterm/addon-search` | 0.16.0 | 0.16.0 | Same |
| `@xterm/addon-serialize` | 0.14.0 | 0.14.0 | Same |
| `@xterm/addon-unicode-graphemes` | 0.4.0 | 0.4.0 | Same |
| `@xterm/addon-web-links` | 0.12.0 | 0.12.0 | Same |
| `@xterm/addon-webgl` | 0.19.0 | 0.19.0 | Same |

### Usage Footprint

| File | Lines | Purpose |
|------|-------|---------|
| `frontend/app/view/term/termwrap.ts` | 545 | Main wrapper — instantiation, addons, PTY, lifecycle |
| `frontend/app/view/term/term.tsx` | 361 | React component, search, theming, resize |
| `frontend/app/view/term/termosc.ts` | 320 | OSC handlers (7, 0/2, 9283, 16162) |
| `frontend/app/view/term/fitaddon.ts` | 101 | Custom FitAddon fork (noScrollbar + private API) |
| `frontend/app/view/term/filelinkprovider.ts` | 129 | File path link provider |
| `frontend/app/view/term/termtheme.ts` | 37 | Dynamic theme + transparency |
| `frontend/app/view/term/term-wsh.tsx` | 55 | WSH RPC (scrollback lines) |
| `frontend/app/theme.scss` | z-index block | xterm CSS z-index overrides |

### Key Architecture Notes

- **Linux forced to CanvasAddon** due to WebKitGTK WebGL bug (control sequences `\x08`, `ESC[K` render incorrectly). This is documented in CLAUDE.md as a critical workaround.
- **WebGL context loss** falls back to Canvas on macOS/Windows.
- **Custom FitAddon** uses private `_terminal._core` and `_terminal._core._renderService` APIs.
- **RAF-batched writes** coalesce PTY data into single `terminal.write()` per frame.
- **Cursor blink disabled** by default, dynamically toggled on focus/blur for CPU savings.
- **Copy-on-select**, custom wheel handler (macOS momentum), and custom key handler all registered.

---

## What v6.0.0 Changes

### Breaking Changes

| Change | Impact on AgentMux | Severity |
|--------|-------------------|----------|
| **Canvas addon removed** | Linux renderer breaks — currently forced to Canvas | **CRITICAL** |
| **`overviewRulerWidth` moved** to `overviewRuler.width` | Search decorations use overview ruler | Low |
| **`windowsMode` removed** | Not used by AgentMux | None |
| **`fastScrollModifier` removed** | Not used by AgentMux | None |
| **Scrollbar/viewport reworked** (VS Code base scrollbar) | Custom z-index overrides in `theme.scss`, FitAddon `noScrollbar` hack, macOS scrollbar hiding | **HIGH** |
| **Internal EventEmitter replaced** with VS Code Emitter | FitAddon uses `_terminal._core` private APIs | **HIGH** |
| **Alt key no longer maps to Ctrl+Arrow** | May affect keyboard navigation in terminal | Low |

### Benefits

| Benefit | Impact |
|---------|--------|
| **30% bundle size reduction** (379kb → 265kb) | Faster load, smaller WASM/Tauri bundle |
| **Synchronized output (DEC mode 2026)** | Eliminates screen tearing during fast streaming — directly helps AgentMux's multi-agent output |
| **ESM support** | Better tree-shaking with Vite |
| **Shadow DOM support** in WebGL | Future-proofs component isolation |
| **SearchLineCache** | Faster search in long scrollback buffers |
| **Memory leak fix** in CoreBrowserService | Important for long-running terminal sessions |
| **`onWriteParsed` API** | Could replace RAF-batched write workaround |
| **Smooth scroll frame limiting** | Reduces unnecessary repaints |

### New Addons Worth Considering

| Addon | Version | Value for AgentMux |
|-------|---------|-------------------|
| `@xterm/addon-progress` | 0.2.0 | Show agent task progress in terminal tab (ConEmu OSC 9;4) |
| `@xterm/addon-clipboard` | 0.2.0 | Native OSC52 clipboard — could simplify copy-on-select |
| `@xterm/addon-image` | 0.9.0 | Inline images — agent output with charts/screenshots |

---

## Critical Issue: Linux Canvas Fallback

### The Problem

AgentMux forces CanvasAddon on Linux (`termwrap.ts:398-406`) because WebGL doesn't render control sequences correctly on WebKitGTK. In v6, **CanvasAddon no longer exists**.

### Options

| Option | Pros | Cons |
|--------|------|------|
| **A. DOM renderer on Linux** | Zero addon needed, ships with core xterm | Slower rendering, no GPU acceleration, may struggle with fast agent output |
| **B. Test WebGL on latest WebKitGTK** | If the bug is fixed, use WebGL everywhere | Regression risk — bug has regressed before per CLAUDE.md |
| **C. Pin Canvas addon at 0.7.0, fork** | Keep working as-is | Unmaintained, will diverge from xterm core, potential breakage |
| **D. WebGL with DOM fallback on Linux** | Best of both worlds | Needs detection logic for when WebGL fails |

### Research Findings (March 2026)

This is **not an xterm.js bug** — it's a WebKitGTK WebGL2 implementation issue, confirmed across multiple projects:

**Evidence:**
- **Tauri #6559** (open, Jan 2026): WebGL Context Lost on Linux — `status: upstream`
- **Tauri #13157**: Glitchy rendering on Ubuntu 22.04 with WebKitGTK 2.48
- **Tauri Discussion #8524**: Tauri team states they "can't 100% recommend Tauri for Linux" due to WebKitGTK
- **xterm.js #4749**: Green artifacts with `allowTransparency` on WebKit/Linux (AgentMux uses `allowTransparency: true`)
- **xterm.js #4779**: Canvas was kept "pretty much exclusively for cases where WebGL would not work like some Linux setups" — removed in v6 anyway
- **WebKit Bug 228268**: GTK4+NVIDIA = "terribly broken, almost blank screen"

**Root cause:** WebGL texture atlas doesn't visually redraw after control sequences. The terminal buffer state is correct (PTY round-trip works), but the GPU-rendered output is stale. Particularly bad on NVIDIA (proprietary + Nouveau), less common on Mesa/Intel/AMD.

**WebKitGTK 2.46+ (Skia backend):** Replaced Cairo, improved MotionMark 4x, but WebGL issues persist. New DMA-BUF rendering path introduced its own driver-dependent failures.

**xterm.js v6 DOM renderer:** Significantly optimized in 5.x/6.x series (PR #4605). VS Code also falls back to DOM on WebGL context loss (#120393). Viable for AgentMux.

### Recommendation

**Option D: DOM renderer as default on Linux, with WebGL opt-in.**

Revised from the original "WebGL primary" recommendation based on research showing the bug is systemic to WebKitGTK, not a single fixable issue:

1. **Default to DOM renderer on Linux** — no addon needed, maintained as part of core, significantly optimized in v6
2. **Add a settings flag** (`terminal:renderer = "auto" | "webgl" | "dom"`) for users with known-good GPU configs to opt into WebGL
3. **When `auto` on Linux:** Use DOM. When `auto` on macOS/Windows: Use WebGL with DOM fallback on context loss
4. **Set `WEBKIT_DISABLE_DMABUF_RENDERER=1`** in AppRun (already done) as additional safety
5. **Log renderer choice** for diagnostics

The render-test approach (write control sequence, read back via SerializeAddon) is fragile — the bug is intermittent and GPU-dependent. Better to be safe by default and let power users opt in.

---

## Critical Issue: Custom FitAddon Private API

### The Problem

`fitaddon.ts` accesses `_terminal._core` and `_terminal._core._renderService` — private APIs that likely changed in v6 (new VS Code Emitter, scrollbar rework).

### Options

| Option | Pros | Cons |
|--------|------|------|
| **A. Update private API references** | Keep custom behavior | Fragile, breaks on every minor update |
| **B. Use upstream FitAddon + CSS for scrollbar** | No private API dependency | May lose precise noScrollbar behavior on macOS |
| **C. Contribute noScrollbar upstream** | Permanent fix | Slow — depends on xterm.js maintainers |

### Recommendation

**Option B.** The `noScrollbar` flag only hides the scrollbar width from dimension calculations. With v6's new VS Code-style scrollbar, this may be solvable with CSS `overflow: hidden` or the new scrollbar configuration options. Test first — the new scrollbar may already handle macOS correctly.

---

## Critical Issue: Scrollbar/Viewport Rework

### The Problem

v6 replaces the viewport and scrollbar with VS Code's base/platform scrollbar. AgentMux has:

- Custom z-index overrides in `theme.scss` (lines 72-81)
- Viewport overlay z-index (`--zindex-xterm-viewport-overlay: 5`)
- Scrollbar visibility toggling in `term.tsx` (lines 234-244)
- FitAddon scrollbar width compensation

### Impact

All CSS targeting `.xterm-viewport`, `.xterm-screen`, or scrollbar elements may need updating. The z-index values and class names may have changed.

### Recommendation

Audit all xterm CSS selectors in `theme.scss` and `xterm.css` against v6's DOM structure. Create a mapping of old → new selectors. The new scrollbar likely has different class names and z-index behavior.

---

## Migration Plan

### Phase 1: Assessment (1-2 days)

- [ ] Install v6.0.0 in a branch, run `npm install`
- [ ] Compile and catalog all TypeScript errors
- [ ] Document all changed/removed CSS class names
- [ ] Test WebGL on Linux (Ubuntu 24.04 + WebKitGTK latest)

### Phase 2: Core Migration (2-3 days)

- [ ] Update `@xterm/xterm` to `^6.0.0`
- [ ] Remove `@xterm/addon-canvas` dependency
- [ ] Update renderer logic in `termwrap.ts:loadRendererAddon()`:
  - macOS/Windows: WebGL with DOM fallback on context loss
  - Linux: DOM renderer (default), WebGL opt-in via settings
- [ ] Replace custom FitAddon with upstream `@xterm/addon-fit` + CSS scrollbar handling
- [ ] Update `overviewRulerWidth` → `overviewRuler.width` in search config
- [ ] Audit and update all CSS selectors for new scrollbar/viewport

### Phase 3: Addon Updates (1 day)

- [ ] Update all addons to v6-compatible versions (use beta channel if stable not yet released)
- [ ] Evaluate `@xterm/addon-progress` for agent task progress
- [ ] Evaluate `@xterm/addon-clipboard` to simplify copy-on-select
- [ ] Evaluate `onWriteParsed` API as potential replacement for RAF-batched writes

### Phase 4: Testing (2-3 days)

- [ ] **Linux:** WebKitGTK control sequence rendering (backspace, erase-in-line, cursor movement)
- [ ] **macOS:** WebGL context loss fallback (now to DOM instead of Canvas)
- [ ] **Windows:** DWM compositor "flash scroll" regression with new scrollbar
- [ ] **All platforms:** Search decorations, overview ruler, font sizing, theme switching
- [ ] **Performance:** Compare streaming throughput (multiple agents, fast output)
- [ ] **Long sessions:** Memory usage over 4+ hours (verify CoreBrowserService leak fix)
- [ ] **E2E:** Run full test suite

### Phase 5: New Features (optional, 1 day)

- [ ] Add `@xterm/addon-progress` for terminal tab progress indicators
- [ ] Explore synchronized output (DEC mode 2026) for cleaner multi-agent rendering
- [ ] Explore `@xterm/addon-image` for inline agent output images

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Linux rendering breaks | High | Critical | DOM default + WebGL opt-in |
| FitAddon private API breaks | High | High | Switch to upstream + CSS |
| CSS selectors break | Medium | Medium | Audit before migration |
| WebGL context loss path breaks | Low | High | Test on all platforms |
| Search decorations break | Low | Low | Minor config update |
| Performance regression | Low | Medium | Benchmark before/after |
| RAF-batched writes interact with new scroll | Medium | Medium | Test streaming output |

---

## Estimated Effort

| Phase | Days | Confidence |
|-------|------|-----------|
| Assessment | 1-2 | High |
| Core Migration | 2-3 | Medium |
| Addon Updates | 1 | High |
| Testing | 2-3 | Medium |
| New Features | 1 | High |
| **Total** | **7-10 days** | |

---

## Decision Required

**Should we proceed with the v6 upgrade?**

**For:**
- 30% smaller bundle
- Synchronized output eliminates screen tearing (key for multi-agent UX)
- Memory leak fix matters for long sessions
- Canvas addon is dead — staying on v5 means no future security patches
- ESM + tree-shaking improves Vite build

**Against:**
- Linux Canvas fallback is critical and well-tested — removing it is risky
- Private API in FitAddon needs rework
- Scrollbar rework touches multiple files
- 7-10 days of work with regression risk

**Recommendation:** **Proceed.** The Canvas addon being deprecated is a ticking time bomb. Better to migrate now while v6.0.0 is fresh and we can reference the VS Code migration (they're the primary consumers of xterm.js). The synchronized output feature alone is worth it for AgentMux's multi-agent use case.
