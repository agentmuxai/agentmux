# Investigation: Responsive Widget Labels in the Action Widget Bar

**Date:** 2026-05-22
**Owner:** AgentY
**Branch:** `agenty/refine-tab-appearance`
**Status:** Analysis — implementation pending one open decision

---

## 1. Goal

The widget text labels in the action widget bar should **show or hide automatically
based on available width**: when the title bar is too narrow to fit the labeled
widgets, the labels collapse so only the icons remain.

Today this is a manual, all-or-nothing toggle with no width-awareness.

---

## 2. Current Implementation

### Layout

The widget bar (`ActionWidgets`) sits on the right of the window header, inside
`SystemStatus`:

```
.window-header
   ├── WindowDrag .left
   ├── TabBar              (flex: 1 1 auto — grows/shrinks)
   └── SystemStatus
         ├── ActionWidgets       ← the widget bar
         └── WindowActionButtons (min / max / close)
```

Files:
- `frontend/app/window/action-widgets.tsx` — widget bar component
- `frontend/app/window/action-widgets.scss` — widget bar styles
- `frontend/app/window/system-status.tsx` — host container
- `frontend/app/window/window-header.tsx` — title bar root

### Label gating

Each widget renders an icon and a label. The label is gated by one boolean:

```tsx
// action-widgets.tsx:213
const iconOnly = () => settings()["widget:icononly"] ?? true;

// action-widgets.tsx:120 — inside ActionWidget
<Show when={!iconOnly() && !isBlank(widget.label)}>
    <div class="text-xs whitespace-nowrap">{widget.label}</div>
</Show>
```

The same flag also gates the "more" button's label (`action-widgets.tsx:428`).

`widget:icononly` is a user setting toggled via right-click on the bar →
"Icon Only" checkbox (`handleBarContextMenu`, `action-widgets.tsx:342`).

### Key observations

- **Default is `true`** — labels are hidden out of the box. Labels only appear
  if the user explicitly turns the setting off.
- **No width-responsiveness exists.** Nothing observes the window-header or
  window width. Labels are purely a static user preference.
- Labels are **conditionally rendered** (`<Show>`), not always-in-DOM-then-
  CSS-hidden. A pure-CSS container-query approach would require restructuring
  this to always render the label and hide it via CSS.

---

## 3. Approach — Overflow Detection (decided)

Two approaches were considered:

| Approach | How | Verdict |
|---|---|---|
| **Width threshold** | `ResizeObserver` on `.window-header`; force icon-only below a fixed pixel width. | Rejected — the threshold is a magic number that needs re-tuning whenever widgets are pinned/unpinned. |
| **Overflow detection** | Measure whether the labeled widgets actually fit the available space; drop labels only when they would overflow. | **Chosen** — self-tuning, no magic constant, adapts to the current set of pinned widgets. |

### Layout finding — the widget bar genuinely compresses

The window header is a flex row:

| Child | Flex | Behavior |
|---|---|---|
| `.window-drag .left` | `flex-shrink: 0` | fixed 2px |
| `.tab-bar` | `flex: 1 1 auto`, `min-width: 0` | grows/shrinks, clips tabs |
| `.system-status` | `flex: 0 1 auto` (default) | **shrinkable** |

`.system-status` → `.action-widgets` are both shrinkable (`flex-shrink: 1`).
So when the window narrows, the widget bar **is** compressed below its natural
width — `scrollWidth > clientWidth` becomes true on `.action-widgets`. Overflow
on the widget bar itself is therefore directly observable; we do not need to
reach into the tab bar's DOM.

### Mechanics

1. A **hidden measuring mirror** of the widget bar is rendered off-screen,
   always in the *labeled* state. Its width is the labeled natural width and
   does not depend on what the visible bar is currently rendering.
2. A `ResizeObserver` tracks the visible bar's available `clientWidth`.
3. `tooNarrow` = `labeledNaturalWidth > availableWidth`.
4. The visible bar renders icon-only whenever `tooNarrow` is true.

Using a mirror (rather than measuring the live bar) means the decision is
computed from an invariant width — collapsing the visible bar cannot change
the mirror's measurement, so there is **no collapse→expand oscillation** and no
hysteresis fudge factor is needed. The mirror re-measures automatically when
the pinned widget set changes (`ResizeObserver` fires on its own resize).

The frontend has no existing `ResizeObserver` usage — this is the first.

---

## 4. Resolved Decisions

### 4.1 Keep `widget:icononly`, default it to `false`

The manual setting **stays**. It keeps one job: let a user force icon-only
*even on a wide screen* where labels would otherwise fit. The new behavior is
additive:

```
effectiveIconOnly = manualSetting || tooNarrow
```

- Manual "Icon Only" ON  → always icon-only.
- Manual "Icon Only" OFF → labels show when they fit, auto-collapse when narrow.

**Default flips from `true` → `false`.** Labels now show by default (when there
is room); `widget:icononly` only needs to be set when a user explicitly wants
the always-compact look.

### 4.2 Show all widgets in the bar by default

Today the default pinned set is derived from `display:pinned` in
`widgets.json` — only 4 of 8 widgets (`agent`, `browser`, `terminal`,
`sysinfo`); the other 4 (`editor`, `swarm`, `drone`, `help`) default into the
"More" dropdown.

**New default: all widgets are pinned/shown in the bar.** The "More" dropdown
becomes empty by default and only populates if a user unpins a widget.

This pairs naturally with §4.1's responsive labels: with 8 widgets the bar is
wider, but labels auto-collapse to icons when the window is narrow, so the
fuller default bar stays usable at any width.

Implementation: set `display:pinned: true` on every entry in
`agentmux-srv/src/config/widgets.json` (the data-driven path — `getPinnedKeys`
already derives the default set from that flag). The widget table in the
repo's `CLAUDE.md` must be updated to match (all rows become "Pinned").

---

## 5. Implementation Plan

1. Flip the `widget:icononly` default: `?? true` → `?? false`
   (`action-widgets.tsx:213`).
2. Set `display:pinned: true` on all entries in `widgets.json` so all widgets
   show by default (§4.2).
3. Render a hidden, always-labeled measuring mirror of the widget bar; capture
   its width as `labeledNaturalWidth`.
4. Add a `ResizeObserver` on the visible `.action-widgets` to track
   `availableWidth`; derive `tooNarrow = labeledNaturalWidth > availableWidth`.
5. Add `effectiveIconOnly = iconOnly() || tooNarrow()` and replace the raw
   `iconOnly()` usage at `action-widgets.tsx:120` and `:428`. The
   `handleBarContextMenu` checkbox stays bound to the raw `iconOnly()` setting.
6. Update the widget table in the repo `CLAUDE.md` (all rows → "Pinned").
7. Verify in `task dev`: shrink the window — labels drop before the bar
   overflows and reappear when widened, with no flicker at the boundary;
   confirm all 8 widgets show on a fresh config.

---

## 6. Files to Touch

| File | Change |
|---|---|
| `frontend/app/window/action-widgets.tsx` | Default flip, mirror, `ResizeObserver`, `tooNarrow`, `effectiveIconOnly` |
| `frontend/app/window/action-widgets.scss` | Off-screen mirror styling |
| `agentmux-srv/src/config/widgets.json` | `display:pinned: true` on all widgets |
| `CLAUDE.md` (repo root) | Widget table — all rows become "Pinned" |
