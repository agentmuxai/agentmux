# Agent Busy Animation — Aurora Bar

**Date:** 2026-06-21
**Scope:** `frontend/app/view/agent/styles/_control-bar.scss`
**Replaces:** current `agent-gradient-sweep` + `agent-pane-progress-bar::before` implementation

---

## Goal

Replace the simple left→right gradient sweep on the agent busy bar with an
organic, non-periodic aurora-like flowing animation that conveys passage of time
without looking mechanical.

---

## Color

**Use the color-wheel complement of `--accent-color`**, not `--accent-color`
itself. The complement has maximum contrast against the theme's primary hue,
making the "working" state immediately legible without blending into accented
UI elements.

Computed via **CSS relative color syntax** (no JS required):

```css
hsl(from var(--accent-color) calc(h + 180) s l)
```

Chrome 119+ / CEF equivalent supports this. No fallback needed for our target
runtime.

### Gradient stops

Use three nearby hues around the complement to create depth and warmth. The
small ±20° spread is enough for visual richness without looking garish:

| Stop | Expression | Role |
|------|-----------|------|
| 0%   | `hsl(from var(--accent-color) calc(h + 160) s calc(l * 0.85))` | cooler edge |
| 30%  | `hsl(from var(--accent-color) calc(h + 180) s l)`              | pure complement |
| 60%  | `hsl(from var(--accent-color) calc(h + 200) s calc(l * 0.90))` | warmer middle  |
| 85%  | `hsl(from var(--accent-color) calc(h + 175) s calc(l * 1.10))` | bright peak    |
| 100% | `hsl(from var(--accent-color) calc(h + 160) s calc(l * 0.85))` | cooler edge    |

Gradient direction: `135deg` (diagonal — allows the rotation animation to create
different slice angles as the frame plays).

---

## Animation Technique

**Pure-CSS, compositor-only. Zero JS. Zero canvas. Zero SVG filters.**

Source: Auroral library pattern (verified by deep research 2026-06-21, 3-0 vote
on performance; SVG feTurbulence refuted 0-3 for thin-bar use).

### Why not feTurbulence?
Adversarially refuted (0-3 vote) for progress bars — CPU-bound, not compositor-
promoted, battery-intensive. The pure-CSS approach avoids paint and layout
entirely.

### Mechanism

The `::before` pseudo-element is made significantly taller and wider than the
2px bar. `overflow: hidden` on the parent clips the visible slice to 2px.

A single `@keyframes` block combines `translateX` and `rotate`. Because the
element is much taller than the visible window, the rotation causes a **different
diagonal slice** of the gradient to cross the visible 2px at each point in the
animation cycle. This slice changes non-linearly — producing the organic,
cloud-like flowing quality without noise functions.

`animation-direction: alternate` means the forward and reverse plays follow
slightly different paths (different rotation angle each way), breaking the
otherwise-periodic feel.

### Sizing

```
parent:   width: 100%, height: 2px, overflow: hidden
::before: width: 200%, height: 120px
          left: -50%
          top: 50%
          transform-origin: center
```

The 120px height with ±12° rotation means the rotated edges (width × sin(12°) ≈
41px of vertical travel) stay well within the 120px pseudo-element — no
transparent gaps at the bar edges.

### Timings

| Property | Value | Rationale |
|----------|-------|-----------|
| translate period | 4.5s | slower than current 2s — less mechanical |
| rotate period | 7.3s | prime-ish ratio vs translate → aperiodic beat |
| `animation-direction` | `alternate` | breaks left↔right symmetry |
| `animation-timing-function` | `ease-in-out` | smooth deceleration at extremes |

The two periods (4.5s, 7.3s) are incommensurate, so the combined motion doesn't
cycle back to the exact start for ~32 seconds — effectively non-repeating during
normal agent turns.

---

## Implementation Sketch

```scss
.agent-pane-progress-bar {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 2px;
    z-index: var(--z-pane-overlay, 4);
    pointer-events: none;
    opacity: 0;
    overflow: hidden;
    transition: opacity 200ms ease;

    &::before {
        content: "";
        position: absolute;
        width: 200%;
        height: 120px;
        left: -50%;
        top: 50%;
        transform-origin: center;
        background: linear-gradient(
            135deg,
            hsl(from var(--accent-color) calc(h + 160) s calc(l * 0.85))  0%,
            hsl(from var(--accent-color) calc(h + 180) s l)               30%,
            hsl(from var(--accent-color) calc(h + 200) s calc(l * 0.90)) 60%,
            hsl(from var(--accent-color) calc(h + 175) s calc(l * 1.10)) 85%,
            hsl(from var(--accent-color) calc(h + 160) s calc(l * 0.85)) 100%
        );
        will-change: transform;
    }

    &--active {
        opacity: 1;

        &::before {
            animation:
                aurora-translate 4.5s ease-in-out infinite alternate,
                aurora-rotate    7.3s ease-in-out infinite alternate;
        }
    }

    &--active#{&}--stopping {
        opacity: 0.55;
    }
}

@keyframes aurora-translate {
    from { transform: translateX(-15%); }
    to   { transform: translateX(15%); }
}

@keyframes aurora-rotate {
    from { transform: rotate(-12deg); }
    to   { transform: rotate(12deg); }
}
```

> **Note on dual transforms:** two `@keyframes` on the same property (`transform`)
> will conflict — the last one wins each frame. The actual implementation must
> combine them into a single keyframe with staggered non-linear values, or use
> a nested wrapper element. The sketch above is for illustration; the real SCSS
> must use one unified `@keyframes aurora-sweep` that expresses both translation
> and rotation at each keyframe stop.

---

## Unified Keyframe (correct implementation)

```scss
@keyframes aurora-sweep {
    0%   { transform: translateX(-15%) rotate(-12deg); }
    25%  { transform: translateX(5%)   rotate(4deg);  }
    50%  { transform: translateX(15%)  rotate(-6deg); }
    75%  { transform: translateX(-5%)  rotate(10deg); }
    100% { transform: translateX(-15%) rotate(-12deg); }
}
```

Single animation: `aurora-sweep 9s ease-in-out infinite`. The non-uniform
stops (−15%, +5%, +15%, −5%) combined with the rotation mean no two 9-second
cycles look identical visually because of how the diagonal gradient is sliced.

---

## Reduced Motion

```scss
@media (prefers-reduced-motion: reduce) {
    .agent-pane-progress-bar--active::before {
        animation: none;
        transform: translateX(0) rotate(0deg);
        opacity: 0.7;
    }
}
```

---

## Out of Scope

- Canvas, WebGL, SVG filters — all ruled out for this bar context
- Changing bar height (stays 2px)
- Per-agent color differentiation (future, tracked separately)
- The `--accent-color` complement is **not** applied to `agent-spinner-dot` or
  any other status indicators — this spec is strictly the top progress bar
