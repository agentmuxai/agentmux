# Terminal glyphs render as black boxes, fixed by selecting text — investigation

**Date:** 2026-07-17
**Severity:** P2 (cosmetic but frequent — "every couple characters"; no data loss, self-heals on selection)
**Symptom:** Individual characters in a terminal pane intermittently render as solid black
boxes instead of their glyph. Happens scattered through normal output, roughly every couple
of characters. Selecting/highlighting the affected text makes the correct glyphs reappear.

This is a **different bug** from
[`terminal-black-screen-race-condition.md`](./terminal-black-screen-race-condition.md) in
this same directory — that one is a whole-pane black screen caused by a subscribe-before-data
race in `termwrap.ts`/`shell.rs` and never self-heals without reopening the pane. The bug
here is scattered per-character corruption that **does** self-heal via selection. Different
mechanism, different code path. Do not conflate the two when triaging.

---

## Executive summary

AgentMux's terminal (`frontend/app/view/term/`) is xterm.js v6 rendered through
`@xterm/addon-webgl` (WebGL2) by default, with a DOM-renderer fallback. The WebGL renderer
keeps a CPU-side "shadow model" of what it believes is currently painted per cell
(character code + fg/bg/ext) purely as a diffing cache to skip redundant GPU draw calls. It
updates that shadow model **unconditionally**, in the same pass where it decides to issue the
draw call — it does not verify the draw actually landed on screen. If any individual glyph
draw is ever dropped or silently fails (GPU driver hiccup, texture-atlas page churn,
context-state race — a well-documented class of bug in GPU-accelerated terminal/text
renderers generally, see evidence below), the shadow model still records "drawn correctly."
Every subsequent frame compares incoming cell data against that shadow model, sees a match,
and skips redrawing — so the black box (empty/cleared pixels where a glyph should be) is
stuck **permanently** until something changes that cell's recorded fg/bg away from the cached
value.

Selecting text is exactly such an event: it changes the resolved background/foreground of
every selected cell (selection highlight color), which breaks the "nothing changed" cache
check and forces those cells to actually redraw — this time succeeding, which is why the
black box disappears. Deselecting flips the color back and forces a second real redraw,
which is why the fix persists after releasing the selection instead of only working transiently.

This also isn't purely an upstream xterm.js concern — AgentMux's own settings wiring
undermines its own attempted mitigation. See §4.

---

## 1. AgentMux's terminal renderer stack

From `package.json`:

```
"@xterm/xterm": "^6.0.0",
"@xterm/addon-webgl": "^0.19.0",
"@xterm/addon-fit": "^0.11.0",
```

`frontend/app/view/term/termwrap.ts:641-683` (`loadRendererAddon`) chooses between the WebGL
addon and xterm's built-in DOM renderer:

```ts
private loadRendererAddon(useWebGl: boolean) {
    // WebKitGTK's WebGL2 implementation has systemic rendering issues —
    // texture atlas doesn't redraw after control sequences (backspace, erase-in-line).
    // This is a WebKitGTK bug, not xterm.js (Tauri #6559, WebKit Bug 228268).
    // Default to DOM renderer on Linux; WebGL opt-in via term:disablewebgl=false.
    if (PLATFORM === PlatformLinux && !useWebGl) {
        ...
        return; // DOM renderer is the default when no renderer addon is loaded
    }
    if (WebGLSupported && useWebGl) {
        const webglAddon = new WebglAddon();
        ...
        this.terminal.loadAddon(webglAddon);
        setTermRendererAtom("webgl");
        ...
    }
    ...
}
```

The comment states the *intent*: default to the DOM renderer on Linux because of known
WebGL2 rendering defects, and require an explicit opt-in (`term:disablewebgl=false`) to use
WebGL there.

## 2. The default-value bug that defeats that intent

`useWebGl` is computed in `frontend/app/view/term/term.tsx:188`:

```ts
useWebGl: !ts?.["term:disablewebgl"],
```

`term:disablewebgl` has no configured default (`schema/settings.json:35-37` declares only
`"type": "boolean"`, no `"default"`), and `settings-template.jsonc:16` ships it
**commented out**. So for any user who has not explicitly set it, `ts?.["term:disablewebgl"]`
is `undefined`, and `!undefined === true` — meaning **`useWebGl` is `true` by default on every
platform, including Linux.**

Walk that back into `loadRendererAddon`: the guard is
`if (PLATFORM === PlatformLinux && !useWebGl)`. With the real-world default `useWebGl = true`,
`!useWebGl` is `false`, so the condition is `false` and the Linux DOM-renderer branch is
**never taken** by default. WebGL loads on Linux anyway, unconditionally undermining the
workaround the comment describes. The code that was supposed to route Linux away from a
renderer its own authors flagged as having "systemic rendering issues" is dead under default
settings — a user would have to already know to set `term:disablewebgl: true` to get the
protection the comment claims is the default.

This is directly relevant here: if the report originated on Linux, the user is very likely
running the WebGL renderer despite the codebase's own stated intention to avoid it there.

## 3. Why WebGL specifically produces "black box, fixed by selection"

