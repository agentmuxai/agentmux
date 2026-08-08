# Agent Busy Bar (Marching Ants) Refinement

**Date:** 2026-06-22
**Owner:** AgentC
**Status:** Proposed (design) (implemented — see note below)

> **2026-08-07 audit note:** Implemented — the marching-ants bar exists in
> `_control-bar.scss`. Status field was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
**Scope:** `frontend/app/view/agent/styles/_control-bar.scss`
**Builds on:** [`SPEC_AGENT_BUSY_ANIMATION_2026_06_21.md`](./SPEC_AGENT_BUSY_ANIMATION_2026_06_21.md) and PR #1694
(`feat(agent-pane): replace gradient sweep with marching-ants progress bar`)

---

## 1. Context

PR #1694 replaced the aurora gradient sweep with a diagonal repeating-stripe
("marching ants") busy indicator. The current implementation (top of
`_control-bar.scss`):

```scss
.agent-pane-progress-bar {
    position: absolute; top: 0; left: 0; right: 0;
    height: 3px;
    z-index: var(--z-pane-overlay, 4);
    pointer-events: none;
    opacity: 0;
    overflow: hidden;
    transition: opacity 200ms ease;

    &::before {
        content: "";
        position: absolute; top: 0; bottom: 0; left: -12px; right: -12px;
        background: var(--accent-color);                       // < Chromium 111 fallback
        background: repeating-linear-gradient(
            -45deg,
            var(--accent-color)                                       0px,
            var(--accent-color)                                       4px,
            color-mix(in srgb, var(--accent-color) 25%, transparent) 4px,
            color-mix(in srgb, var(--accent-color) 25%, transparent) 8px
        );
        will-change: transform;
    }

    &--active { opacity: 1; &::before { animation: agent-ant-march 0.5s linear infinite; } }
    &--active#{&}--stopping { opacity: 0.55; }
}

@keyframes agent-ant-march {
    from { transform: translateX(0); }
    to   { transform: translateX(11.314px); }   // one stripe period (8px * sqrt2)
}

@media (prefers-reduced-motion: reduce) {
    .agent-pane-progress-bar--active::before { animation: none; }
}
```

The bar is rendered in `agent-view.tsx:1008` as a `position:absolute; top:0`
overlay (z-index 4) sitting above the agent content; `.agent-view` itself paints
`background: var(--main-bg-color)` (`agent-view.scss:39`).

Two defects, reported while running v0.48.1 on Windows 11.

---

## 2. Problem 1: bar is "stuck" (frozen) on Windows 11, animates on Windows 10

### Root cause (confirmed, not hypothesized)

The `@media (prefers-reduced-motion: reduce)` block sets `animation: none` and
provides **no replacement motion**. When the OS reports reduced motion, the bar
still shows at full opacity (`--active` sets `opacity: 1`) but the stripe pattern
is frozen mid-phase. To the user that reads as a stuck/broken progress bar, not
as an intentional "motion suppressed" state.

CEF/Chromium maps `prefers-reduced-motion: reduce` to the Win32
`SPI_GETCLIENTAREAANIMATION` system parameter, which is the
**Settings > Accessibility > Visual effects > Animation effects** toggle.

Verified on claudius (Windows 11) via `SystemParametersInfo(0x1042)`:

```
ClientAreaAnimation (animations ON) = False   => prefers-reduced-motion: REDUCE
```

So this box reports reduced motion and the bar freezes. A Windows 10 machine
with Animation effects ON reports `no-preference`, so `agent-ant-march` plays.
The behavior is therefore **per-machine OS setting**, not an OS-version code
path: any Windows 11 (or 10) box with Animation effects off, or in a power mode
that auto-disables animations, will freeze.

This is a **regression introduced by #1694**: the prior aurora implementation
gave reduced motion a visible static fallback (`transform: translateX(0)
rotate(0deg); opacity: 0.7`). #1694 deleted that, leaving only `animation: none`.

### Requirement

Under reduced motion the bar must still clearly communicate "agent is working"
without positional motion, and must not look frozen mid-pattern.

### Proposed fix (revised)

