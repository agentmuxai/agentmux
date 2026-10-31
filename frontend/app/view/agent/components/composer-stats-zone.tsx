// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The strip's stats zone: an empty span since 2026-08-31 (SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md),
 * kept with its measurement so the row layout's fit decision still has a footprint to read.
 */

import { createEffect, createSignal, type Accessor, type JSX } from "solid-js";

export function useStatsZone(
    stripWidth: Accessor<number>,
    slotWidths: Accessor<Record<string, number>>
): { statsWidth: Accessor<number>; statsZone: (variant: "single" | "multi") => JSX.Element } {
    // Real measured footprint of the stats zone (`statsZone` below), fed
    // into the `layout` memo's `statsInline` decision (since 2026-08-26;
    // previously `computeComposerRows`'s now-removed `reservedWidth`
    // param, Codex P1, PR #2812) — whether the stats can share the
    // single row's line. `statsZone()` is rendered in one of two
    // mutually exclusive places (its own doc comment), so it mounts two
    // separate DOM instances even though only one is ever actually
    // attached to a live parent at a time — the OTHER stays detached and
    // reports a 0 rect. Reading whichever one is `isConnected` at effect
    // time gets the real on-screen footprint regardless of which
    // position is currently active.
    //
    // 2026-08-31 (SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md):
    // the stats zone's content was removed (duplicated AgentWorkingRow —
    // see that spec), so `statsZone()` below never renders a content
    // span anymore and this effect now always measures 0. Left in place
    // rather than removed — see that spec's Non-goals — a permanently-0
    // `statsWidth` is harmless input to `statsInline` (the `layout` memo
    // in AgentComposerStrip.tsx), not wrong input, and this effect's own history (right
    // below) is worth keeping intact for whoever eventually does remove
    // this scaffolding for real.
    //
    // Previously depended on `rightText()` (the stats zone's own content
    // changing width) AND `stripWidth()`/`slotWidths()` (ReAgent P1, PR
    // #2812, re-review) — NOT on `layout()` or `statsWidth()` itself, which
    // would be circular (the `layout` memo's `statsInline` reads
    // `statsWidth()`). `stripWidth()`/`slotWidths()` are the actual root
    // inputs behind `statsInline`'s placement flip: without depending on
    // them, resizing across that threshold swaps which of
    // `statsRefs.single`/`.multi` is connected (the `<Show>` blocks
    // toggling) without ever re-running this effect, leaving `statsWidth`
    // stuck at whatever the now-DISCONNECTED variant last reported. A
    // stale, too-small `statsWidth` could then make the `statsInline`
    // check wrongly keep the stats on a line that doesn't actually have
    // room for their real footprint — the same "stale measurement
    // crosses a structural transition" failure mode already fixed for
    // zoom/padding in this same PR, one layer up.
    //
    // Measures the zone's CONTENT (`firstElementChild`, the
    // `.agent-composer-strip-stats` span inside the `<Show>`), NOT the
    // zone wrapper itself — regression found live post-#2813: in the
    // evicted (own-line) position the zone is a direct child of the
    // column-flex strip, so default `align-items: stretch` blockifies it
    // to the FULL strip width even when `rightText()` is empty and it
    // renders nothing. Measuring the wrapper there reported a footprint
    // ≈ the whole strip width, making the fit decision it feeds
    // unsatisfiable at ANY pane width — a one-way trap: the first
    // legitimate visit to the evicted-stats state locked it forever (the
    // widest tier stuck multi-line). The content span's own rect is the
    // real footprint in BOTH positions (inline-block in the below
    // position; `flex: 0 0 auto` child in the single-row position), and
    // is 0/absent exactly when there is nothing to reserve space for.
    let statsRefs: { single?: HTMLElement; multi?: HTMLElement } = {};
    const [statsWidth, setStatsWidth] = createSignal(0);
    createEffect(() => {
        stripWidth();
        slotWidths();
        const zone = statsRefs.single?.isConnected ? statsRefs.single : statsRefs.multi?.isConnected ? statsRefs.multi : undefined;
        const content = zone?.firstElementChild;
        // ceil, not round — same optimistic-fit reasoning as the slot
        // measurement effect in AgentComposerStrip.tsx.
        setStatsWidth(content ? Math.ceil(content.getBoundingClientRect().width / 8) * 8 : 0);
    });

    // Stats zone — no longer shows content (2026-08-31,
    // SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md: this centered
    // token/elapsed readout duplicated AgentWorkingRow's "Worked" row,
    // confusingly — while loading, both showed the identical live
    // turnTokens+elapsed; once idle, this zone showed CUMULATIVE
    // session totals in the same visual shape AgentWorkingRow uses for
    // PER-TURN stats, with no label distinguishing the two).
    // AgentWorkingRow is now the sole surface for these stats. The
    // wrapper span (and the row-layout accommodation in AgentComposerStrip.tsx) stays —
    // removing it would mean touching the strip's most fragile,
    // heavily-revised logic (composer-row-layout.ts's module comment) for a
    // permanently-inert case, not a functional gain; see that spec's
    // Non-goals. Was: always centered (this zone's identity at every
    // tier). Rendered in ONE of two places, mutually
    // exclusive (`statsInline()` vs. not — since 2026-08-26 this is its
    // own decision, no longer synonymous with `rows().length === 1`: a
    // single slot row can have the stats evicted to their own line
    // when slots-plus-stats don't fit together, the middle tier that
    // stops the 1-line → 3-line jump): as the
    // single row's own third child when everything fits one line
    // (matching the true-centered widest-tier position every revision
    // through Rev 6 already had — see _composer-strip.scss's
    // `.agent-composer-strip-row > .agent-composer-strip-stats-zone`), or
    // as its own dedicated line ABOVE the rows block otherwise
    // (user-directed, 2026-08-26 — "the center token count should float
    // up as the width is made narrow"; pre-Rev-7 tiers and the first
    // #2817 revision rendered this line BELOW the rows — see
    // `.agent-composer-strip > .agent-composer-strip-stats-zone`). The
    // wrapper span always renders (even with no stats yet) so this zone's
    // presence in the flow order stays stable whether or not stats are
    // populated yet. Deliberately NOT folded into the row-pairing
    // algorithm itself (spec §3.4) — a third, always-centered concern,
    // not a left/right slot; its real measured footprint feeds the
    // `layout` memo's `statsInline` decision in AgentComposerStrip.tsx (since 2026-08-26;
    // previously `computeComposerRows`'s now-removed `reservedWidth`
    // param, Codex P1 PR #2812), which picks between these two mount
    // positions so the shared line is never overflowed. `variant`
    // tags which of the two mutually-exclusive call sites in AgentComposerStrip.tsx is
    // mounting this instance, so the measurement effect above can tell
    // them apart (see that effect's own doc comment).
    const statsZone = (variant: "single" | "multi") => (
        <span class="agent-composer-strip-stats-zone" ref={(el) => (statsRefs[variant] = el)} />
    );

    return { statsWidth, statsZone };
}
