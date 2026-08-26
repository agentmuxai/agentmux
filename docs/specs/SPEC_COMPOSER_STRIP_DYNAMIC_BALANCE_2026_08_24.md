# SPEC: Composer strip — dynamic left/right slot pooling

**Date:** 2026-08-24
**Status:** Implemented
**Supersedes:** `SPEC_COMPOSER_STRIP_LEFT_RIGHT_BALANCE_2026_08_24.md` (same day)
**Trigger:** Direct user feedback on the static rebalance from earlier the same day: *"it looks the same .. there are stages with empty slots .. did you mot understand the req? the elements need to be equality distributed. along the left and right edges, there should never be an empty slow [sic]."*

## Why the static split failed

The prior spec hardcoded which zone each misc element lived in: process badge + auth tag always in the controls (left) zone, everything else always in the right zone. That looks balanced in the screenshot where every element happens to be visible — but visibility is conditional per element:

- `AgentRuntimeDropup` only renders for Claude agents (`showControls()`).
- The process badge only renders when `processCount > 0`.
- The auth tag only renders once auth status is known.

For a non-Claude agent with no tracked processes and unknown auth status, **all three** of the left zone's possible occupants are hidden simultaneously — the controls zone renders completely empty while the right zone still holds Shell (which has no visibility gate — it always renders) plus whatever else applies. A fixed per-item zone assignment can't account for this; it only "balances" the specific combination it was eyeballed against.

## Change

Replace the fixed zone assignment with dynamic, WEIGHT-balanced pooling, computed fresh on every render. Two iterations happened the same day:

**Rev 1 (count-based, since corrected):** split the pool by item count — `floor(N/2)` left, remainder right. This still produced lopsided results: a 2-left/3-right split by COUNT doesn't account for the context group slot alone rendering up to 3 sub-elements (ctx text + countdown + Compact), so the right zone could need its own internal wrap to a 2nd line while the left zone's smaller content sat on 1 — direct user feedback caught this same-day ("we get 2 lines on the right, 1 on the left — that should be impossible").

**Rev 2 (weight-based, current):**

1. Build an ordered list of "slots" — one entry per element/element-group that's *currently* applicable (see the pool order below). Each entry is `{ key, weight, render }`; `render()` reproduces exactly what that element rendered before (same classes, handlers, titles) — only which zone renders it changed. `weight` is the number of visually distinct sub-elements that slot is rendering right now (see the table below — most slots are a fixed weight 1, but the context group and HOST/SANDBOX+Shell pair have dynamic weight since they bundle multiple sub-elements as one non-splittable unit).
2. Find the prefix cut `splitIndex` (0..pool length) that minimizes `|leftWeight − rightWeight|`, scanning left-to-right and keeping the first (smallest) index that achieves the best diff so far — since weight is always ≥1, cumulative left weight strictly increases with the cut index, making the diff unimodal, so a plain linear scan finds the true minimum, and the "first index wins ties" rule naturally biases toward putting less on the left when two cuts balance equally well.
3. `leftSlots = slots.slice(0, splitIndex)`, `rightSlots = slots.slice(splitIndex)`. Render `<For each={leftSlots()}>`/`<For each={rightSlots()}>` in the controls/right zones respectively.

Slot pool, in order:

| # | Slot | Visibility gate | Weight | Notes |
|---|---|---|---|---|
| 1 | `AgentRuntimeDropup` | `showControls()` (Claude only) | 1 | |
| 2 | Process badge (`⚙N`) | `processCount > 0` | 1 | |
| 3 | Auth tag | `authStatus` known | 1 | |
| 4 | Context group | `ctxText() != null` | 1–3 | `1 + (countdown showing ? 1 : 0) + (Compact showing ? 1 : 0)`. Bundles context text + countdown + Compact button as ONE non-splittable slot — Compact must sit immediately right of the context text (pre-existing constraint, unchanged) — but its weight still reflects how many sub-elements it's actually showing. |
| 5 | HOST/SANDBOX + Shell | always | 1–2 | No visibility gate — Shell always renders (weight 1); +1 if HOST/SANDBOX is showing. Bundled as one atomic slot (never split across zones) per the pre-existing direct-user-request constraint that HOST/SANDBOX sits immediately left of Shell. Always last in the pool; the tie-break in step 2 means it lands in the right zone whenever any other slot is also visible, and only crosses left in the single-slot degenerate case (mathematically unavoidable — one item can't populate two zones). |

## What's unaffected

- Stats (center) zone — unchanged, still the turn/session tokens+elapsed or the live "Compacting…"/"Reconnecting…" readout, not part of this pool.
- The 3-line/2-line/1-line responsive tiers, the `flex-wrap`-based reflow architecture, and every internal-wrap safety net in `_composer-strip.scss` (PR #2393/#2408 clipping-bug fixes) — this change is purely about which zone renders which children, computed reactively; the CSS zone rules (`.agent-composer-strip-controls`/`-right`'s flex/flex-basis/container-query behavior) apply identically regardless of which specific slots a zone currently contains.
- Element styling/behavior — every slot's `render()` is a verbatim copy of what that element rendered under the old fixed-zone JSX (same class names, `data-strip-button`, click handlers, titles/tooltips). No visual or behavioral change to any individual element, only its zone.

## Edge cases

- **Only the HOST/SANDBOX+Shell slot applicable** (non-Claude, no processes, unknown auth, no context yet): pool weight total = 1 or 2, `splitIndex` finds both `i=0` and `i=1` give the same diff, and picks the first (`i=0`) → left zone empty, right zone holds the one slot. Unavoidable — there's nothing else to balance against. Not treated as a bug (see file-header comment in the component).
- **Exactly 2 slots applicable, equal weight** (e.g. runtime + badge, both weight 1): 1 left, 1 right — evenly split.
- **All 5 slots applicable, context group at full weight 3** (runtime=1, badge=1, auth=1, ctx=3, hostShell=1 or 2): total weight 7 or 8. `splitIndex` cuts after auth (weight so far 3) since that's closest to half of 7/8 — left = [runtime, badge, auth] (weight 3), right = [ctx, hostShell] (weight 4 or 5). Both zones now carry comparable visual mass even though the right zone has fewer discrete slots — this is the case the count-based Rev 1 got wrong (it would've put ctx+hostShell's combined 4-5 sub-elements against auth's 1, needing an internal wrap on the right).
- **Context group at weight 1** (ctx text only, no countdown/Compact — e.g. non-Claude with contextWindow known): behaves close to Rev 1's split, since every slot is weight 1 in that case.
- **Reactivity**: `slots` is a `createMemo` depending only on props that change per turn/state-transition (not per-tick) — `showControls()`, `processCount`, `authStatus`, `ctxText()`, `ctxCountdownText()`, `agentMode`, `providerId`/`onCompact` (for the Compact button's weight contribution) — so it doesn't recompute every second the way the stats zone's live elapsed-time readout does.

## Files touched

```
frontend/app/view/agent/components/AgentComposerStrip.tsx   MODIFY — replace the two fixed-zone
                                                               JSX blocks with a `slots`/`leftSlots`/
                                                               `rightSlots` computation + <For> loops.
                                                               Added `For` to the solid-js import.
                                                               No changes to any individual element's
                                                               rendered output, props, or memos
                                                               (rightText/ctxText/ctxClass/
                                                               ctxCountdownText/canCompact all
                                                               unchanged).
docs/specs/SPEC_COMPOSER_STRIP_LEFT_RIGHT_BALANCE_2026_08_24.md   MODIFY — marked Superseded, left
                                                               in place for history per repo
                                                               convention.
```

## Acceptance criteria

1. With every slot applicable, left/right split is weight-balanced (not just count-balanced) — verified by hand-tracing the weight table above for the "all 5 slots, full-weight context group" case.
2. With only the HOST/SANDBOX+Shell slot applicable, it renders in the right zone (not left).
3. With exactly 2 equal-weight slots applicable (any combination), one renders left and one right — neither zone empty.
4. Context text, countdown, and Compact button never appear on opposite sides of the strip from each other.
5. HOST/SANDBOX tag never appears separated from the Shell toggle.
6. All existing `AgentComposerStrip.test.tsx` tests pass unmodified.
7. Typecheck and stylelint clean (SCSS unaffected — no changes needed since zone styling is class-based, not content-based).
8. Visually verified in `task dev`: left zone is never empty while the right zone holds 2+ items, AND neither zone needs an internal wrap to a 2nd line while the other sits on 1 (the specific regression that motivated moving from count-based to weight-based splitting).

## Rev 3 — subset partition, not prefix cut (2026-08-24, same day)

Rev 2's weight-balanced PREFIX cut still failed in practice: traced against a real screenshot showing `runtime(1) + auth(1) + ctx-group(3) + hostShell(2)`, total weight 7. The two heaviest slots (ctx-group=3, hostShell=2) sit adjacent at the end of the pool, so every possible contiguous cut point either separates them (not allowed — both are atomic, non-splittable units) or lumps them together — the best a prefix cut can achieve here is `[runtime,auth]=2` vs `[ctx,hostShell]=5`. That's the literal bug: 1 line of content on the left, 2 lines' worth on the right, direct user feedback: *"we get 2 lines of elements on the right side, by only 1 line of elements on the left. that should be impossible."*

Fix: `leftMask` brute-forces every possible left/right SUBSET assignment (pool capped at 5 slots → at most 32 combinations, trivial) instead of only contiguous prefixes, picking whichever assignment minimizes `|leftWeight − rightWeight|`. Tie-breaks: (a) prefer assignments that keep the hostShell slot (Shell — the strip's one real action) on the right, matching its established outermost-right convention; (b) among remaining ties, prefer fewer items on the left.

For the traced example, the true minimum is `diff=1`: `{ctx-group}=3` left vs `{runtime,auth,hostShell}=4` right. This is a real trade-off worth flagging: achieving genuine weight balance can now place the `AgentRuntimeDropup` (Mode/Model/Effort trigger) on the right zone in some states, abandoning its previous "always leftmost anchor" convention — the algorithm now optimizes purely for balance, not for preserving any single slot's habitual position (other than hostShell's right-side preference, which is a deliberate exception). Flagged for visual confirmation in `task dev` — if a jumping mode-selector position reads as more confusing than the empty-slot/line-mismatch bugs it fixes, that's a real design trade-off to revisit, not a bug in the balancing math itself.

Slots keep their ORIGINAL pool order within whichever side they land on (`leftSlots`/`rightSlots` filter the pool, not reorder it) — so relative order is still predictable once you know which side a slot is on, even though which side depends on the whole pool's weight distribution now, not a fixed per-slot assignment.

## Rev 4 — the bug was in the CSS, not the JS (2026-08-25)

Rev 3's subset-partition search was mathematically correct against its own weight numbers, but still produced visible dead space on one side — confirmed via screenshots, direct user feedback: *"do we need an architecture rethink and modularization? you are violating invariants, for example, I see times there are empty spots on the left when the right is full."*

Root cause, finally isolated with the screenshots in hand: `_composer-strip.scss`'s widest tier (`@container agent-pane (min-width: 482px)`) gave `.agent-composer-strip-controls` and `.agent-composer-strip-right` matching `flex: 1 1 0` — forcing both to occupy **exactly equal width**, regardless of how much actual content each one had. A zone with genuinely less content (e.g. just the runtime trigger) still got stretched to the same box width as a zone holding several items, and its content — packed to one edge via `justify-content` — left real, visible blank space in the rest of its own box. No JS-side rebalancing of *which slot goes where* could fix this: the dead space was a property of the box, not the content assignment. Three consecutive attempts (static assignment, weight-balanced prefix cut, weight-balanced subset partition) were all solving the wrong layer.

**Fix, two parts:**

1. **CSS:** removed the `flex: 1 1 0` forcing entirely. Both zones now stay their initial `flex: 0 1 auto` (content-sized, never stretched) at every tier, including the widest. The outer strip's existing `justify-content: space-between` still pins controls to the line start and right to the line end; with exactly 3 items on the line, the remaining free space splits into 2 equal gaps (one on each side of the stats zone), which reads as "approximately centered" without forcing either edge zone wider than its own content. This is not mathematically perfect centering when controls/right differ significantly in width (true fixed centering regardless of asymmetry would need CSS grid or real DOM measurement — deliberately out of scope here) — a slightly-off-center stats zone was judged the correct trade-off against real, visible dead space.
2. **JS:** replaced the entire weight/subset-partition system with a fixed semantic `side` per slot (left: runtime trigger, process badge, auth tag — state/config; right: context group, HOST/SANDBOX+Shell — counters + the one real action) and a single override: if that leaves the left zone with zero slots while the right has any, borrow the first right-side slot over. No computed weight, no search over possible splits — the JS layer now only solves the problem it was originally introduced for (never a fully empty zone), not visual pixel balance, which is the CSS layer's job.

This also means the file-header comment's "weight" framing (slots have a `weight: number` field) is gone — replaced by `side: "left" | "right"`. See `AgentComposerStrip.tsx`'s own file-header comment for the current, authoritative description; this spec's §3/§6 tables (Rev 2/Rev 3, above) describe superseded designs, kept for history.

### Why this is (hopefully) the last revision

The first three revisions all changed the JS split algorithm while leaving the CSS's forced-equal-width rule in place — each one produced a plausible-looking fix that turned out to still be constrained by that CSS rule, just in a different way each time. This revision is the first to remove the actual constraint. The simplified JS (fixed semantic side + single empty-zone override) is also easier to reason about than any of the three previous attempts, which is its own hedge against a 5th regression: there's no computed number to get subtly wrong, just a fixed table and one clear rule.

## Rev 5 — Rev 4's CSS fix had its own bug, and fixing that exposed a second, independent problem (2026-08-25)

Two more rounds after Rev 4 shipped, both against real screenshots:

**Round A — Rev 4's CSS fix was incomplete.** Removing the widest tier's `flex: 1 1 0` override on `.agent-composer-strip-right` left that zone falling back to its own BASE rule's `flex-basis: 100%` (set unconditionally, needed to force it onto a dedicated line at the narrower tiers) — so at the widest tier, right-zone still forced itself onto its own full-width line, just via a different mechanism than Rev 4 had removed. Symptom: controls alone on line 1 (nothing could fit next to a 100%-wide item), stats sharing line 2 with right instead of proper zone separation — direct user report: *"its literally the same thing... do you understand what the problem is?"* Fix: explicit `.agent-composer-strip-right { flex: 0 1 auto; }` inside the widest-tier query, resetting the base rule's 100% basis rather than relying on an absent override to do it implicitly.

**Round B — fixing Round A exposed the grouping itself was lopsided.** Once the CSS genuinely stopped forcing dead space, the strip correctly collapsed to one line — and Rev 4's side grouping (badge/auth/context-group all on the right, only the runtime trigger on the left) turned out to put most of the strip's actual content on one side in the common case anyway. The context group alone renders up to 3 sub-elements (ctx text, countdown, Compact); the entire left zone at the time was 1-2. Direct user feedback: *"different but same issue .. i cant believe how hard this is for you."* Fix: moved the context group to the LEFT side, pairing it with the runtime trigger — "what agent, what mode, how much context" as one coherent left-side grouping — leaving badge/auth paired with HOST/SANDBOX+Shell on the right as "status indicators + the action button." Chosen by counting realistic sub-element totals for the common (Claude, context tracked) case, not a per-render computed weight.

### Takeaway

Two genuinely different bugs, at two different layers (CSS width-forcing; JS content-grouping), got reported as "the same issue" back-to-back because both manifest as "one side looks heavier than the other." Fixing layer 1 (CSS) was necessary but not sufficient — it just made layer 2's pre-existing lopsidedness visible for the first time instead of masking it inside forced-equal boxes. If this needs touching again: get a real screenshot AND check which layer is actually responsible (is a zone stretched wider than its content, or is the zone's actual content list just unbalanced by design) before changing either one.
