# SPEC: Divider-Pill Rule Misalignment Fix

**Date:** 2026-08-12
**Status:** Implemented 2026-08-13
**Affects:** `context_compacted`, `session_outcome` transcript dividers

**Correction (2026-08-13):** the original draft of this spec also listed
`compaction_started` ("Compacting conversation…") as affected. On closer
inspection of the actual CSS, `.agent-compaction-started` never had
`::before`/`::after` rule lines at all — it renders as a plain pill with no
flanking rule, so it was never subject to this bug. It is unchanged by the
fix below; see "Out of Scope".

---

## Problem

The transcript dividers that show a centered pill flanked by horizontal rules —
"CONTEXT COMPACTED", "COMPACTING CONVERSATION…", "NEW SESSION STARTED" /
"SESSION CONTINUED" — render with the rule lines crossing directly through the
pill text instead of framing it cleanly. The effect reads as garbled/overlapping
text rather than the intended `─── label ───` divider look described in
`docs/specs/SPEC_CONTEXT_COMPACTION_NOTIFICATION_2026_06_20.md` (§Surface):

```
─────────────── context compacted ───────────────
        Earlier history summarized · 847k → 52k tokens
```

The calendar-day divider (`.agent-day-divider`, used in the Agent History view)
renders correctly — it's the reference for what "right" looks like.

## Root Cause

Both bugs live in `frontend/app/view/agent/styles/_document-nodes.scss`, and
both stem from copying `.agent-day-divider`'s rule pattern onto a container
shape it wasn't designed for.

### 1. The rule's `top: 50%` bisects the wrong box

`.agent-day-divider` has exactly **one** row of content (the pill) inside a
`justify-content: center` flex row, so `top: 50%` naturally lands at the
pill's own vertical center:

```scss
.agent-day-divider {
    display: flex;
    align-items: center;
    justify-content: center;
    // ...
    &::before, &::after { position: absolute; top: 50%; /* ... */ }
}
```

`.agent-context-compacted` and `.agent-session-outcome` instead stack **two**
rows — the pill, then a detail line — inside a `flex-direction: column`
container, but keep the exact same `top: 50%` rule positioned on the *outer*
two-row container:

```scss
.agent-context-compacted {
    display: flex;
    flex-direction: column;   // pill row + detail row, stacked
    align-items: center;
    gap: 3px;
    padding: 10px var(--space-2);
    position: relative;

    &::before, &::after { position: absolute; top: 50%; /* ... */ }
    // ...
    .agent-context-compacted-label  { /* pill, ~19px tall */ }
    .agent-context-compacted-detail { /* detail line, below the pill */ }
}
```

`top: 50%` here is 50% of the *combined* pill+gap+detail height, not 50% of
the pill's own height. Working through the actual box metrics
(`--space-2: 8px` horizontal padding, 10px vertical padding, 11px labels,
2px pill padding + 1px border, 3px gap):

- Pill occupies roughly the content range `[10px, 29px]` from the box top.
- Detail line occupies roughly `[32px, 45px]`.
- Total box height ≈ 55px, so `top: 50%` ≈ 28px — inside the pill's range,
  but ~90% of the way down it, i.e. just above the pill's *bottom* border,
  not centered on it.

Because the detail line's height varies with content (token counts, duration
suffix, pane width causing wraps), the 50% mark moves around: on a wide pane
with a short detail line the rule sits almost on the pill's bottom edge; on a
narrow pane where the detail line wraps to two lines, the rule can end up
crossing into the detail text instead. Nothing about the rule's position is
actually anchored to the pill — it just happens to land somewhere in its
lower half most of the time, which is what makes it look like it's slicing
through the label text.

### 2. The pill has no background to mask the rule

`.agent-day-divider-label` gives the pill an opaque(ish) background so the
segment of rule line that geometrically falls behind it is hidden:

```scss
.agent-day-divider-label {
    // ...
    background: var(--panel-bg-color, rgba(255, 255, 255, 0.04));
}
```

`.agent-context-compacted-label` and `.agent-session-outcome-label` never got
the equivalent treatment — they set `border` but no `background`:

```scss
.agent-context-compacted-label {
    font-size: 11px;
    font-weight: 600;
    // ...
    padding: 2px 10px;
    border: 1px solid var(--border-color, rgba(255, 255, 255, 0.12));
    border-radius: 10px;
    // no background
}
```

So even where the rule position happened to be closer to correct, there is
nothing to occlude the line where it passes behind the label — it paints
straight across the transparent pill, visible through/behind the uppercase
text.

Both bugs point at the same underlying mistake: the two broken dividers were
built by pattern-matching `.agent-day-divider`'s CSS onto a two-row layout
without re-deriving where the rule and the background actually need to live
for that shape.

## Reference: the correct pattern (`.agent-day-divider`)

Single row, rule bisects it exactly, pill has an opaque background masking
the line behind it:

```
──────────────  Aug 12  ──────────────
```

No detail line exists in this variant, so there is nothing to get the
vertical math wrong about.

## Proposed Fix

Split the "rule row" from the "detail row" structurally, so the rule's
`top: 50%` only ever bisects a single-row box (mirroring
`.agent-day-divider`), and give every pill variant an explicit background.

### Markup change (`DocumentRow.tsx`)

Wrap the pill in its own rule-bearing element; keep the detail line as a
plain sibling outside it. Applies to both affected node types
(`context_compacted`, `session_outcome`):

```tsx
// Before
<div class="agent-context-compacted">
    <div class="agent-context-compacted-label">context compacted — auto-compacted</div>
    <div class="agent-context-compacted-detail">Earlier history summarized · 847k → 52k tokens</div>
</div>

// After
<div class="agent-context-compacted">
    <div class="agent-context-compacted-rule">
        <span class="agent-context-compacted-label">context compacted — auto-compacted</span>
    </div>
    <div class="agent-context-compacted-detail">Earlier history summarized · 847k → 52k tokens</div>
</div>
```

Same wrapper for `.agent-session-outcome-rule`. `compaction_started` is
untouched (see "Out of Scope").

### CSS change (`_document-nodes.scss`)

Move `position: relative` and the `::before`/`::after` rule off the outer
(multi-row) container and onto the new single-row `-rule` element, matching
`.agent-day-divider`'s shape exactly. The outer container goes back to being
a plain vertical stack with no rule of its own:

```scss
.agent-context-compacted {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    padding: 10px var(--space-2);
    user-select: none;
    // no position: relative, no ::before/::after here anymore

    .agent-context-compacted-rule {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 100%;
        position: relative;

        &::before, &::after {
            content: "";
            position: absolute;
            top: 50%;
            width: 25%;
            height: 1px;
            background: var(--border-color, rgba(255, 255, 255, 0.12));
        }
        &::before { left: 0; }
        &::after  { right: 0; }
    }

    .agent-context-compacted-label {
        font-size: 11px;
        font-weight: 600;
        letter-spacing: 0.06em;
        text-transform: uppercase;
        color: var(--secondary-text-color, rgba(255, 255, 255, 0.45));
        padding: 2px 10px;
        border: 1px solid var(--border-color, rgba(255, 255, 255, 0.12));
        border-radius: 10px;
        background: var(--panel-bg-color, rgba(255, 255, 255, 0.04)); // masks the rule
        position: relative; // stack above the rule's ::before/::after
        z-index: 1;
    }

    .agent-context-compacted-detail {
        font-size: 11px;
        color: var(--tertiary-text-color, rgba(255, 255, 255, 0.3));
    }
}
```

Apply the same restructuring to `.agent-session-outcome`, keeping its
existing accent color (warning-tinted rule + border for
`.agent-session-outcome-fresh`, neutral otherwise) — only the
background/rule scoping changes, not the color logic.

`.agent-day-divider` needs no change; it already has this shape (it just
never grew a second row).

### Why this fixes it

- The rule row now only ever contains the pill, so `top: 50%` on that row is
  always exactly the pill's vertical center — content-length-independent,
  matching `.agent-day-divider`'s already-correct behavior.
- The detail line lives entirely outside the rule row, so it can wrap to any
  number of lines without ever being crossed by the rule.
- The pill's explicit background masks the rule segment behind it in every
  variant, not just the day-divider.

## Files

| File | Change |
|------|--------|
| `frontend/app/view/agent/virtualization/DocumentRow.tsx` | Add `-rule` wrapper `<div>` around each pill (`context_compacted`, `session_outcome` branches) |
| `frontend/app/view/agent/styles/_document-nodes.scss` | Move `position:relative` + `::before`/`::after` from `.agent-context-compacted` / `.agent-session-outcome` onto new `-rule` classes; add `background` to the two `-label` classes |

## Acceptance Criteria

- [x] "Context compacted" and "New session started" / "Session continued"
      dividers render as `─── PILL ───` with no rule line visible crossing
      the pill text, at any pane width
- [x] The rule position does not shift based on detail-line length or wrapping
- [x] Detail line (when present) renders below the rule, never intersected by it
- [x] `.agent-day-divider` (Agent History calendar dividers) is visually
      unchanged (untouched by this change)
- [x] Verified in both light and dark themes (pill background token is themed
      via `--panel-bg-color`, same token `.agent-day-divider-label` already used)

## Out of Scope

- Any change to when these dividers appear (detection/trigger logic) —
  purely a rendering fix
- `.agent-day-divider` — already correct, no changes needed there
- `compaction_started` ("Compacting conversation…") — never had rule lines
  to begin with (just a bordered pill + detail line, no `::before`/`::after`
  at all), so it isn't affected by this bug and is left as-is. If it should
  visually match the other two dividers (with flanking rules), that's a
  separate design decision, not a bug fix — worth a follow-up if desired.