> **Update (2026-06-22):** an earlier revision swapped the march for an opacity
> "breathe" on a solid bar under reduced motion. On a reduced-motion machine
> that reads as a soft glow / "aurora", not the marching ants the indicator is
> supposed to show (and it is a different effect from what motion-enabled
> machines see). Product intent is to **always show the ants**. The breathe is
> dropped.

Keep the marching ants under reduced motion - this is a small (3px), looping,
**essential** "agent is working" indicator, not large or parallax motion. Rather
than change the effect, simply **slow the march** so the motion is gentle when
the OS requests reduced motion. Override only `animation-duration` so the same
`agent-ant-march` keyframe runs slower; the striped gradient and translation are
unchanged.

```scss
@media (prefers-reduced-motion: reduce) {
    .agent-pane-progress-bar--active::before {
        animation-duration: 1.5s;   // gentler march; same keyframe
    }
}
```

Notes:
- This keeps a single effect (marching ants) on every machine, so a
  reduced-motion box no longer shows a different-looking bar. Motion-enabled
  machines keep the 0.5s march; reduced-motion machines get a calmer 1.5s march.
- `animation: none` (freeze) was rejected: it reads as a stuck bar (the original
  Windows 11 report). The opacity breathe was rejected: it reads as a glow and
  is a different effect from the ants.

### Secondary robustness (compositor)

`will-change: transform` is declared permanently, promoting the layer for the
bar's whole lifetime even at rest. This is not the cause of the freeze above,
but to avoid a permanently-promoted idle layer (and any driver-specific
tick-starvation on a promoted-but-static layer), scope `will-change` to the
animating state only:

```scss
.agent-pane-progress-bar::before { /* no will-change here */ }
.agent-pane-progress-bar--active::before { will-change: transform; }
```

---

## 3. Problem 2: translucent stripe gaps let underlying text bleed through

### Root cause

The gap stripes use `color-mix(in srgb, var(--accent-color) 25%, transparent)`,
i.e. 25% accent over 75% **transparent**, and the bar element has **no opaque
background**. The bar sits at `top:0` (z-index 4) over the agent content. As the
ants march across content that reaches the top edge (text, headers), that
content shows through the translucent gaps. The clean stripe pattern is muddied
and the effect is lost.

### Requirement

The bar must be fully opaque. Nothing behind the 3px strip should be visible
through the gaps, in any theme.

### Proposed fix

Two layers of defense (apply both):

1. **Opaque base on the bar element** so the 3px strip always covers the content
   beneath it, matching the pane surface so the bar reads as chrome:

   ```scss
   .agent-pane-progress-bar {
       background: var(--main-bg-color);   // same token .agent-view paints
       // ...existing props...
   }
   ```

2. **Opaque gap stripes** so the pattern itself never carries alpha. Mix the
   faded stripe against the pane background instead of `transparent`:

   ```scss
   &::before {
       background: var(--accent-color);    // pre-color-mix fallback (unchanged)
       background: repeating-linear-gradient(
           -45deg,
           var(--accent-color)                                          0px,
           var(--accent-color)                                          4px,
           color-mix(in srgb, var(--accent-color) 25%, var(--main-bg-color)) 4px,
           color-mix(in srgb, var(--accent-color) 25%, var(--main-bg-color)) 8px
       );
   }
   ```

   This keeps the same visual contrast (bright accent stripe vs. dim accent-tinted
   stripe) but every stop is fully opaque, so the marching pattern stays crisp
   over any content.

`--main-bg-color` is the agent pane's own background, so the bar blends into the
pane chrome and the ants ride on the surface rather than floating over glass.
(If a more saturated bar is wanted, `--block-bg-solid-color` is an alternative
opaque token; `--main-bg-color` is the match for the agent view.)

---

## 3b. Problem 3: bar scales with pane zoom

### Root cause

`.agent-view` applies per-pane zoom as the CSS `zoom` property
(`agent-view.tsx:1000`, `style={{ zoom: zoomFactor() }}`, range 0.5-2.0). The
progress bar is a child of `.agent-view`, so `zoom` scales the whole bar: at 2x
the 3px bar renders 6px tall with a doubled stripe period and march distance; at
0.5x it shrinks. The busy indicator is chrome, not content, so it should read at
a constant size no matter how the user has zoomed the pane.

