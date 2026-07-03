# SPEC — Composer strip responsive architecture: stop hiding the runtime controls

**Date:** 2026-07-02
**Type:** Architecture spec
**Status:** Root-caused (live-verified); proposes an architecture rethink + an immediate fix.
**Owner:** asaf
**Scope:** `frontend/app/view/agent/components/AgentComposerStrip.tsx` +
`frontend/app/view/agent/styles/_composer-strip.scss`.

> **Symptom.** On a normal agent pane the composer strip shows **only the "Log" button** — no
> Mode / Model / Effort controls. Reported as "may need an architecture rethink … a lot of hackish crude."

---

## 1. Root cause (live-verified, not theory)

Instrumented the running dev build (`task dev`, this branch) with a DOM probe. Findings:

| Probe | Value |
|---|---|
| `showControls()` | **true** (`providerId=claude`, `agentProvider=claude`, `blockAtom!=null`) |
| Trigger buttons in DOM | **3** — rendered with correct labels **`Bypass▴` `Sonnet 4.6▴` `High▴`** |
| Their computed style | **`display:none`, width 0** |
| Strip width | **310px** |

So the controls **are** rendered correctly (the drop-up works) — they're **force-hidden by CSS**. The
culprit is the narrow-pane container query (`_composer-strip.scss:219`):

```scss
@container modal-mount (max-width: 320px) {
    .agent-composer-strip-select { display: none; }        // ← hides ALL runtime controls
    .agent-composer-strip-log-btn { font-size: 9px; padding: 1px 3px; }
}
```

The user's pane container is **310px** — under the **320px** threshold — so **every** Mode/Model/Effort
control is `display:none`, leaving only Log. **320px is a very common pane width**, so this fires for a
large fraction of real panes. This predates the drop-up work (it hid the native `<select>`s the same
way); the drop-up just made it obvious because the user was looking for the new controls.

## 2. The "hackish crude" — what's actually wrong architecturally

1. **Blunt `display:none` on *essential* controls.** Responsive shrink is done by hiding the runtime
   controls entirely — no fallback, no collapsed form. The controls (how the agent runs: Mode/Model/
   Effort) are arguably the *most* important thing in the strip, and they're the *first* thing dropped.
2. **The truncation priority is backwards.** The strip *keeps* informational content (token stats,
   elapsed, context text) and *drops* the actionable controls. It should be the reverse: shed
   informational bits first; controls last.
3. **A magic 320px threshold, guessed not measured.** It's a single hardcoded breakpoint that doesn't
   correspond to the actual rendered width of the controls, so it clips at a width where the controls
   would still fit (three compact drop-up pills fit well under 320px).
4. **Two overlapping truncation systems.** There's a `240px` query (drops stats + ctx) *and* a `320px`
   query (drops controls, shrinks Log) — applied in the "wrong" order (controls go at the *wider* 320px
   breakpoint, before some informational content at 240px).
5. **Historical multi-consumer drift** (context, mostly resolved): Mode/Model/Effort were defined
   independently in `AgentComposerStrip`, `AgentControlBar`, and the `/model` command — three lists that
   drifted. #1912 removed the `AgentControlBar` duplicate; the strip + `/model` should converge on one
   catalog (see `SPEC_MODEL_CATALOG_REFRESH_2026_07_02`).
6. **Provider gating is a hard string check** (`providerId === "claude"`). Works, but tightly couples the
   strip to one provider id; fine for now, flagged for awareness.

## 3. Proposed architecture — progressive collapse, controls last

**Principle: the runtime controls never fully disappear.** As width shrinks, degrade *gracefully* and
shed *informational* content first. The drop-up trigger is the right primitive for this — it can shrink
from full label → short label → icon-only, and its popup (opened upward) always shows the full options
regardless of trigger size.

### Width tiers (measure real content widths; these are illustrative)
```
WIDE (≳ 420px) — everything, full labels
┌──────────────────────────────────────────────────────────────────────┐
│ [Bypass ▴][Sonnet 4.6 ▴][X-High ▴]  [Log]      ↑2k ↓1k 1m12s ⚙2 12k/64k│
└──────────────────────────────────────────────────────────────────────┘

MEDIUM (~320–420px) — drop informational tail first (stats → ctx), keep controls full
┌──────────────────────────────────────────────────┐
│ [Bypass ▴][Sonnet 4.6 ▴][X-High ▴]  [Log]   12k/64k│
└──────────────────────────────────────────────────┘

NARROW (~240–320px) — controls shrink to short/icon; Log stays; info gone
┌────────────────────────────────────┐
│ [Byp▴][Son▴][XHi▴]  [Log]           │   ← still actionable, not hidden
└────────────────────────────────────┘

TINY (< 240px) — controls collapse into ONE combined chip; Log stays
┌────────────────────────┐
│ [⚙ Byp·Son·XHi ▴] [Log]│   ← one trigger opens a small drop-up panel
└────────────────────────┘        with Mode / Model / Effort stacked
```

### Rules
- **Never** apply `display:none` to the runtime controls. Instead:
  - **Tier MEDIUM:** hide `.agent-composer-strip-stats` then `.agent-composer-strip-ctx` (informational).
  - **Tier NARROW:** switch the drop-up triggers to a compact form — short label (first 3 chars) or
    icon-only. The popup still shows full option text.
  - **Tier TINY:** collapse the three triggers into a **single combined chip** (`⚙ Byp·Son·XHi ▴`) whose
    drop-up panel stacks Mode/Model/Effort. This guarantees the controls are always reachable at any
    width the pane can realistically be.
- **Order of shedding (widest→narrowest):** stats → elapsed → context text → process badge → control
  labels (→ icons) → combine controls. Log and the controls are the last things standing.
- **Breakpoints come from measured content**, not a guessed 320. Prefer a container query per tier keyed
  to the widths the compact forms actually need (a drop-up pill is ~44–68px; three fit < 240px in short
  form).
- **One control model.** Define Mode/Model/Effort as a single data list (value + label + optional short
  label + optional color) and render every surface (strip, `/model`, any future one) from it — so labels
  never drift and the compact/short forms are defined once.

## 4. Immediate fix (unblocks the current bug now, low-risk)

The full progressive-collapse is the *architecture*; the one-line unblock is: **stop hiding the controls,
and if anything must go at narrow widths, drop the informational content, not the controls.** Concretely
in `_composer-strip.scss`:

- **Remove** `.agent-composer-strip-select { display: none; }` from the `≤320px` query (drop-up pills are
  compact enough to keep). Keep the Log shrink.
- Optionally move stats/ctx hiding up so informational content sheds first.

This makes the drop-ups appear on the 310px pane immediately (verified: they render `Bypass▴ Sonnet 4.6▴
High▴`, just hidden). Progressive collapse (short/icon/combined forms) is the follow-up that makes narrow
widths graceful rather than just "unhidden."

## 5. Verification
- Live probe (2026-07-02): 3 triggers present, labels correct, `display:none` at strip width 310px.
- After the §4 fix: the three drop-ups show on a 310px pane; opening each opens upward with full options.
- Progressive collapse: shrink the pane through the tiers and confirm controls never vanish (they shrink /
  combine), and informational content sheds first.

## 6. Sources
- Root cause: `frontend/app/view/agent/styles/_composer-strip.scss:171-182,219` (the `240px` + `320px`
  container queries); `AgentComposerStrip.tsx` (drop-up triggers use `.agent-composer-strip-select`).
- Related: `SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02` (Mode promotion + Fix 7 drop-ups),
  `SPEC_MODEL_CATALOG_REFRESH_2026_07_02` (one control catalog), PR #1922 (drop-up impl).