Source checked directly (`@xterm/addon-webgl@0.19.0`, matching the version pinned in
`package.json`, from a local `node_modules` install):

`WebglRenderer.ts` → `_updateModel()` (the per-frame cell-diffing loop):

```ts
// Nothing has changed, no updates needed
if (this._model.cells[i] === code &&
    this._model.cells[i + RENDER_MODEL_BG_OFFSET] === this._cellColorResolver.result.bg &&
    this._model.cells[i + RENDER_MODEL_FG_OFFSET] === this._cellColorResolver.result.fg &&
    this._model.cells[i + RENDER_MODEL_EXT_OFFSET] === this._cellColorResolver.result.ext) {
  continue;   // <-- skip redraw, assumes the cell is already correctly on screen
}
modelUpdated = true;
...
// Cache the results in the model
this._model.cells[i] = code;
this._model.cells[i + RENDER_MODEL_BG_OFFSET] = this._cellColorResolver.result.bg;
this._model.cells[i + RENDER_MODEL_FG_OFFSET] = this._cellColorResolver.result.fg;
this._model.cells[i + RENDER_MODEL_EXT_OFFSET] = this._cellColorResolver.result.ext;
width = cell.getWidth();
this._glyphRenderer.value!.updateCell(x, y, code, ..., chars, width, lastBg);
```