### Requirement

The bar's height, stripe period, overshoot, and march distance render at a fixed
screen size at any pane zoom. (Width still spans the pane, by design.)

### Proposed fix

Counter-scale every px dimension by the zoom. `zoom` re-multiplies all lengths in
the subtree, so dividing first yields a fixed rendered size. Expose the factor as
a custom property (custom properties are plain values, unaffected by `zoom`):

```tsx
// agent-view.tsx
style={{ zoom: zoomFactor(), "--agent-pane-zoom": String(zoomFactor()) }}
```

```scss
// _control-bar.scss — divide every px by var(--agent-pane-zoom, 1)
.agent-pane-progress-bar { height: calc(3px / var(--agent-pane-zoom, 1)); }
.agent-pane-progress-bar::before {
    left:  calc(-12px / var(--agent-pane-zoom, 1));
    right: calc(-12px / var(--agent-pane-zoom, 1));
    background: repeating-linear-gradient(-45deg,
        var(--accent-color) 0px,
        var(--accent-color) calc(4px / var(--agent-pane-zoom, 1)),
        color-mix(in srgb, var(--accent-color) 25%, var(--main-bg-color)) calc(4px / var(--agent-pane-zoom, 1)),
        color-mix(in srgb, var(--accent-color) 25%, var(--main-bg-color)) calc(8px / var(--agent-pane-zoom, 1)));
}
@keyframes agent-ant-march {
    from { transform: translateX(0); }
    to   { transform: translateX(calc(11.314px / var(--agent-pane-zoom, 1))); }
}
```

Notes:
- `var(--agent-pane-zoom)` is set on `.agent-view` and inherits down to the bar
  and its `::before` (custom properties inherit), so the keyframe resolves it per
  element. Fallback `1` keeps current behavior if the var is ever absent.
- Overshoot is scaled by the same factor so it stays larger than the (also
  scaled) march distance, preserving edge coverage at every zoom.

---

## 4. Out of scope

- The 0.5s march speed, stripe width (4px/4px), -45deg angle, and 3px height are
  unchanged; this spec only fixes the freeze and the transparency.
- No change to the show/hide logic or the `--stopping` interrupt dim.
- No JS changes; `agent-view.tsx` markup is untouched.

---

## 5. Verification

1. **Reduced motion (the reported bug):** with claudius's Animation effects OFF
   (current state, `SPI_GETCLIENTAREAANIMATION = false`), launch an agent turn
   and confirm the bar breathes (or, in the strict variant, shows a solid bar)
   instead of freezing. Toggle Animation effects ON and confirm the ants march.
   In DevTools, `Rendering > Emulate CSS prefers-reduced-motion` reproduces both
   states without changing OS settings.
2. **Opacity / no bleed:** start a turn with content (a long header / wrapped
   text) reaching the very top of the agent view. Confirm no text is visible
   through the stripe gaps as the bar animates, in both a light and a dark theme.
3. **Compositor:** DevTools `Layers` panel - confirm the bar's layer is promoted
   only while `--active`, and `Performance` shows the animation on the compositor
   thread (no main-thread paint per frame).
4. **Regression:** confirm normal (animations-on) Windows and Linux still show
   the marching ants exactly as in #1694.

---

## 6. Summary of edits (all in `_control-bar.scss`)

| Change | Reason |
|--------|--------|
| Add `background: var(--main-bg-color)` to `.agent-pane-progress-bar` | Opaque base, no content bleed (Problem 2) |
| Gap stops `…, transparent` -> `…, var(--main-bg-color)` | Opaque stripe pattern (Problem 2) |
| Move `will-change: transform` onto `--active::before` only | Avoid permanent idle layer promotion (Problem 1 robustness) |
| Reduced-motion: keep `agent-ant-march`, override `animation-duration: 1.5s` (slower) | Bar no longer freezes (Problem 1) and shows the same ants effect everywhere, just gentler (no "aurora" glow) |
