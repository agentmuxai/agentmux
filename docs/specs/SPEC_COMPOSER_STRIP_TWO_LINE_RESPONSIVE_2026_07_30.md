# SPEC — Composer strip: two-line wrap when the pane narrows

**Date:** 2026-07-30
**Type:** UI responsiveness spec
**Status:** Proposed — audited and designed, not yet implemented
**Scope:** `frontend/app/view/agent/components/AgentComposerStrip.tsx` +
`frontend/app/view/agent/styles/_composer-strip.scss`
**Trigger:** User request — *"refine the responsiveness of the stats bar above the agent pane's
input; when the pane is made thinner it should move to 2 lines."*
**Related:** `SPEC_COMPOSER_STRIP_RESPONSIVE_ARCHITECTURE_2026_07_02.md` (progressive-collapse
architecture, controls-never-hidden principle), `SPEC_COMPOSER_STRIP_LAYOUT_MIC_CENTER_MODEL_DEFAULTS_2026_07_10.md`
(introduced the current 3-column grid).

---

## 1. Audit — current state

The "stats bar" is `AgentComposerStrip`, a 28px status row rendered as a **flow sibling**
directly above `.agent-composer-region` (which wraps `AgentFooter`'s textarea) inside
`.agent-view`'s root flex column. It is not absolutely positioned and does not overlap the
input — it pushes the textarea down in normal flow.

```
.agent-view (flex column, container-name: agent-pane)
 ├── AgentComposerStrip        → .agent-composer-strip  (CSS Grid, 1fr auto 1fr)
 └── .agent-composer-region
      └── AgentFooter → .agent-footer → .agent-input-container → textarea.agent-input
```

### Current layout (`_composer-strip.scss:16-35`)

```scss
.agent-composer-strip {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    gap: var(--space-2);
    min-height: 28px;
    ...
}
```

A single-row, 3-column grid: **left** = runtime controls (`AgentRuntimeDropup` "Mode · Model ·
Effort" trigger + optional HOST/SANDBOX tag), **center** = token/elapsed stats (true-centered via
the grid, not a flex auto-margin), **right** = `⚙N` process badge → context text (`12.1k / 64k`)
→ auth tag (`● Logged in`) → `Shell` toggle (rightmost). This 3-column structure is intentional
and recent (2026-07-10) — keep it as the wide-pane baseline.

### Bug found during this audit: the narrow-pane rules are dead code

```scss
// _composer-strip.scss:260-278
@container modal-mount (max-width: 240px) { ... }   // drop stats + ctx
@container modal-mount (max-width: 320px) { ... }   // shrink trigger + Shell
```

Both query container name **`modal-mount`**. The container that actually wraps the composer
strip is `.agent-view`, which declares:

```scss
// agent-view.scss:36-38
.agent-view {
    container-type: inline-size;
    container-name: agent-pane;
}
```

`modal-mount` is a different container name, declared only in `modal.scss` for `<Modal>`
portal mounts. The docked agent pane is never nested inside a `<Modal>`, so **these rules never
match in the real composer strip** — confirmed by the 2026-07-02 spec's own live DOM probe,
which found the trigger buttons `display:none` at a 310px strip width despite querying `@container
modal-mount (max-width: 320px)` (the rule *did* fire there, actually — that probe pre-dates
this finding and the fix was never applied; the `modal-mount` name has been wrong in every
version of this file since it was introduced). Both later specs (07-02, 07-10) proposed content
changes *inside* these dead blocks without correcting the container name, so the mismatch has
persisted through three iterations.

**Net effect today:** narrowing the agent pane produces **no responsive behavior at all** for
the strip — the grid's `1fr` tracks just keep shrinking. `.agent-composer-strip-stats`,
`-ctx`, and `-auth` are all `white-space: nowrap` with no overflow handling of their own, and
`.agent-runtime-dropup-trigger` has no width cap outside the dead rule, so content visually
crowds, clips, or overlaps rather than reflowing. Fixing the container name is a **prerequisite**
for any of this spec's rules to take effect — folded into the proposal below rather than filed
separately, since there's no reason to land it without the behavior it's meant to enable.

---

## 2. Proposed solution

**Mechanism:** below a width threshold, `.agent-composer-strip` switches from a 1-row/3-column
grid to a **2-row grid** via `grid-template-areas`. Controls and the right-side badges/Shell
share row 1; stats move to a full-width, centered row 2. No JSX changes are required for the
base two-line behavior — `AgentComposerStrip.tsx`'s three top-level children
(`.agent-composer-strip-controls`, `.agent-composer-strip-stats-zone`,
`.agent-composer-strip-right`) already map 1:1 onto three named grid areas; only their CSS
`grid-area` assignment needs to change per breakpoint. DOM order (and therefore tab/screen-reader
order) is unaffected.

This also resolves the open question the 07-10 spec left unanswered ("a 3-column grid has no
built-in equivalent of wrap-to-a-second-row... recommend starting with the old container-query
breakpoints and adding a wrap fallback only if narrow testing shows clipping") — clipping is
exactly what's happening today (per §1), so this spec is that follow-up.

### Tier 1 — WIDE (≳480px): single line, unchanged

```
┌──────────────────────────────────────────────────────────────────────────────┐
│ [Auto · Sonnet 5 · Medium ▴] [SANDBOX]   ↑2.1k ↓480 · 1m12s   ⚙3  12.1k/64k  ● Logged in  [Shell] │
└──────────────────────────────────────────────────────────────────────────────┘
```

No change from current behavior or markup.

### Tier 2 — NARROW (2-line, ~300–480px): stats drop to their own row

```
┌────────────────────────────────────────────┐
│ [Auto · Sonnet 5 · Medium ▴]  ⚙3  12.1k/64k  ● Logged in  [Shell] │
│                ↑2.1k ↓480 · 1m12s                          │
└────────────────────────────────────────────┘
```

Row 1 = controls (left) + right zone (badges/ctx/auth/Shell, right) — the two "actionable /
state" clusters. Row 2 = stats, full-width, centered. **Nothing is hidden.** This directly
supersedes the old `≤240px "drop stats"` idea from the 07-02 spec — stats no longer need to be
dropped at all, because giving them a dedicated row removes them from the row-1 space contest
entirely. This is a strictly better outcome than the previously-designed (and never-working)
hide-on-narrow behavior.

### Tier 3 — NARROW-2 (2-line + shed low-priority row-1 items, ~220–300px)

Row 1 still has 5 things competing (controls, process badge, ctx, auth, Shell). Shed the least
essential two, informational-only items — in order:

```
┌──────────────────────────────────┐
│ [Auto · Sonnet 5 · Medium ▴]   12.1k/64k  [Shell] │
│         ↑2.1k ↓480 · 1m12s                 │
└──────────────────────────────────┘
```

1. **Auth tag hidden first** — a passive status indicator (colored dot + "Logged in"); the user
   only needs it when something's wrong, and a failed send already surfaces auth errors inline.
2. **Process badge hidden second** — a shortcut into the swarm view; the swarm view remains
   reachable via its own widget regardless.

Context text (`12.1k / 64k`) and **Shell stay visible at every tier** — ctx is the safety signal
for context-window/compaction awareness (the one thing in this bar with a "critical" pulsing
state), and Shell is the strip's one real call-to-action, consistent with the 07-02 spec's
"controls/actions are shed last, not first" principle. This reorders that spec's original literal
priority list (which put context text ahead of the process badge in the shed order) — that list
predates both the auth tag and the two-line redesign; recommend this revised order but flag it
as a judgment call worth a quick look at real usage.

### Tier 4 — TINY (2-line + controls compact, <220px)

```
┌──────────────────────────┐
│ [Auto·S5·Med ▴]  64k  [Shell] │
│   ↑2.1k ↓480 · 1m12s          │
└──────────────────────────┘
```

Controls trigger shrinks via the existing (now-fixed) `max-width` + ellipsis cap rather than
disappearing — consistent with the 07-02 spec's "never `display:none` the runtime controls"
rule. Full icon-only/combined-chip collapse (the 07-02 spec's TINY tier) is a larger follow-up
or can be layered in later; not required to satisfy "move to 2 lines," called out as an
explicit non-goal below.

---

## 3. CSS sketch

```scss
.agent-composer-strip {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    grid-template-areas: "controls stats right";
    align-items: center;
    gap: var(--space-2);
    min-height: 28px;
    padding: var(--space-1) var(--space-2);
    ...
}

.agent-composer-strip-controls  { grid-area: controls; justify-self: start; ... }
.agent-composer-strip-stats-zone { grid-area: stats;    justify-self: center; ... }
.agent-composer-strip-right      { grid-area: right;    justify-self: end; ... }

// Tier 2 — two-line layout. Fixes the container-name bug (was `modal-mount`,
// never matched the docked pane's own `agent-pane` container — see audit).
@container agent-pane (max-width: 480px) {
    .agent-composer-strip {
        grid-template-columns: 1fr auto;
        grid-template-areas:
            "controls right"
            "stats    stats";
        min-height: 52px;   // two rows + row-gap + padding — verify against real content
        row-gap: 2px;
    }

    .agent-composer-strip-stats-zone {
        justify-self: stretch;
        text-align: center;
    }
}

// Tier 3 — shed auth tag, then process badge (informational, lowest priority).
@container agent-pane (max-width: 300px) {
    .agent-composer-strip-auth { display: none; }
}
@container agent-pane (max-width: 260px) {
    .agent-composer-strip-process-badge { display: none; }
}

// Tier 4 — controls trigger compacts; Shell padding shrinks.
// (Replaces the old dead `@container modal-mount (max-width: 320px)` block.)
@container agent-pane (max-width: 220px) {
    .agent-runtime-dropup-trigger { max-width: 120px; }
    .agent-composer-strip-log-btn { font-size: 9px; padding: 1px 3px; }
}
```

All four tiers key off the **correct** container name, `agent-pane` — matching every other
working responsive rule in `_responsive.scss`.

## 4. Breakpoint values — need live verification

The `480 / 300 / 260 / 220` widths above are reasoned from approximate content widths (controls
pill ~150–170px, right-zone cluster ~230px at Tier 1, stats ~110–130px), not measured DOM output
— same caveat the 07-02 spec flagged about its own guesses. Before shipping: resize a real pane
in `task dev` through these widths and confirm each tier's row 1 doesn't clip or overlap right at
its own threshold; nudge the numbers to match actual rendered widths rather than trusting the
estimate above.

## 5. Non-goals for this pass

- **Icon-only / combined-chip controls** (07-02 spec's full TINY-tier design) — not required to
  satisfy "move to 2 lines"; the Tier 4 sketch above reuses the existing ellipsis-cap behavior
  instead. Worth a follow-up spec if panes are regularly used below ~220px.
- **One shared Mode/Model/Effort catalog** (07-02 spec item 3) — orthogonal, unaffected by this
  change.

## 6. Verification plan (once implemented)

1. `npx tsc --noEmit` — the `grid-area` CSS changes require no `.tsx` edits, so this should be a
   no-op check.
2. `task dev` — resize the agent pane continuously from wide to narrow and confirm:
   - Tier 1→2 transition moves stats to their own centered row with no clipping, at whatever
     width real content requires (adjust the 480px estimate if needed).
   - Tier 2→3: auth tag disappears first, then the process badge: `⚙N` stays clickable up to
     the tier-2 floor and is absent below it.
   - Context text and Shell remain visible and usable at every tier down to the practical
     minimum pane width.
   - No `@container modal-mount` rule remains — confirm via DevTools computed styles that the
     `agent-pane` rules are the ones actually applying.
3. Confirm the strip's height growth at Tier 2+ (28px → ~52px) doesn't visually collide with
   anything above it (message list) or cause a layout jump mid-conversation when a pane is
   resized while a turn is in flight.

## 7. Files touched

```
frontend/app/view/agent/styles/_composer-strip.scss   # grid-template-areas, container queries
```

No `.tsx` changes required for the core two-line behavior (see §2).

---

*End of spec. Proposed — not yet implemented; ready for review/go-ahead.*
