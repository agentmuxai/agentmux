# SPEC: Composer strip — balance misc elements across left/right zones

**Date:** 2026-08-24
**Status:** superseded — same day, see `Superseded-by:` below.
**Superseded-by:** [`SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md`](./SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md)

Why: this spec's static per-item zone assignment (badge+auth always left, everything else always right) looked balanced only when every item happened to be visible at once. In practice the controls zone's other occupant, `AgentRuntimeDropup`, is Claude-only, and badge/auth are each independently conditional — a non-Claude agent with no tracked processes and unknown auth status left the ENTIRE left zone empty while the right zone still held Shell (always rendered). Direct user feedback caught this the same day ("there are stages with empty slots... elements need to be equally distributed... there should never be an empty slot"). Kept here for history; see the superseding spec for the actual (dynamic-pooling) fix.
**Trigger:** User feedback on the current composer strip (`SPEC_COMPOSER_STRIP_CENTERED_SMART_SPLIT_2026_08_14.md`) — the strip already has a deliberate left/center/right zone system with a 3-line/2-line/1-line responsive tier split, but the misc elements aren't actually balanced across the two edge zones: the left (controls) zone holds only the `AgentRuntimeDropup` trigger, while the right zone stacks six items (process badge, context text, context countdown, Compact button, auth tag, HOST/SANDBOX+Shell). "Left and right zones, edge-split" was solved; "misc elements allocated evenly between them" was not.

## Change

Move the process badge (`⚙N`) and the auth tag ("Logged in" / "Not logged in") from the right zone into the controls (left) zone, joining the `AgentRuntimeDropup` trigger. No other zone behavior changes:

- Center (stats zone): unchanged — turn/session tokens+elapsed, or the live "Compacting…"/"Reconnecting…" readout.
- Right zone: unchanged except for the two removed items — still context text, context countdown, Compact button, and the HOST/SANDBOX+Shell pair (which stays fused as one unit per prior direct user feedback — never split by this change). Shell remains the outermost/rightmost element.
- The 3-line/2-line/1-line responsive tiers, the `flex-wrap`-based (not grid) reflow architecture, and every internal-wrap safety net documented in `_composer-strip.scss` (PR #2393/#2408 clipping fixes) are untouched — this is purely a JSX flow-order change (which zone renders which children), not a layout-mechanism change.

## Why this split (badge + auth left, not some other pairing)

Badge and auth are both compact "status" indicators (process count, login state) — the same category as the Mode/Model/Effort control they now sit beside. The right zone's remaining items (context fill, countdown, Compact, HOST/SANDBOX+Shell) are the session's counters and its one real action, which benefit from staying grouped together with Shell anchored at the far right edge (existing, unchanged UX expectation — the primary action sits outermost).

Two pairings from prior specs are explicitly preserved, not renegotiated by this change:
- HOST/SANDBOX must stay immediately left of Shell (`SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md`-era comment: "per direct user request").
- Compact sits immediately right of the context text (existing comment, unchanged).

## Files touched

```
frontend/app/view/agent/components/AgentComposerStrip.tsx   MODIFY — move process-badge/auth
                                                               <Show> blocks from the right zone's
                                                               JSX into the controls zone's JSX.
                                                               No prop/memo/logic changes.
frontend/app/view/agent/styles/_composer-strip.scss          MODIFY — comments only (zone content
                                                               descriptions updated to match); no
                                                               rule/selector/breakpoint changes —
                                                               `.agent-composer-strip-process-badge`
                                                               and `.agent-composer-strip-auth`'s
                                                               styling is class-based, not
                                                               zone-scoped, so it applies unchanged
                                                               regardless of which zone renders them.
```

## Acceptance criteria

1. Controls zone (left, every tier) renders: runtime trigger, then process badge (if any processes tracked), then auth tag (if auth status is known).
2. Right zone no longer renders process badge or auth tag; renders context text, countdown, Compact, HOST/SANDBOX+Shell in that order, unchanged from before.
3. HOST/SANDBOX stays fused to Shell as one flex item (unchanged `.agent-composer-strip-host-shell` wrapper) — never separated by this change.
4. All existing tests in `AgentComposerStrip.test.tsx` (Tier 3 countdown coverage) still pass unmodified — they don't assert zone placement, only countdown text/class behavior.
5. No `max-height`/`overflow:hidden`/grid-template-areas reintroduced — the existing flex-wrap reflow architecture and its documented clipping-bug history (PR #2393/#2408) are unaffected.
6. Visually verified in `task dev` at each of the three tiers (<280px, 280–481px, ≥482px) that the left zone no longer reads as near-empty next to a crowded right zone.
