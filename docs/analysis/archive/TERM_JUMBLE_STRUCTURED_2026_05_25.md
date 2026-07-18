> **Archived 2026-07-17:** Resolved. The "thaw" fix proposed in §7a shipped the same
> day as PR #1043 (PSReadLine cursor-desync thaw, still present in `termwrap.ts` as
> `thawTimeoutId`/`thawRafId`), closing issue #1042. Kept for historical reference
> only — do not treat as an open investigation.

# Term-jumble: structured analysis after 8 failed fix attempts

**Date:** 2026-05-25
**Author:** AgentA (Claude Opus 4.7)
**Tracking issue:** [#1042](https://github.com/agentmuxai/agentmux/issues/1042)
**Related history doc:** [`docs/terminal-jumbled-startup-investigation.md`](../terminal-jumbled-startup-investigation.md) (the original timeline + the PR-#1040 follow-up section)
**Methodology note:** [feedback_3strikes_term_jumble.md](../../../.claude/projects/C--Systems/memory/feedback_3strikes_term_jumble.md) (agent memory — internal)

---

## TL;DR

Eight fix attempts have failed to resolve the "rapid pane creation → last-opened terminal(s) render jumbled" bug. Three of those shipped (PRs #1030, #1040, #1041-closed). After reverting to main and doing a research+analysis pass, the most credible remaining hypothesis is **H1: Solid reactive cascade leaves transient style values for the new tile during the moment its term widget's `onMount` runs**, so xterm's renderer creates DOM children sized against the wrong initial container — and those inner DOM widths don't update when the outer container settles, because xterm only invalidates them on an explicit `resize()` call.

The diagnostics we had this session captured outer container size + xterm's *reported* cols/cellW — those were correct end-to-end. They did NOT capture **the actual xterm-rendered DOM dimensions** (the `.xterm-screen` / `.xterm-rows` children). The next attempt should add those diagnostics first, BEFORE writing any fix.

---

## 1. Symptom

When the user opens **N terminals in rapid succession** (e.g. split, split, split, split, split), the **last-opened terminal(s) render with cursor/glyph mis-alignment**:

- Buffer content is correct (verified: copy-paste returns clean text)
- Cursor visually offset from where the shell tracks it; pressing Enter "jumps" the cursor
- Self-heals on **manual pane resize**, **zoom** (Ctrl+wheel), or **Vite HMR reload**
- All three fixes have in common: they trigger an `xterm.resize()` call against a *settled* layout
- The FIRST opened terminal is usually fine; jumble concentrates on later ones, especially the very last

Severity: not a functional break (input works, output works, text is captured correctly) but is visibly broken to the user and resets to OK on any resize.

---

## 2. What is RULED OUT (do not re-chase)

Across 8 attempts on this bug:

| # | Hypothesis | How disproved |
|---|---|---|
| 1 | Font not loaded at `terminal.open()` (PR #1030 — `fonts.ready`) | Initial fix, didn't survive multi-terminal cold-cache repro |
| 2 | `fonts.ready` is vacuous — need `fonts.load(spec)` after open (PR #1040) | Merged; bug still reproduces |
| 3 | `fonts.load(spec)` must run BEFORE `terminal.open()` because xterm caches metrics (PR #1041 — closed) | Tested locally; bug still reproduces |
| 4 | xterm caches cell-width at open() and never invalidates | Disproved by diagnostic: `cssCellW` measurably changes 7.225 → 7.214 between `post-open` and `post-customfit`, so cache IS invalidated |
| 5 | `handleResize_debounced` 50ms window swallows the corrective fit | Removed debounce; bug persists |
| 6 | WebGL renderer caches glyphs in atlas, atlas not regenerated | `term:disablewebgl: true` + DOM-only path; bug persists |
| 7 | Bug is in xterm.js's renderer implementation | Replaced with `ghostty-web` (Coder's WASM Ghostty parser, xterm-API-compatible); **bug got worse** (additional special-char artifacts) — so bug class is upstream of the renderer |
| 8 | `windowsPty: { backend: "conpty" }` + `reflowCursorLine: true` for ConPTY reflow | Made it worse |
| 9 | `_charSizeService.measure()` + `terminal.refresh()` post-init | No effect |
| 10 | Stable-wait (2 consecutive rAF ticks at same container size) before `open()` | No effect |
| 11 | Synthetic resize cycle (`resize(c-1, r); resize(c, r)`) post-init mimicking HMR | No effect |
| 12 | `customFit` BEFORE `loadInitialTerminalData` (so restore captures correct curTermSize) | Cols changed mid-restore on some terminals but did not fix the jumble |

The renderer swap (entry 7) is the most informative: it proves the bug is **not in the terminal widget implementation** — both xterm.js and ghostty-web exhibit it.

---

## 3. What we KNOW

1. **The bug fires concentrated on the last-opened terminal in a burst.** Specifically reproducible by opening 5 panes in quick succession; 4 are clean, last 1 is jumbled (sometimes 2-3 are).
2. **xterm's reported numerical state is correct throughout init.** Across `init-start` → `post-customfit` → `post-resync` → `raf-refit`, the diagnostic chain shows consistent, plausible `cols`/`rows`/`cssCellW`/`connectElemRect` for every terminal — including the jumbled ones.
3. **The bug is not renderer-specific.** Demonstrated by swapping xterm.js → ghostty-web.
4. **Three different mechanisms fix the jumble after it appears:** manual resize, zoom, HMR re-mount. All three trigger a fresh `xterm.resize()` (or component re-create) against a settled layout.
5. **AgentMux's layout system is transform-based, not flex grow/shrink.** Each tile-leaf gets explicit `width`/`height`/`transform` from `addlProps()` (see `frontend/layout/lib/TileLayout.win32.tsx:527-534`). The CSS `transition-duration` is `--animation-time-s` which defaults to `0s` — so there is no actual CSS animation in the layout. Sizes change instantly.

---

## 4. SolidJS + reactive layout — research findings

Sources consulted:
- [SolidJS onMount lifecycle](https://docs.solidjs.com/reference/lifecycle/on-mount)
- [SolidJS 2.0 deterministic batching + `flush()`](https://www.infoq.com/news/2026/05/solidjs-2-async/)
- [SolidJS issue #1224 — undefined style props are not written to DOM](https://github.com/solidjs/solid/issues/1224)
- [ResizeObserver loop with flex items](https://github.com/caplin/FlexLayout/issues/406)
- [xterm.js issues #103, #171, #1701, #3962 — resize cursor/prompt drift](https://github.com/xtermjs/xterm.js/issues/103)

### Findings

**(a)** Solid 1.x batches reactive updates via microtasks. Signal setters do NOT immediately propagate; downstream effects run on the next microtask. So in a parent-resizes-when-child-added scenario, the parent's `addlProps()` recomputation may flush on a different microtask than the new child's `onMount`.

**(b)** Per Solid issue #1224: **"style prop object properties that evaluate to undefined do not end up in the DOM styling attribute"**. This is critical for our layout because `tileTransform()` returns `addlProps()?.transform`, and if `addlProps()` returns `null` initially (before the layout state cascade settles), the tile-node has NO width/height in the DOM at the moment of first render.

**(c)** Solid's `onMount` runs after the component's initial render, but "initial render" can complete with transient prop values that resolve to `undefined` if the reactive memos they depend on haven't yet flushed. The component IS mounted; its DOM IS in the document tree; its style attrs may be missing.

**(d)** `ResizeObserver` does not fire for elements with zero size in some browsers, and for elements that transition from 0 → real-size in the same frame, may fire only once for the final size — meaning we don't get an opportunity to "catch" the transient state.

---

## 5. Refined hypothesis (H1)

**The new tile-node mounts with transient/missing dimensions, and xterm's `terminal.open()` captures that transient state into the inner xterm DOM (`.xterm-screen`, `.xterm-rows > div`, the cursor layer). When the tile's `addlProps()` later flushes real dimensions, the OUTER container resizes — but xterm's INNER DOM elements stay at their initial widths (rendered cell widths). xterm only fully re-renders the inner DOM on an explicit `resize()` call — which is exactly what manual resize / zoom / HMR all trigger.**

This explains every observation:
- **Numerical state correct** because `_renderService.dimensions.css.cell.width` reflects the current font metrics, which update independently of inner DOM widths.
- **Outer container measures correct** because the tile-node's style DID eventually flush.
- **Inner xterm DOM is wrong** because it was created against zero/transient state and not re-emitted.
- **Fixed by resize** because xterm's `resize()` regenerates the inner DOM cell elements.
- **Renderer-agnostic** because both xterm.js and ghostty-web do this "render once, hold" pattern for their inner DOM/canvas.
- **Concentrated on last-opened** because the last-opened tile mounts during the active layout cascade; earlier tiles got corrective ResizeObserver fires when later splits resized them, which forced an `xterm.resize()` and rebuilt their inner DOM. The last tile gets no such corrective resize.

### Counter-evidence

- The "stable-wait" attempt (wait for 2 rAF ticks with same container size before `terminal.open()`) should have fixed H1, but didn't. So either the container DOES settle within 2 frames but xterm's inner-DOM-creation has its own delay, OR H1 is incomplete and the bug has a second component.
- The "fake resize cycle" (`resize(c-1, r); resize(c, r)` at end of init) should also have fixed H1 by forcing xterm to recreate inner DOM, but didn't. This is harder to explain — possibly the two consecutive `resize()` calls coalesce internally to a no-op when the final size equals the previous size.

### Status

H1 is the most plausible remaining hypothesis but is not fully proven. Next session must **measure inner xterm DOM dimensions** before declaring H1 confirmed.

---

## 6. Diagnostic plan for the next attempt

The diagnostics we had this session were exhaustive for the OUTER container + xterm's REPORTED metrics but had no visibility into:

- The actual xterm-created inner DOM dimensions (`.xterm-screen`, `.xterm-rows > div:first-child`)
- Whether the tile-node's style attribute was set/unset/transient at the moment of term mount
- Whether PTY bytes arrived during the pre-fit window

### Diagnostic placements to ADD

| Location | Tag | Captures | Distinguishes |
|---|---|---|---|
| `TileLayout.win32.tsx` — leaf component mount | `[term-jumble:tile-mount]` | `addlProps()` snapshot, `tileTransform()` value, element's current `style.cssText` | Whether new tile mounts with `undefined` style attributes (H1 root cause) |
| `term.tsx` — onMount, before TermWrap create | `[term-jumble:pre-open-rect]` | `connectElem.getBoundingClientRect()` full rect (x/y/w/h) | Whether outer rect is at final position before xterm gets it |
| `term.tsx` — multiple times in init flow | `[term-jumble:rects-snapshot]` | Outer rect AND `terminal.element.querySelector('.xterm-screen').getBoundingClientRect()` AND first `.xterm-rows > div`'s computed width | H1 confirmation: does inner DOM drift from outer? |
| `termwrap.ts` `handleNewFileSubjectData` | `[term-jumble:pty-bytes-pre-fit]` | Byte length + first 20 bytes as hex, gated on `!this.hasResized` | H2: are PTY bytes arriving during the unfit window? |
| `termwrap.ts` `customFit()` | `[term-jumble:customfit-internals]` | `_renderService._dimensions.css.cell.width`, `actualCellWidth`, the `_renderer._charsRect` if accessible | Detect drift between "logical" and "actual" cell dims |

### Visual capture (best diagnostic if possible)

After the user reports jumble on a specific block, capture `terminal.element.outerHTML` and the canvas/DOM screenshot. Compare the HTML/pixel layout between a jumbled terminal and a clean one. The pixel-level difference will localize the bug definitively.

This can be wired via a console command: `window.captureTermSnapshot('<blockId>')` that the user runs in DevTools when they see the jumble.

---

## 7. Once H1 is confirmed — candidate fixes

In rough order of cleanliness:

1. **Block init() until `addlProps()` is non-null.** Add a Solid effect in `term.tsx` that waits for the parent layout's `addlProps` signal to be defined before calling `TermWrap.init()`. Most surgical; matches the actual cause.

2. **Force xterm's inner DOM re-create after a layout-settle frame.** After init, schedule a `terminal.resize(cols-1, rows); terminal.resize(cols, rows)` 100ms post-init **regardless of dim changes**. The previous attempt at this didn't work because the two `resize()` calls likely coalesce; need to confirm by reading xterm.js's resize handling code. If coalescing is the issue, do the resizes across two rAF ticks.

3. **Dispose + recreate xterm 100ms post-init.** Brute force. Visible flicker. Last resort.

4. **Migrate to Solid 2.0 + `flush()`.** Use `flush()` before `terminal.open()` to force the entire reactive graph to settle. Big version bump; risky.

---

## 7a. Update — H1 REFUTED, H6 confirmed (2026-05-25 evening)

A diagnostic cycle in the same day captured `[term-jumble:rects ...]` snapshots at every init phase AND `[term-jumble:onRender]` chains capturing every xterm paint with its cols/rows state. User reproduced "open 9 terminals, 5 broken" — clean test for the hypothesis tree.

### H1 (outer/inner DOM drift) — REFUTED

All 9 terminals showed identical geometry at `post-200ms`:
- outer: 112×462
- xtermElem: 101×462
- xtermScreen: 101×462
- firstRow: 101×14
- cols: 14

No drift between outer container and inner xterm DOM elements, on any of the 9 terminals — broken or clean. The bug is NOT in DOM layout.

### H6 (resize-count correlates with broken state) — CONFIRMED

The `[term-jumble:onRender]` chain showed exactly **5 terminals had only one resize transition (80→14)**, and **4 terminals went through multiple transitions** as later splits shrank them:

| Block | onRender cols progression | Status (per user) |
|---|---|---|
| a1211aee | 80 → 84 → 40 → 26 → 18 | ✅ clean (5 transitions) |
| 46aeb56a | 80 → 40 → 26 → 18 → 14 | ✅ clean (5 transitions) |
| 02e4f936 | 80 → 26 → 18 → 19 → 14 | ✅ clean (5 transitions) |
| 4def8d2d | 80 → 18 → 19 → 14 | ✅ clean (4 transitions) |
| 47adb1e0 | **80 → 14** | 🔴 broken (1 transition) |
| 0a1d1974 | **80 → 14** | 🔴 broken (1 transition) |
| d9c11d22 | **80 → 14** | 🔴 broken (1 transition) |
| d21a9d7e | **80 → 14** | 🔴 broken (1 transition) |
| 2876dde9 | **80 → 14** | 🔴 broken (1 transition) |

User reported "9 terminals, 5 broken". Diagnostic confirms exactly 5 terminals with one-transition history. **Match.**

### Mechanism

xterm.js initializes at default 80×24. `customFit` shortly after `terminal.open()` resizes to the actual pane dims and sends a SIGWINCH via `sendTermSize`. Backend spawns pwsh at the customFit'd size in `resyncController("init")`. pwsh + PSReadLine emit their prompt.

For **broken terminals**: there is exactly one xterm-side resize (80→14 in this layout). PSReadLine likely emits its prompt against the inherited cols=80 environment (or otherwise gets confused by the inherited default), then the SIGWINCH from sendTermSize lands as cols=14. The prompt is in the buffer at 80-col wrap points; xterm now displays at 14-col grid; PSReadLine's tracked cursor diverges from xterm's actual cursor. Pressing Enter sends a `\r` but PSReadLine emits its next prompt at its (wrong) tracked cursor → user sees the prompt jump around.

For **clean terminals**: subsequent pane splits shrink the existing pane several times. Each shrink triggers `handleResize` → `customFit` → `sendTermSize` → SIGWINCH. PSReadLine sees multiple SIGWINCH and re-syncs its cursor tracking on each one. By the final state, PSReadLine is aligned with xterm. The cascade of resizes "thaws" the bug.

This is consistent with:
- The user's exact wording "if I press enter, the cursor will jump around" — PSReadLine cursor desync
- "Fixes on manual resize / zoom / HMR" — all three trigger another SIGWINCH/redraw that re-syncs PSReadLine
- "Last-opened terminals are most often broken" — they get no subsequent resize cascade

### Refuted by-products

- **H2 (PTY bytes pre-fit)**: `[term-jumble:pty-bytes-pre-fit]` fired 0 times. No bytes arrive before customFit.
- **H4 (cross-block stream contamination)**: not investigated further; H6 explains the symptoms fully.

### Targeted fix (next attempt)

**Force a "thaw" resize cycle at end of init** — replicate what naturally happens to clean terminals. Specifically, after `resyncController("init")`:

```ts
// After PSReadLine has emitted its first prompt, force a resize cycle
// to nudge it into resync. xterm.js coalesces back-to-back same-frame
// resizes — splitting across rAF ticks ensures both fire.
await new Promise((r) => requestAnimationFrame(() => r(undefined)));
this.terminal.resize(this.terminal.cols + 1, this.terminal.rows);
this.sendTermSize();
await new Promise((r) => requestAnimationFrame(() => r(undefined)));
this.terminal.resize(this.terminal.cols - 1, this.terminal.rows);
this.sendTermSize();
```

This produces TWO SIGWINCH events post-shell-startup, mimicking what clean terminals naturally receive from sibling-split-induced resizes. Each one triggers PSReadLine to re-sync.

**Risk:** brief visible flicker (one cell width change). Acceptable since manual resize causes the same thing.

### Cleaner alternative

A more principled fix: don't spawn the shell until the layout is settled. Detect "stable for N ms" before calling `resyncController("init")`. New panes opening during this window reset the timer. This eliminates the entire SIGWINCH-during-shell-startup race rather than papering over it. Trade-off: shell startup is deferred by 100-500ms when rapidly creating panes. Likely acceptable.

We will try the thaw fix first (simpler, smaller surface area), and fall back to defer-spawn if thaw doesn't work.

## 8. Methodology lesson

I burned ~5h in one session pushing 8 patches without restructuring. The right action at attempt 4 was to write THIS document, not push another patch. CLAUDE.md has the "3-strike rule" — STOP, present alternatives, let user pick. Followed it on the 8th attempt, not the 4th.

Key signals that you're in the trap:
- User's eye-test repeatedly says "same / worse"
- Diagnostics show stable values where the bug should manifest
- Each fix targets a different layer
- Swapping the suspect component doesn't help

Save the agent memory: [feedback_3strikes_term_jumble.md](../../../.claude/projects/C--Systems/memory/feedback_3strikes_term_jumble.md).