The shadow model (`this._model.cells[...]`) is written **before/regardless of** whatever
`GlyphRenderer.updateCell()` → `TextureAtlas.getRasterizedGlyph()` actually manages to paint
into the WebGL texture atlas and instance buffer. There is no feedback path where a failed or
dropped atlas rasterization/texture-upload un-does that cache write. If that single draw call
is lost — a plausible outcome under GPU/driver churn, described in the same file's own
`beginFrame()`/atlas-page-merge handling as a known trigger for needing a full-model clear
(`WebglRenderer.ts:344-352`, referencing xterm.js issue #4480, see below) — the cell is now
permanently "believed correct" and will never be redrawn by ordinary terminal output, because
nothing about that cell's code/fg/bg/ext changes again until something else overwrites that
screen position.

`handleSelectionChanged()` (`WebglRenderer.ts:228`) updates `_model.selection`, and
`CellColorResolver.resolve()` blends the selection highlight into the resolved bg/fg for any
selected cell. That means the *cached* bg/fg for that cell (recorded from the failed draw) no
longer matches the freshly resolved bg/fg (now including the selection tint) — the "nothing
changed" check fails, the cache is bypassed, and the glyph is actually redrawn. This time it
succeeds (whatever transient condition dropped the original draw is gone), so the box
disappears. Releasing the selection reverts bg/fg to the un-selected values, which again
differs from the (now selection-tinted) cached values, forcing one more real redraw — with
the final correct colors. That's the full mechanism for why selecting *and then deselecting*
leaves the glyph fixed rather than just fixed-while-selected.

This also explains "every couple characters": it doesn't require a total renderer failure,
just an occasional single dropped draw call per some number of characters — consistent with
a rare GPU/driver-timing race rather than a systemic renderer break.

## 4. External evidence this is a known class of bug, not something novel to AgentMux

**In xterm.js's own WebGL addon:**
- [xtermjs/xterm.js#4480](https://github.com/xtermjs/xterm.js/issues/4480) — "blacked out
  content" appearing during heavy WebGL-renderer updates, worsening with higher update
  frequency; fixed by PR #4533 (partly via the atlas-page-merge full-model-clear path visible
  in `WebglRenderer.ts:345`, `#4480` cited directly in that comment).
- [xtermjs/xterm.js#3548](https://github.com/xtermjs/xterm.js/issues/3548) — canvas renderer's
  CharAtlas not refreshing/invalidating correctly after palette changes; same family of
  "cache says drawn, screen disagrees" bug, different trigger.
- [xtermjs/xterm.js#4065](https://github.com/xtermjs/xterm.js/issues/4065) — canvas/WebGL
  texture atlas duplication/consolidation, underlining that atlas-page management is a
  recurring correctness hazard in this renderer family.

**In other GPU-accelerated terminal/text renderers (same architectural pattern, independent
implementations — supports this being a general class of bug, not an xterm.js-specific
oddity):**
- [microsoft/terminal#8286](https://github.com/microsoft/terminal/issues/8286) — Windows
  Terminal's AtlasEngine shows graphical corruption on focus loss/regain (implicated: Intel
  iGPU); explicitly "forcing a repaint by dragging over it... repairs the view" — the same
  "forced redraw fixes stale pixels" signature as selection does here.
- [Arch Linux forum #299620](https://bbs.archlinux.org/viewtopic.php?id=299620) — GNOME
  `kgx`/`gnome-text-editor` black-box glitches traced to GTK4's Vulkan (`GSK_RENDERER`)
  renderer misbehaving on certain AMD driver combinations; fixed by forcing `GSK_RENDERER=gl`
  or swapping the Vulkan driver. Different toolkit, same "GPU renderer + specific driver =
  black box" pattern.
- [WebKit Bug 228268](https://bugs.webkit.org/show_bug.cgi?id=228268) — "[GTK4] Rendering on
  Nvidia is terribly broken, almost a blank screen" — cited by name in AgentMux's own
  `termwrap.ts:644` comment as one of the two upstream bugs motivating the (currently
  non-functional, see §2) Linux DOM-renderer default.
- Historical precedent at the font-glyph-cache layer specifically:
  [freedesktop.org bug 21790](https://bugs.freedesktop.org/show_bug.cgi?id=21790) —
  `xf86-video-intel` "pixmap corruption in the font glyph cache," i.e. stale/corrupted glyph
  bitmaps surviving in a GPU-side cache past the point where they should have been
  invalidated — conceptually the same failure mode as §3, one layer lower in the stack.

**Already-known-but-distinct AgentMux-internal precedent:**
`docs/analysis/archive/TERM_JUMBLE_STRUCTURED_2026_05_25.md` documents eight prior fix attempts at a
*different* terminal rendering bug (cursor/glyph misalignment after rapid pane creation, not
scattered black boxes). Its hypothesis list explicitly ruled out disabling WebGL as a fix for
*that* bug (entry #6: "`term:disablewebgl: true` + DOM-only path; bug persists") — that result
does not transfer to the bug in this report, since the two symptoms have different root
causes (that investigation converged on a PSReadLine/SIGWINCH cursor-resync race, unrelated
to GPU texture atlases). Worth cross-linking so a future reader doesn't assume "WebGL was
already ruled out for terminal rendering bugs in general" — it was ruled out for one specific
symptom, not this one.

## 5. Suggested validation (before committing to a fix)

1. Check the status-bar GPU/renderer indicator (driven by `setTermRendererAtom`, referenced
   throughout `termwrap.ts`) at the moment a black box appears, to confirm the pane is
   actually on `"webgl"` and not `"dom"`.
2. Force `term:disablewebgl: true` in settings and try to reproduce. If the black boxes stop
   appearing entirely on the DOM renderer, that strongly confirms the WebGL atlas/shadow-model
   desync as root cause rather than e.g. a font-substitution issue.
3. Collect GPU/driver info from the affected machine (on Linux: `glxinfo | grep -i "opengl
   renderer"`, Mesa version, and whether it's Nvidia proprietary, Mesa/Intel, or Mesa/AMD —
   the external evidence above skews toward specific driver/GPU combinations rather than
   "WebGL in general").
4. If reproducible on demand, try capturing whether the black box coincides with high-volume
   output (matches the "worse with heavier updates" pattern from xterm.js#4480) versus
   appearing during idle/low-rate output.

## 6. Candidate fixes

**Immediate mitigation (no code change):** set `term:disablewebgl: true` in
`settings.json` to force the DOM renderer, trading some scroll-heavy-output performance for
eliminating this whole bug class.

**Fix the dead default (independent of whether WebGL is the confirmed cause):**
`termwrap.ts:646`'s Linux DOM-default branch is unreachable under the actual default value of
`useWebGl`. Either give `term:disablewebgl` an explicit schema default of `true` on Linux, or
change the condition so the intended platform default doesn't depend on an unset user
setting evaluating the way the author assumed. As written, every Linux user is on WebGL
unless they've manually discovered and set `term:disablewebgl`, which is the opposite of
what the surrounding comment says should happen.

**Defensive self-heal (treats the symptom, doesn't require the exact GPU race to be
pinned down):** periodically force a full-model invalidation/redraw independent of user
interaction — e.g. call the equivalent of `_clearModel(true)` + full-viewport `_updateModel`
(the same path `beginFrame()`'s atlas-page-merge case already takes at
`WebglRenderer.ts:344-352`) on an idle timer, or after N kilobytes of PTY output, similar in
spirit to the "thaw resize cycle" idea already used elsewhere in this codebase for the
jumbled-glyph bug. This would make the terminal self-correct without requiring the user to
know "select the garbled text" as a manual workaround.

**Upstream:** confirm the installed `@xterm/addon-webgl` version doesn't already have a newer
patch addressing dropped-draw/atlas-desync edge cases beyond #4480/#4533, and if the bug
reproduces cleanly on the DOM-vs-WebGL toggle test in §5, consider filing a minimal repro
upstream at xtermjs/xterm.js — the existing issues found in this investigation are closely
related but not an exact symptom match ("scattered single-character black boxes fixed by
selection" specifically), so this may be a distinct, not-yet-filed edge case in the same
subsystem.

---

## Open questions for whoever picks this up

- Confirm the reporting user's OS/GPU/driver and which renderer (`webgl` vs `dom`) their pane
  was actually using at repro time — everything above is a strong architectural hypothesis
  from reading the renderer's own source, not yet a confirmed-on-hardware root cause.
- If DOM renderer also reproduces the black boxes (falsifying the WebGL-atlas hypothesis
  above), redirect investigation toward font rendering / glyph substitution instead (the
  CoderLuii/HolyClaude#40 report of "characters replaced with black squares" turned out to be
  a missing-monospace-font-in-container issue, unrelated to any renderer's atlas — worth
  ruling out early since it's a much simpler fix if applicable).
