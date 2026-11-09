// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer strip's row layout: which slots go on which side of which row, from their real
 * measured widths. Pure functions, no DOM. The strip (AgentComposerStrip.tsx) measures and renders;
 * this decides. The history of the logic, Revs 4 to 9, follows.
 *
 * This is the 5th revision of this file's zone-balancing logic in two days
 * (2026-08-24/25) — see
 * docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md for the
 * full history, worth reading before changing this again. Two separate
 * bugs had to be found and fixed, at two different layers, and getting
 * one right didn't fix the other:
 *
 *   - Layer 1 (CSS, Rev 4): `_composer-strip.scss`'s widest tier forced
 *     BOTH zones to exactly equal width (`flex: 1 1 0`) regardless of how
 *     much content each one actually had, so a zone with less content
 *     showed real dead space in its half no matter how "balanced" the JS
 *     split was. Three earlier JS-side attempts (static assignment,
 *     count-based split, weight-balanced subset partition) all failed to
 *     fix this because none of them touched the actual constraint. Fixed
 *     by letting both zones size to their own content instead.
 *
 *   - Layer 2 (JS, Rev 5): fixing Layer 1 revealed a SECOND, independent
 *     issue: even with no forced dead space, the ORIGINAL side grouping
 *     (Rev 4: badge/auth/context-group on the right, only the runtime
 *     trigger on the left) put most of the strip's actual visual weight
 *     on one side in the common case — the context group alone can render
 *     3 sub-elements (text + countdown + Compact), more than the entire
 *     left zone. No amount of "don't force equal width" fixes a grouping
 *     that's just inherently lopsided by content. Fixed by moving the
 *     context group to the left (paired with the runtime trigger — both
 *     are "primary awareness" of the running agent), leaving badge/auth
 *     paired with NATIVE/CONTAINER+Shell on the right ("secondary status +
 *     the action button"). This is a FIXED regrouping chosen by counting
 *     realistic sub-element totals for the common case, not a per-render
 *     computed weight — simpler, and the earlier per-render weight
 *     computations (Rev 2/3) were themselves a source of bugs.
 *
 * The lesson, if this needs touching again: a visual-balance report needs
 * BOTH the actual rendered screenshot AND a check of which layer (CSS
 * width-forcing vs. JS content-grouping) is actually responsible before
 * changing either one — this file's history is what happens when that
 * diagnosis is skipped.
 *
 *   - Rev 6 (2026-08-26): even with both of the above fixed, the FIXED
 *     Rev 5 grouping still needed 2 lines in the single common case that
 *     matters most (Claude + context tracked + NATIVE + logged in) — the
 *     runtime trigger + full context group (3 sub-elements) together are
 *     wider than one line holds, while badge+auth+hostShell fit
 *     comfortably. No fixed pairing can be right for every combination
 *     of which slots happen to be present, because that depends on real
 *     content width, not a semantic label decided at design time — see
 *     docs/status/STATUS_COMPOSER_STRIP_ZONE_BALANCE_HANDOFF_2026_08_25.md
 *     for the full diagnosis this revision is the direct answer to.
 *     Replaced the fixed `side` grouping with `computeBalancedLeftKeys`
 *     (below) — a real DOM-measurement pass, not a guessed integer
 *     weight (that's what made Rev 2/3's earlier computed-balance
 *     attempts buggy): each slot's actual rendered width is measured via
 *     a `display: contents` ref wrapper (invisible to layout — see
 *     `.agent-composer-strip-slot-measure` in the SCSS) after every
 *     commit, and the split that minimizes left/right width difference
 *     wins the NEXT render. `hostShell` stays pinned right regardless
 *     (the strip's one action keeps a stable, predictable position); the
 *     `side` field on each slot in AgentComposerStrip.tsx still exists as the fallback used
 *     until the first real measurement lands (avoids an arbitrary/empty
 *     split on first paint) and in any environment with no real layout
 *     engine (e.g. this file's own unit tests run under JSDOM, which
 *     always reports 0-width elements).
 *
 *   - Rev 7 (2026-08-26): Rev 6's real-width measurement fixed the
 *     ≥482px single-line case, but the FIXED two-zone architecture below
 *     that width — `.agent-composer-strip-controls`/`-right`, each
 *     independently deciding when IT needed its own dedicated line —
 *     could produce a line that was 100% one zone's content, left- or
 *     right-justified, with the OTHER half of that exact line completely
 *     empty. Aggregate left/right totals looked balanced (Rev 6's own
 *     acceptance criteria); the actual per-LINE layout did not, because
 *     nothing in that architecture could ever make two independently-
 *     wrapping zones share a rendered line with each other. See
 *     docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md
 *     for why this took six revisions (including this file's own real-
 *     screenshot verification) to actually notice, and
 *     docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md for
 *     the full design. Replaced the two zones with an explicit list of
 *     ROWS (`computeComposerRows`, `rows()` in AgentComposerStrip.tsx) — every row has both a
 *     left and right occupant except one named, mathematically
 *     unavoidable exception (an odd total slot count leaves exactly one
 *     row unpaired) and the pre-existing fully-degenerate single-slot-
 *     total case. Needs the strip's own real available width to decide
 *     single-row vs. multi-row (`stripWidth`, a `ResizeObserver`) — a
 *     genuinely new capability; every prior revision relied purely on CSS
 *     `@container` queries for this decision.
 *
 *   - Rev 9 (2026-09-16, user-directed): the `auth` slot ("Logged in" /
 *     "Not logged in") and `ctx` slot (context text + Compact) disagreed
 *     with each other on which side to render, because the single-row
 *     balancer (`computeBalancedLeftKeys`) and the multi-row two-pointer
 *     pairing (`computeComposerRows` below) each decide their relative
 *     order independently, with no shared state — one balances them
 *     against `runtime`/`hostShell`'s fixed widths (incidentally landing
 *     auth left, ctx right in the common case), the other just sorts by
 *     raw width in isolation (ctx is reliably wider, so it always won
 *     "left" there instead). They'd swap sides right at whatever width
 *     the pane crossed from one regime to the other. Fixed by extending
 *     Rev 8's anchor mechanism to a SECOND pinned pair (`pinnedPairs`):
 *     `auth`/`ctx` are now excluded from both algorithms' free search
 *     exactly like `runtime`/`hostShell` already are, and always resolve
 *     to `auth=left, ctx=right` — in the single row by folding their
 *     widths into the existing fixed-width accumulators, in multi-row by
 *     reserving them out of the two-pointer pool and emitting their own
 *     row (ordered before the anchor row, so `hostShell` stays outermost/
 *     last regardless). Only engages when BOTH are actually present in a
 *     given render; either alone falls through to the ordinary free pool,
 *     unchanged. See
 *     docs/specs/SPEC_COMPOSER_STRIP_AUTH_COMPACT_SIDE_STABILITY_2026_09_16.md.
 *
 */

/**
 * Rev 6 of the zone-balancing logic — see the file-header comment's Rev
 * 4/5 history above this. Picks which of the MOVABLE slots (everything
 * except `hostShell`) render in the left zone vs. the right zone by
 * their REAL measured widths, minimizing the width difference between
 * the two — see docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md
 * Rev 6. `hostShell` is excluded from the search and always counted
 * toward the right side's width instead: "Shell always outermost," a
 * stable, predictable position for the strip's one real action, not
 * something that should jump sides just because a token count nudged
 * some OTHER slot's width by a few pixels. Brute-forces every subset of
 * the remaining slots (at most 4 in practice — runtime/badge/auth/ctx,
 * `2**4 = 16` combinations) — small enough that full enumeration is
 * simpler and more obviously correct than a cleverer search, which is
 * exactly where two earlier attempts at computed balance (a count-based
 * split, then a weight-guessed subset-partition search — see that
 * spec's Rev 2/Rev 3) introduced their own bugs. Exported for direct
 * unit testing without needing a real layout engine to produce widths.
 */
export function computeBalancedLeftKeys(
    movable: { key: string; width: number }[],
    fixedRightWidth: number,
    fixedLeftWidth = 0,
): Set<string> {
    const n = movable.length;
    const totalMovable = movable.reduce((sum, m) => sum + m.width, 0);
    let best = new Set<string>();
    let bestDiff = Infinity;
    for (let mask = 0; mask < 1 << n; mask++) {
        let leftWidth = 0;
        const leftKeys = new Set<string>();
        for (let i = 0; i < n; i++) {
            if (mask & (1 << i)) {
                leftWidth += movable[i].width;
                leftKeys.add(movable[i].key);
            }
        }
        // A completely empty left with movable slots available is never
        // "balanced" — that's the pre-existing "never a dead zone" rule,
        // applied by the caller (the `layout` memo in AgentComposerStrip.tsx) as a uniform override
        // regardless of which path (measured or fallback) produced the
        // split, so it's deliberately not special-cased again here.
        //
        // `fixedLeftWidth > 0` (the anchored model selector — see
        // `computeComposerRows`'s `anchorLeftKey`) means the left side is
        // already occupied by something this search can't move, so an
        // empty movable-left is a legitimate, genuinely-balanced answer
        // rather than a dead zone. Without this carve-out the anchor
        // would drag an arbitrary extra slot leftward purely to satisfy a
        // rule that exists to prevent emptiness the anchor already prevents.
        if (leftKeys.size === 0 && fixedLeftWidth === 0) continue;
        const rightWidth = fixedRightWidth + (totalMovable - leftWidth);
        const diff = Math.abs(fixedLeftWidth + leftWidth - rightWidth);
        if (diff < bestDiff) {
            bestDiff = diff;
            best = leftKeys;
        }
    }
    return best;
}

/** One rendered line of the composer strip's slot pool — see `computeComposerRows`. */
export interface ComposerRow {
    left: string[];
    right: string[];
}

type Slot = { key: string; width: number };
type Pair = [string | undefined, string | undefined];

/** The slots that are anchored this render: the model selector, the Shell, and the pinned pairs that have both members. */
interface Anchors {
    anchorLeft: Slot | undefined;
    hostShell: Slot | undefined;
    pinned: { left: Slot; right: Slot }[];
    pinnedKeys: Set<string>;
}

function resolveAnchors(
    slots: Slot[],
    hostShellKey: string,
    anchorLeftKey: string | undefined,
    pinnedPairs: { leftKey: string; rightKey: string }[]
): Anchors {
    const anchorLeft = anchorLeftKey ? slots.find((s) => s.key === anchorLeftKey) : undefined;
    const hostShell = slots.find((s) => s.key === hostShellKey);

    // Only pairs where BOTH members are present in this render activate —
    // see the "SECOND ANCHORED PAIR" doc comment on `computeComposerRows`.
    const pinned = pinnedPairs
        .map((p) => ({ left: slots.find((s) => s.key === p.leftKey), right: slots.find((s) => s.key === p.rightKey) }))
        .filter((p): p is { left: Slot; right: Slot } => Boolean(p.left && p.right));
    const pinnedKeys = new Set(pinned.flatMap((p) => [p.left.key, p.right.key]));
    return { anchorLeft, hostShell, pinned, pinnedKeys };
}

function singleRow(slots: Slot[], hostShellKey: string, { anchorLeft, hostShell, pinned, pinnedKeys }: Anchors): ComposerRow[] {
    // Single row: the anchors are the OUTERMOST occupant of their own
    // side rather than a whole reserved row (there is only one row —
    // "forget the top, it's just left and right respectively").
    // Pinned-pair members fold into the fixed bases alongside the
    // primary anchor, exactly like it already does — see the
    // "SECOND ANCHORED PAIR" doc comment.
    const pinnedLeftWidth = pinned.reduce((sum, p) => sum + p.left.width, 0);
    const pinnedRightWidth = pinned.reduce((sum, p) => sum + p.right.width, 0);
    const movable = slots.filter((s) => s.key !== hostShellKey && s.key !== anchorLeft?.key && !pinnedKeys.has(s.key));
    const leftKeys = computeBalancedLeftKeys(
        movable,
        (hostShell?.width ?? 0) + pinnedRightWidth,
        (anchorLeft?.width ?? 0) + pinnedLeftWidth,
    );
    return [
        {
            left: [
                ...(anchorLeft ? [anchorLeft.key] : []),
                ...pinned.map((p) => p.left.key),
                ...movable.filter((s) => leftKeys.has(s.key)).map((s) => s.key),
            ],
            right: [
                ...movable.filter((s) => !leftKeys.has(s.key)).map((s) => s.key),
                ...pinned.map((p) => p.right.key),
                ...(hostShell ? [hostShell.key] : []),
            ],
        },
    ];
}

/** The generic two-pointer walk over the pool, widest slot first; see "Per-pair capacity" in `computeComposerRows`'s doc comment. */
function pairPool(pool: Slot[], availableWidth: number, gapPx: number): Pair[] {
    const sorted = [...pool].sort((a, b) => b.width - a.width);
    const pairs: Pair[] = [];
    let i = 0;
    let j = sorted.length - 1;
    while (i < j) {
        if (sorted[i].width + sorted[j].width + gapPx <= availableWidth) {
            pairs.push([sorted[i].key, sorted[j].key]);
            i++;
            j--;
        } else {
            pairs.push([sorted[i].key, undefined]);
            i++;
        }
    }
    if (i === j) {
        pairs.push([sorted[i].key, undefined]);
    }
    return pairs;
}

// Pinned pairs' rows go after the generic two-pointer pairing,
// BEFORE the primary anchor row, so `hostShell` stays the
// outermost/last occupant regardless of how many pinned pairs exist.
// Never subject to the two-pointer sort (§ SECOND ANCHORED PAIR) — but
// STILL subject to the same per-pair capacity check the generic walk
// already applies (codex P1, PR #3282): an unconditional pair here
// would bypass that check, and since a real row is `flex-wrap`, an
// over-width pair renders as two one-sided PHYSICAL lines anyway —
// reproducing spec §1's one-sided-lines bug through a different
// mechanism, exactly what the generic check already exists to avoid.
// When it doesn't fit, split into two one-sided rows, preserving each
// member's own pinned side (left stays in the row's `left` array,
// right stays in `right`) rather than collapsing both to the generic
// walk's singleton convention (always `left`) — so ctx still reads as
// right-aligned even alone on its own line. A further instance of the
// already-named physical-capacity exception, not a new one.
function pinnedRows(pinned: Anchors["pinned"], availableWidth: number, gapPx: number): Pair[] {
    const pairs: Pair[] = [];
    for (const p of pinned) {
        if (p.left.width + p.right.width + gapPx <= availableWidth) {
            pairs.push([p.left.key, p.right.key]);
        } else {
            pairs.push([p.left.key, undefined]);
            pairs.push([undefined, p.right.key]);
        }
    }
    return pairs;
}

function multiRow(
    slots: Slot[],
    hostShellKey: string,
    availableWidth: number,
    gapPx: number,
    { anchorLeft, hostShell, pinned, pinnedKeys }: Anchors
): ComposerRow[] {
    // Multi-row. When BOTH anchors exist they are reserved out of the
    // pairing pool entirely and emitted as the final row, so neither one
    // travels between rows as the pane resizes (the whole point of the
    // constraint). Reserving exactly TWO slots preserves the parity of
    // what's left, so spec §1's "at most one singleton row" still holds
    // unchanged — pairing 2 fewer slots can't turn an even remainder odd.
    // Pinned pairs (§ SECOND ANCHORED PAIR) are reserved out the same way,
    // independent of whether the primary anchor pair is — each pinned
    // pair also reserves exactly two slots, so parity still holds.
    const anchorsReserved = Boolean(anchorLeft && hostShell);
    const pool = (
        anchorsReserved
            ? slots.filter((s) => s.key !== anchorLeft!.key && s.key !== hostShellKey)
            : slots
    ).filter((s) => !pinnedKeys.has(s.key));

    const pairs = pairPool(pool, availableWidth, gapPx);
    pairs.push(...pinnedRows(pinned, availableWidth, gapPx));

    if (anchorsReserved) {
        // The constraint row — ALWAYS exactly one row, never split, even
        // when the two anchors can't fit side by side at this width
        // (reagent P1, PR #2839). An earlier revision emitted them as two
        // adjacent one-sided rows in that case, borrowing the generic
        // physical-capacity exception (spec §1). That was wrong here for a
        // reason specific to the anchors: only the LAST row is hoisted out
        // of the render's `<For>` (see `precedingRowIndices`), so a split
        // put `runtime` in the second-to-last row — back inside the `<For>`
        // — and resizing across the split boundary destroyed and rebuilt
        // `AgentRuntimeDropup`, reintroducing the very open-state loss the
        // hoist exists to prevent, just at a different width.
        //
        // Keeping them one row is safe precisely because this pair is
        // degenerate: two items, one per side, nothing else competing. The
        // row's own `flex-wrap` (SCSS, the long-standing reagent-P1 #2393
        // safety net) then breaks them onto two VISUAL lines at narrow
        // widths — the identical rendering the split produced, reached
        // without ever moving either anchor between DOM subtrees. The
        // generic exception still applies to every other pair above, where
        // the wrap outcome genuinely is unpredictable.
        pairs.push([anchorLeft!.key, hostShell!.key]);
    } else {
        // No anchor pair to reserve (e.g. the runtime slot is absent
        // because controls are hidden) — fall back to the pre-anchor
        // behavior: reorient whichever pair `hostShell` landed in so it's
        // the RIGHT occupant, then move that pair to the end.
        const hostPairIdx = pairs.findIndex(([a, b]) => a === hostShellKey || b === hostShellKey);
        if (hostPairIdx !== -1) {
            let [a, b] = pairs[hostPairIdx];
            if (a === hostShellKey) {
                [a, b] = [b, a];
            }
            pairs.splice(hostPairIdx, 1);
            pairs.push([a, b]);
        }
    }

    const rows = pairs.map(([left, right]) => ({
        left: left ? [left] : [],
        right: right ? [right] : [],
    }));
    return mergeRows(rows, new Map(slots.map((s) => [s.key, s.width])), availableWidth, gapPx);
}

/**
 * Rev 10 (2026-11-09, #30203): rows that fit together on one line share it. The pairing above puts one slot a
 * side per row, so a narrow pane with seven small slots came out as three or four half-empty lines (the
 * operator's screenshot of a 420 px pane). This packs the rows, best fit, widest first, into as few lines as
 * hold them: a row's slots keep their own sides, so a merged line has the left slots of both on the left and the
 * right slots of both on the right. The LAST row (the anchors, or the one holding Shell) is the seed of its own
 * line and stays last, and no row is ever split, so the invariants of Revs 7 to 9 are untouched: the anchors
 * share the last line, a pinned pair stays together when it fits, nothing becomes one-sided that was not.
 */
function mergeRows(rows: ComposerRow[], widthOf: Map<string, number>, availableWidth: number, gapPx: number): ComposerRow[] {
    if (rows.length < 2) return rows;
    const widthOfRow = (r: ComposerRow): number => {
        const keys = [...r.left, ...r.right];
        return keys.reduce((sum, k) => sum + (widthOf.get(k) ?? 0), 0) + Math.max(0, keys.length - 1) * gapPx;
    };
    type Line = { first: number; left: string[]; right: string[]; width: number };
    const lineOf = (r: ComposerRow, first: number): Line => ({ first, left: [...r.left], right: [...r.right], width: widthOfRow(r) });
    const anchor = lineOf(rows[rows.length - 1], rows.length - 1);
    const lines: Line[] = [];
    const widest = rows
        .slice(0, -1)
        .map((r, i) => ({ r, i, width: widthOfRow(r) }))
        .sort((a, b) => b.width - a.width || a.i - b.i);
    for (const item of widest) {
        // The line with the least room left that still holds it (the anchor line included).
        let best: Line | undefined;
        for (const line of [...lines, anchor]) {
            const room = availableWidth - (line.width + gapPx + item.width);
            if (room >= 0 && (!best || room < availableWidth - (best.width + gapPx + item.width))) best = line;
        }
        const target = best ?? (lines[lines.push(lineOf({ left: [], right: [] }, item.i)) - 1]);
        const empty = target.left.length + target.right.length === 0;
        target.left.push(...item.r.left);
        // On the anchor line the right slots go BEFORE the anchors' own (Shell stays outermost, Rev 8); the
        // edge ordering keeps the order within the clickable ones.
        if (target === anchor) target.right.unshift(...item.r.right);
        else target.right.push(...item.r.right);
        target.width = empty ? item.width : target.width + gapPx + item.width;
        target.first = Math.min(target.first, item.i);
    }
    return [...lines.sort((a, b) => a.first - b.first), anchor].map(({ left, right }) => ({ left, right }));
}

/**
 * Rev 7 — see docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md
 * and docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md
 * for why this exists. Rev 6's `computeBalancedLeftKeys` picked a GLOBAL
 * left/right split, then let `.agent-composer-strip-controls`/`-right`
 * each independently wrap onto their OWN dedicated lines when they didn't
 * fit — which meant a line could be 100% one zone's content, left- or
 * right-justified, with the other half of that exact line completely
 * empty. Aggregate totals looked balanced; every individual LINE did not.
 *
 * This builds an explicit list of ROWS instead, each with its own left
 * and right occupant, so every line that exists has content on both
 * sides — the actual invariant (spec §1), not a proxy for it. Two paths:
 *
 *   - Everything fits within `availableWidth` on one line: delegate to
 *     `computeBalancedLeftKeys` for a single row — the already-verified
 *     ≥482px single-line behavior, untouched.
 *   - Otherwise: sort ALL slots (movable + hostShell) descending by
 *     width, then pair widest-with-narrowest by walking from both ends
 *     toward the middle — one row per pair. An odd total count leaves
 *     exactly one row unpaired (the one named, mathematically unavoidable
 *     exception — spec §1's "singleton exception"); nothing else is
 *     allowed to be one-sided. `hostShellKey` is reoriented to the RIGHT
 *     side of whichever pair it lands in and that pair is moved to the
 *     END of the row list — "Shell always outermost" (Rev 4/5/6),
 *     expressed here as "always the last row's right occupant."
 *
 * ANCHORED ELEMENTS (2026-08-29, user-directed — Rev 8). Two slots must
 * never travel as the pane resizes: the model selector (`anchorLeftKey`,
 * the `runtime` dropup) and the Shell toggle (`hostShellKey`). Both are
 * pinned to the BOTTOM — nearest the composer input:
 *
 *   - Multi-row: they are reserved OUT of the pairing pool and emitted as
 *     the final row, `{left: [anchor], right: [hostShell]}`. Reserving
 *     exactly two slots preserves the remainder's parity, so §1's "at most
 *     one singleton row" is unaffected.
 *   - Single row: there is no "bottom" to speak of, so the constraint
 *     degrades to its positional meaning — the anchor is the outermost
 *     LEFT occupant, hostShell the outermost RIGHT one, on the one row
 *     that exists.
 *   - The anchor row is NEVER split, even when the two can't fit side by
 *     side — the row's own `flex-wrap` breaks them onto two visual lines
 *     instead, which renders identically without moving either anchor
 *     between DOM subtrees (reagent P1, PR #2839). See the inline comment
 *     at the `anchorsReserved` branch for why this pair is exempt from
 *     the generic physical-capacity exception.
 *
 * This deliberately reverses the earlier "the model selector moving sides
 * is acceptable" call recorded in the retro's step 3 — it was dismissed
 * once as cosmetic and has now been made a hard constraint.
 *
 * SECOND ANCHORED PAIR — `pinnedPairs` (2026-09-16, user-directed — Rev
 * 9, see the file-header comment and
 * docs/specs/SPEC_COMPOSER_STRIP_AUTH_COMPACT_SIDE_STABILITY_2026_09_16.md).
 * `runtime`/`hostShell` are ONE hardcoded anchor pair; `pinnedPairs`
 * generalizes the same idea to any number of additional pairs that must
 * never disagree with themselves about which side each member renders on
 * — today just `auth`/`ctx` ("Logged in" and Compact swapped sides
 * exactly at the single-row/multi-row boundary, because the two regimes
 * decided their relative order by two different, uncoordinated rules).
 * Each pair activates only when BOTH members are present in `slots` — if
 * either is absent (its own visibility condition unmet), the other falls
 * through to the ordinary free pool untouched, same escape hatch the
 * `runtime`/`hostShell` anchor already has via `anchorsReserved`.
 * Multi-row: reserved out of the two-pointer pool, emitted as its own
 * row — ordered BEFORE the `runtime`/`hostShell` anchor row (never
 * after), so `hostShell` stays the outermost/last occupant regardless of
 * how many pinned pairs exist. UNLIKE the primary anchor row, a pinned
 * pair's row IS still subject to the per-pair capacity check the generic
 * two-pointer walk already applies — splitting into two one-sided rows
 * (each member keeping its own pinned side) rather than emitting an
 * unconditional pair that `flex-wrap` would silently break onto two
 * physical lines anyway (codex P1, PR #3282; the primary anchor pair's
 * own capacity exemption is narrower than it looks — see its own comment
 * below for why THAT one specifically must stay unsplit). Single row: both members' widths fold
 * into the existing `fixedLeftWidth`/`fixedRightWidth` accumulators
 * instead of participating in `computeBalancedLeftKeys`'s brute-force
 * search — the same tradeoff Rev 8 already accepted for the first anchor
 * pair (less flexibility for the TRULY free remainder to balance totals,
 * in exchange for these specific elements never moving).
 *
 * Deliberately a sort + two-pointer walk, not a search — the smallest
 * mechanism that can satisfy the invariant, matching this file's own
 * repeated lesson (SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md's
 * Rev 2/3 postmortem: cleverer search algorithms are where this file's
 * bugs have historically come from). Exported for direct unit testing
 * without needing a real layout engine to produce widths.
 *
 * No `reservedWidth` param anymore (2026-08-26, supersedes Codex P1, PR
 * #2812): row membership is decided from slot widths alone. Whether the
 * stats zone SHARES the single row is the component's own separate
 * `statsInline` decision — reserving stats space inside THIS fit check
 * made a too-wide stats zone split the SLOTS, jumping from 1 visual
 * line straight to 3 (2 slot rows + the stats' own line) and skipping
 * the strictly-better middle tier of "slots on one line, stats evicted
 * to their own line above the rows" (2 lines). The overflow Codex P1
 * guarded against cannot recur: when slots-plus-stats don't fit, the
 * stats leave the row entirely instead of overflowing it.
 *
 * Per-pair capacity (Codex P1, PR #2812): the two-pointer walk used to
 * pair `sorted[i]`/`sorted[j]` unconditionally, even when their combined
 * width didn't actually fit `availableWidth` — the resulting row object
 * had both sides "filled" by the data, but the real rendered line still
 * overflowed onto two physical lines via the row's own `flex-wrap`,
 * reproducing the one-sided-lines bug through a different mechanism.
 * `sorted[j]` is always the smallest still-unplaced width at that point
 * (descending sort, closing in from the end) — if it doesn't fit next to
 * the widest remaining slot, no other remaining slot (all ≥ its width)
 * fits any better, so the widest slot is emitted as its own one-sided row
 * instead of forcing an overflowing pair onto one line. This is a THIRD,
 * physical-capacity exception to spec §1's invariant, alongside the
 * already-named odd-count and degenerate cases — genuinely unavoidable
 * when content simply cannot fit two-up in the available width, not a
 * gap in this function's own logic.
 */
export function computeComposerRows(
    slots: { key: string; width: number }[],
    hostShellKey: string,
    availableWidth: number,
    gapPx: number,
    anchorLeftKey?: string,
    pinnedPairs: { leftKey: string; rightKey: string }[] = [],
): ComposerRow[] {
    if (slots.length === 0) return [];

    const anchors = resolveAnchors(slots, hostShellKey, anchorLeftKey, pinnedPairs);

    const totalWidth = slots.reduce((sum, s) => sum + s.width, 0) + Math.max(0, slots.length - 1) * gapPx;
    return totalWidth <= availableWidth
        ? singleRow(slots, hostShellKey, anchors)
        : multiRow(slots, hostShellKey, availableWidth, gapPx, anchors);
}

/**
 * Edge priority for interactive elements (2026-08-26, user-directed
 * follow-up to Rev 7): on every rendered line, interactive slots
 * (buttons/dropdowns — things you CLICK) sit flush against the strip's
 * outer edges; passive/informational slots (auth status, ctx text) sit
 * inward. A stable partition, not a sort — relative order within the
 * interactive and passive groups is preserved, so this can never fight
 * the pool's own ordering rules (e.g. "Shell always outermost" survives
 * because hostShell is both interactive AND last in pool order).
 *
 * Ordering only — row membership and widths are decided upstream by
 * `computeComposerRows`, so this cannot affect the §1 no-one-sided-rows
 * invariant or any fit/pairing decision. The matching second half of
 * the constraint (a composite slot's own internal order — ctx's Compact
 * button, hostShell's Shell button — mirroring to the outer end of
 * whichever side it renders on) lives in those slots' `render(rowSide)`
 * callbacks, not here.
 */
export function orderKeysForEdgePriority(
    keys: string[],
    side: "left" | "right",
    isInteractive: (key: string) => boolean,
): string[] {
    const interactive = keys.filter((k) => isInteractive(k));
    const passive = keys.filter((k) => !isInteractive(k));
    return side === "left" ? [...interactive, ...passive] : [...passive, ...interactive];
}

/**
 * Whether the stats zone shares the single row's line (2026-08-26,
 * extracted from the `layout` memo per ReAgent P2 on PR #2817 so the
 * decision has a pure-function test guard — this exact stats-width math
 * has regressed twice before: Codex P1 #2812, the post-#2813 wrapper
 * trap). True only when there is one slot row AND either no stats exist
 * or slots-plus-stats-plus-gap genuinely fit the available width. False
 * evicts the stats to their own line (above the rows) WITHOUT splitting the slot
 * row — the middle tier (1 line → 2 → 3) that stops the strip jumping
 * from 1 visual line straight to 3.
 */
export function computeStatsInline(
    rowCount: number,
    slotsTotalWidth: number,
    statsWidth: number,
    gapPx: number,
    availableWidth: number,
): boolean {
    return rowCount === 1 && (statsWidth === 0 || slotsTotalWidth + statsWidth + gapPx <= availableWidth);
}
