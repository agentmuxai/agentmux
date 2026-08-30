// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentComposerStrip — status row that sits directly above the textarea in
 * the agent pane composer region. As of Rev 7 (2026-08-26): a stats zone
 * (center, always true-centered) plus a list of ROWS, each with its own
 * left-anchored and right-anchored half — see `computeComposerRows` and
 * docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md. The strip
 * grows from 1 row up to however many it actually needs as the pane
 * narrows, based on real rendered slot widths, not a guessed magic number
 * — but (unlike Rev 1-6) deciding HOW MANY rows are needed now also needs
 * the strip's own real available width (a `ResizeObserver`, see `stripWidth`
 * below), not purely CSS `@container` queries — see Rev 7's own paragraph
 * further down for why.
 *
 * Misc elements (everything except the centered stats zone) are pooled
 * into an ordered list of "slots" (see `slots` below), each carrying a
 * fixed semantic `side` — left is "what agent, what mode, how much context
 * is left" (the runtime trigger + the context group); right is "status
 * indicators + the one real action" (process badge, auth tag, then
 * HOST/SANDBOX+Shell, Shell always outermost) — used ONLY as the fallback
 * ordering when real measurement isn't available yet (see Rev 6/7's own
 * paragraphs). Once real widths ARE available, `rows()` computes the
 * actual left/right split and row grouping fresh, which can and does
 * override this fixed `side` pairing when real widths call for it.
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
 *     paired with HOST/SANDBOX+Shell on the right ("secondary status +
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
 *     matters most (Claude + context tracked + HOST + logged in) — the
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
 *     `side` field on each slot below still exists as the fallback used
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
 *     ROWS (`computeComposerRows`, `rows()` below) — every row has both a
 *     left and right occupant except one named, mathematically
 *     unavoidable exception (an odd total slot count leaves exactly one
 *     row unpaired) and the pre-existing fully-degenerate single-slot-
 *     total case. Needs the strip's own real available width to decide
 *     single-row vs. multi-row (`stripWidth`, a `ResizeObserver`) — a
 *     genuinely new capability; every prior revision relied purely on CSS
 *     `@container` queries for this decision.
 *
 * Stats zone (center, unaffected by the above): tokens (↑in ↓out) ·
 * elapsed.
 *
 * The strip bar itself is not clickable. "Shell" is the sole toggle for
 * the details drawer (the AgentShellSubblock terminal — activity-log lines
 * write directly into it rather than a separate panel, see agent-view.tsx's
 * `log`/`handleShellTermReady`. SPEC_AGENT_SHELL_XTERM_TERMINAL_2026_07_03.md).
 * Mode/Model/Effort used to be
 * three separate FlyoutMenu drop-up pills here (SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02
 * Fix 7); they're now consolidated into one AgentRuntimeDropup trigger + panel
 * — see docs/specs/SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md.
 */

import { useTick } from "@/app/hook/useTick";
import { compactionThreshold } from "@/app/store/agent-pane-state/context-window";
import type { CompactionState } from "@/app/store/agent-pane-state/types";
import { formatCompactNumber, formatExactNumber } from "@/util/format-count";
import { formatElapsedCompact } from "@/util/format-time";
import { For, Show, createEffect, createMemo, createSignal, onCleanup, onMount, untrack, type JSX } from "solid-js";
import type { SessionStats, TurnTokens } from "../types";
import { AgentRuntimeDropup } from "./AgentRuntimeDropup";
import { RuntimeBadge } from "./RuntimeBadge";

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
        // applied by the caller (zones() below) as a uniform override
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
 *   - If the two anchors cannot physically share a line, the same
 *     physical-capacity exception that governs any other pair applies:
 *     two adjacent one-sided rows, still bottom-most, rather than an
 *     overflowing row that `flex-wrap` would split anyway.
 *
 * This deliberately reverses the earlier "the model selector moving sides
 * is acceptable" call recorded in the retro's step 3 — it was dismissed
 * once as cosmetic and has now been made a hard constraint.
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
): ComposerRow[] {
    if (slots.length === 0) return [];

    const anchorLeft = anchorLeftKey ? slots.find((s) => s.key === anchorLeftKey) : undefined;
    const hostShell = slots.find((s) => s.key === hostShellKey);

    const totalWidth = slots.reduce((sum, s) => sum + s.width, 0) + Math.max(0, slots.length - 1) * gapPx;
    if (totalWidth <= availableWidth) {
        // Single row: the anchors are the OUTERMOST occupant of their own
        // side rather than a whole reserved row (there is only one row —
        // "forget the top, it's just left and right respectively").
        const movable = slots.filter((s) => s.key !== hostShellKey && s.key !== anchorLeft?.key);
        const leftKeys = computeBalancedLeftKeys(movable, hostShell?.width ?? 0, anchorLeft?.width ?? 0);
        return [
            {
                left: [
                    ...(anchorLeft ? [anchorLeft.key] : []),
                    ...movable.filter((s) => leftKeys.has(s.key)).map((s) => s.key),
                ],
                right: [
                    ...movable.filter((s) => !leftKeys.has(s.key)).map((s) => s.key),
                    ...(hostShell ? [hostShell.key] : []),
                ],
            },
        ];
    }

    // Multi-row. When BOTH anchors exist they are reserved out of the
    // pairing pool entirely and emitted as the final row, so neither one
    // travels between rows as the pane resizes (the whole point of the
    // constraint). Reserving exactly TWO slots preserves the parity of
    // what's left, so spec §1's "at most one singleton row" still holds
    // unchanged — pairing 2 fewer slots can't turn an even remainder odd.
    const anchorsReserved = Boolean(anchorLeft && hostShell);
    const pool = anchorsReserved
        ? slots.filter((s) => s.key !== anchorLeft!.key && s.key !== hostShellKey)
        : slots;

    const sorted = [...pool].sort((a, b) => b.width - a.width);
    const pairs: [string | undefined, string | undefined][] = [];
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

    if (anchorsReserved) {
        // The constraint row. If the two anchors genuinely cannot share a
        // line at this width, the physical-capacity exception (spec §1)
        // applies exactly as it does to any other pair — emit them as two
        // adjacent one-sided rows rather than forcing an overflow that the
        // row's own `flex-wrap` would silently split anyway (which is the
        // one-sided-lines bug this file exists to prevent, reintroduced by
        // a different route). They stay bottom-most and adjacent either
        // way, so neither anchor travels; only their sharing of one line
        // degrades.
        if (anchorLeft!.width + hostShell!.width + gapPx <= availableWidth) {
            pairs.push([anchorLeft!.key, hostShell!.key]);
        } else {
            pairs.push([anchorLeft!.key, undefined]);
            pairs.push([undefined, hostShell!.key]);
        }
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

    return pairs.map(([left, right]) => ({
        left: left ? [left] : [],
        right: right ? [right] : [],
    }));
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

// ── Helpers ────────────────────────────────────────────────────────────────

function fmtTokens(t: TurnTokens): string {
    return `↑${formatCompactNumber(t.input)} ↓${formatCompactNumber(t.output)}`;
}

type CtxBand = "low" | "mid" | "high" | "critical";

function ctxBand(tokens: number, contextWindow: number): CtxBand {
    const fraction = tokens / compactionThreshold(contextWindow);
    if (fraction >= 0.9) return "critical";
    if (fraction >= 0.75) return "high";
    if (fraction >= 0.5) return "mid";
    return "low";
}

function contextTitle(tokens: number, contextWindow: number | undefined): string {
    if (contextWindow == null) {
        return `Context: ${formatExactNumber(tokens)} tokens`;
    }
    const pct = ((tokens / contextWindow) * 100).toFixed(1);
    const remaining = Math.max(0, compactionThreshold(contextWindow) - tokens);
    return (
        `Context window: ${formatExactNumber(tokens)} / ${formatExactNumber(contextWindow)} tokens (${pct}%)\n` +
        `This is the total conversation history sent to the model on each turn.\n` +
        `Auto-compacts around ${formatExactNumber(compactionThreshold(contextWindow))} tokens ` +
        `(≈${formatExactNumber(remaining)} tokens left).\n` +
        `Applies to auto-compaction only — a manual /compact can happen at any fill level.`
    );
}

/**
 * Tier 3 of docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md
 * §7 — explicit countdown language, surfaced inline (not hover-gated) once
 * the fill level is worth calling out (mid band and above). Predicts only
 * the `auto` trigger: `compactionThreshold(window) − tokens` is the
 * distance to the CLI's own auto-compact point, computable every turn
 * (Tier 0's groundwork) — a manual `/compact` can happen at any fill level
 * and is fundamentally unpredictable, so this must never be read as "the"
 * time compaction will happen, only as "no sooner than."
 */
function compactionCountdownText(tokens: number, window: number): string | null {
    const band = ctxBand(tokens, window);
    if (band === "low") return null;
    const remaining = Math.max(0, compactionThreshold(window) - tokens);
    return `~${formatCompactNumber(remaining)} to auto-compact`;
}

// ── Props ──────────────────────────────────────────────────────────────────

interface AgentComposerStripProps {
    /** True while a turn is in flight. */
    loading?: boolean;
    /**
     * Cumulative cost/tokens/duration across every completed turn in this
     * pane's lifetime; non-null after the first TurnEnd. Sums, rather than
     * replaces, on each turn — see SPEC_AGENT_SESSION_COST_TOTALS_2026_07_02.md.
     */
    sessionTotals?: SessionStats | null;
    /** Live tokens for the in-flight turn. */
    turnTokens?: TurnTokens | null;
    /** Count of OS processes tracked for this agent block. */
    processCount?: number;
    /** Fires when the user clicks the ⚙N process badge. */
    onProcessBadgeClick?: () => void;
    /** Reducer-projected: activity log panel open/closed. */
    logOpen: boolean;
    /** Dispatches `DetailsToggle` to the pane reducer. */
    onToggleLog: () => void;
    /** Current context fill in tokens (from message_start). */
    contextTokens?: number | null;
    /** Provider's max context window size. undefined = unknown. */
    contextWindow?: number;
    /** Durable logged-in/out state (useAgentControllerStatus's authStatus) —
     *  rendered as a small red/green tag right of the context text. Hidden
     *  entirely for "unknown" (before the first auth check resolves), so the
     *  strip doesn't flash a wrong color for an instant on every mount. */
    authStatus?: "authenticated" | "unauthenticated" | "unknown";

    // ── Inline model / effort controls ────────────────────────────
    /** Block id — needed for applyRuntimeChange. */
    blockId?: string;
    /** Block atom — reads current model/effort from meta. */
    blockAtom?: () => Block | undefined;
    /** Provider id — needed for applyRuntimeChange. */
    providerId?: string;
    /** `block.meta["agentMode"]` — "host" or "container". Drives the compact
     *  HOST/SANDBOX tag next to the model selector (replaces the old,
     *  confirmed-inert "Host — full system access" / "Container — isolated
     *  Docker sandbox" pane row — see
     *  docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md §3.2). */
    agentMode?: string;
    /**
     * Live "compaction in progress" state (reducer-owned, set by the
     * `PreCompact` hook's `compaction_started` signal). Gates the manual
     * Compact button (`canCompact()` below) and its tooltip. The
     * "Compacting…" + live elapsed readout this used to also drive here
     * moved to `AgentWorkingRow` (2026-08-27, Part 2 of
     * SPEC_REMOVE_AGENT_UNRESPONSIVE_DETECTION_2026_08_25.md) — see
     * `agent-view.tsx`'s `<AgentWorkingRow compacting=.../>` call site.
     */
    compacting?: CompactionState | null;
    /**
     * Manually trigger compaction now, instead of waiting for the CLI's
     * own auto-compact threshold. Only meaningful for Claude — sends the
     * literal text "/compact" through the normal send-message path
     * (agent-view.tsx's handleSendMessage), which the CLI's persistent
     * stream-json stdin protocol recognizes as its real /compact command
     * (verified empirically against a live `claude` process — see
     * docs/reports/REPORT_TOKEN_ACCOUNTING_AND_COMPACTION_CONTROL_2026_08_18.md
     * §6). Deliberately NOT routed through the SlashCommand registry
     * (commands/providers/claude.ts) — a REGISTERED command's handler
     * returning `{kind:"passthrough"}` is a documented no-op in
     * dispatch.ts's `formatResult` ("passthrough from a handler is a
     * noop here... ignored"), so registering `/compact` there would
     * actually swallow it instead of forwarding it. Leaving it
     * unregistered means dispatchSlashCommand's own top-level
     * unknown-command path returns passthrough BEFORE calling any
     * handler, which useAgentCommands.sendMessage correctly forwards
     * as a real turn — the same path a user manually typing "/compact"
     * already takes today.
     */
    onCompact?: () => void;
}

// ── Component ──────────────────────────────────────────────────────────────

export const AgentComposerStrip = (props: AgentComposerStripProps): JSX.Element => {
    const tick = useTick(1000);
    const [loadStartMs, setLoadStartMs] = createSignal<number | null>(null);
    createEffect(() => {
        if (props.loading) {
            setLoadStartMs((prev) => prev ?? Date.now());
        } else {
            setLoadStartMs(null);
        }
    });
    const elapsedMs = createMemo(() => {
        const s = loadStartMs();
        return s != null ? (tick(), Date.now() - s) : 0;
    });

    const rightText = createMemo((): string => {
        const parts: string[] = [];
        if (props.loading) {
            if (props.turnTokens) parts.push(fmtTokens(props.turnTokens));
            parts.push(formatElapsedCompact(elapsedMs()));
        } else if (props.sessionTotals) {
            const s = props.sessionTotals;
            if (s.input_tokens != null || s.output_tokens != null) {
                parts.push(fmtTokens({ input: s.input_tokens ?? 0, output: s.output_tokens ?? 0 }));
            }
            if (s.duration_ms != null) {
                parts.push(formatElapsedCompact(s.duration_ms));
            }
        }
        return parts.join("  ·  ");
    });

    // Show model/effort controls only for Claude agents (controls are claude-specific;
    // non-claude providers (codex/gemini/kimi) have different model enumerations and
    // buildRuntimeArgs silently drops effort for them — spec §1.3).
    const showControls = () => props.blockAtom != null && props.providerId === "claude";

    // Context text color based on proximity to compaction threshold.
    const ctxClass = (): string => {
        const t = props.contextTokens;
        const w = props.contextWindow;
        if (t == null || t <= 0 || w == null) return "";
        const b = ctxBand(t, w);
        return `agent-composer-strip-ctx--${b}`;
    };

    const ctxText = (): string | null => {
        const t = props.contextTokens;
        const w = props.contextWindow;
        if (t == null || t <= 0) return null;
        if (w == null) return `${formatCompactNumber(t)} ctx`;
        return `${formatCompactNumber(t)} / ${formatCompactNumber(w)}`;
    };

    // Tier 3 — explicit countdown, visible inline (not hover-gated) once
    // the fill level is worth calling out. null below the mid band, or
    // whenever the window itself is unknown (nothing to count down from).
    //
    // Gated to Claude only (Codex P2 on PR #2729): `compactionThreshold()`
    // hard-codes Claude Code's own ~33K auto-compact buffer
    // (context-window.ts's own doc comment). A non-Claude provider that
    // happens to report Claude-shaped `message_start` usage (e.g. a
    // `muxcode`-catalog entry) would otherwise get an invented countdown
    // against a threshold that has never been verified for it — the same
    // reason `canCompact()` below already restricts the manual-compact
    // button to Claude.
    const ctxCountdownText = (): string | null => {
        const t = props.contextTokens;
        const w = props.contextWindow;
        if (t == null || t <= 0 || w == null) return null;
        if (props.providerId !== "claude") return null;
        return compactionCountdownText(t, w);
    };

    // Only offer manual compaction for Claude (the only provider this has
    // been verified against — see onCompact's doc comment), only once
    // there's something to compact, and not while a turn is already in
    // flight or a compaction is already running.
    const canCompact = (): boolean =>
        props.providerId === "claude"
        && props.onCompact != null
        && ctxText() != null
        && !props.loading
        && !props.compacting;

    // Dynamic left/right pooling — see the file-header comment for why a
    // static per-item zone assignment left the controls zone empty for
    // non-Claude/no-process/unknown-auth panes. Each slot's `render`
    // reproduces exactly what that element rendered under its old fixed
    // zone (same classes/handlers/titles) — only WHICH zone renders it,
    // computed fresh here every time, changed. `side` is a fixed semantic
    // preference (left = state/config, right = counters + the one real
    // action) — see `zones` below for how the "left must never be
    // completely empty" override works, and
    // docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md Rev 4
    // for why this replaced two earlier attempts at balancing by a
    // computed integer "weight" (count-based, then weight-based subset
    // partition) — both were solving the wrong layer: the actual dead-
    // space bug was the CSS forcing both zones to equal width regardless
    // of content (see _composer-strip.scss), not which slot goes where.
    // `interactive` — feeds the edge-priority ordering (see
    // `orderKeysForEdgePriority`): true for slots whose primary content
    // is something you CLICK (buttons/dropdowns), false for passive
    // status/info. `render` receives the side of the row it is being
    // mounted into, so composite slots (ctx, hostShell) can mirror their
    // own internal order to keep their interactive element on the outer
    // end — the side is fixed for the lifetime of one render() call
    // (a slot changing sides leaves one <For> and enters the other,
    // re-invoking render with the new side), so it is deliberately a
    // plain parameter, not a reactive read.
    const slots = createMemo((): { key: string; side: "left" | "right"; interactive: boolean; render: (rowSide: "left" | "right") => JSX.Element }[] => {
        const out: { key: string; side: "left" | "right"; interactive: boolean; render: (rowSide: "left" | "right") => JSX.Element }[] = [];

        if (showControls()) {
            out.push({
                key: "runtime",
                side: "left",
                interactive: true,
                render: () => (
                    <AgentRuntimeDropup
                        blockId={props.blockId ?? ""}
                        blockAtom={props.blockAtom ?? (() => undefined)}
                        providerId={props.providerId ?? ""}
                    />
                ),
            });
        }

        if ((props.processCount ?? 0) > 0) {
            out.push({
                key: "badge",
                side: "right",
                interactive: true,
                render: () => (
                    <button
                        type="button"
                        class="agent-composer-strip-process-badge"
                        data-strip-button
                        title={`${props.processCount} tracked ${props.processCount === 1 ? "process" : "processes"} — click to open swarm`}
                        onClick={() => props.onProcessBadgeClick?.()}
                    >
                        <span aria-hidden="true">⚙</span>
                        <span>{props.processCount}</span>
                    </button>
                ),
            });
        }

        if (props.authStatus === "authenticated" || props.authStatus === "unauthenticated") {
            out.push({
                key: "auth",
                side: "right",
                interactive: false,
                render: () => (
                    <span
                        class="agent-composer-strip-auth"
                        classList={{
                            "agent-composer-strip-auth--ok": props.authStatus === "authenticated",
                            "agent-composer-strip-auth--bad": props.authStatus === "unauthenticated",
                        }}
                        title={
                            props.authStatus === "authenticated"
                                ? "Signed in to this agent's provider"
                                : "Not signed in — click Log in to continue"
                        }
                    >
                        <span class="agent-composer-strip-auth-dot" aria-hidden="true" />
                        {props.authStatus === "authenticated" ? "Logged in" : "Not logged in"}
                    </span>
                ),
            });
        }

        if (ctxText() != null) {
            out.push({
                key: "ctx",
                // ctx text + countdown (conditional) + Compact button
                // (conditional) render together as one unit — this slot
                // can't be split across zones, since Compact must stay
                // immediately adjacent to the context text (on the OUTER
                // side of it — see render(rowSide) below). Grouped with the
                // runtime trigger on the LEFT (2026-08-25, Rev 5) — this
                // slot alone can render 3 sub-elements, more than
                // everything else in the pool combined in the common
                // case, so pairing it with badge/auth/hostShell on the
                // right (as Rev 4 first shipped) put most of the strip's
                // visual weight on one side every time a Claude agent had
                // context tracking active. "Mode + context awareness"
                // (left) / "status indicators + the action button"
                // (right) is the resulting split — see
                // docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md
                // Rev 5.
                side: "left",
                // Interactive exactly when the Compact button actually
                // renders (same gate as its <Show> below) — a pure
                // ctx-text slot has nothing clickable and should sit
                // inward like any other passive content.
                interactive: props.providerId === "claude" && props.onCompact != null,
                // Compact sits on the OUTER end of whichever side this
                // slot renders on (edge priority): first on the left,
                // last on the right. The text + countdown stay adjacent
                // in reading order either way.
                render: (rowSide) => {
                    const compactBtn = () => (
                        <Show when={props.providerId === "claude" && props.onCompact != null}>
                            <button
                                type="button"
                                class="agent-composer-strip-compact-btn"
                                data-strip-button
                                disabled={!canCompact()}
                                title={
                                    canCompact()
                                        ? "Summarize and trim this session's history now, instead of waiting for the CLI's own auto-compact threshold."
                                        : props.compacting
                                            ? "Already compacting…"
                                            : "Wait for the current turn to finish before compacting."
                                }
                                onClick={() => props.onCompact?.()}
                            >
                                Compact
                            </button>
                        </Show>
                    );
                    return (
                        <>
                            {rowSide === "left" && compactBtn()}
                            <span
                                class={`agent-composer-strip-ctx ${ctxClass()}`}
                                title={
                                    props.contextTokens != null
                                        ? contextTitle(props.contextTokens, props.contextWindow)
                                        : undefined
                                }
                            >
                                {ctxText()}
                            </span>
                            <Show when={ctxCountdownText()}>
                                <span
                                    class={`agent-composer-strip-ctx-countdown ${ctxClass()}`}
                                    classList={{ "agent-composer-strip-ctx-countdown--critical": ctxClass().endsWith("critical") }}
                                    title="Applies to auto-compaction only — a manual /compact can happen at any fill level."
                                >
                                    {ctxCountdownText()}
                                </span>
                            </Show>
                            {rowSide === "right" && compactBtn()}
                        </>
                    );
                },
            });
        }

        // Always present (Shell has no visibility gate) and always last
        // in pool order — see `zones` below for why that keeps it on the
        // right except in the fully degenerate one-slot-total case.
        out.push({
            key: "hostShell",
            side: "right",
            interactive: true,
            // Shell (the button) on the OUTER end, HOST badge (passive)
            // inward — mirrored by side the same way ctx's Compact is.
            // On the right this is the existing "Shell always outermost"
            // order, unchanged; the flip only shows in the degenerate
            // case where this slot is the line's sole (left) occupant.
            //
            // No wrapper span (element-level edge priority, 2026-08-26):
            // badge and button render as direct siblings in the slot's
            // measure wrapper — same shape as the ctx slot's elements —
            // so the SCSS edge-priority `order` rules can place the
            // passive badge inward past a NEIGHBORING slot's interactive
            // element (Compact), which a single wrapping flex item never
            // allowed. The user explicitly chose this over the older
            // "HOST just left of Shell" pairing when the two directives
            // collided (see _composer-strip.scss's removal note).
            render: (rowSide) => {
                const hostBadge = () => (
                    <Show when={props.agentMode === "host" || props.agentMode === "container"}>
                        <RuntimeBadge runtime={props.agentMode!} size="tag" />
                    </Show>
                );
                const shellBtn = () => (
                    <button
                        type="button"
                        class="agent-composer-strip-log-btn"
                        classList={{ "agent-composer-strip-log-btn--active": props.logOpen }}
                        title={props.logOpen ? "Hide the shell" : "Show the shell"}
                        onClick={() => props.onToggleLog()}
                    >
                        Shell
                    </button>
                );
                return (
                    <>
                        {rowSide === "left" && shellBtn()}
                        {hostBadge()}
                        {rowSide === "right" && shellBtn()}
                    </>
                );
            },
        });

        return out;
    });

    // Rev 7 — the strip's own real available width, needed to decide
    // single-row vs. multi-row (see `rows` below and
    // docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md §3.3).
    // A genuinely new capability this file didn't need before Rev 7 —
    // every prior revision relied purely on CSS `@container` queries.
    // Guarded (`typeof ResizeObserver === "undefined"`) the same way
    // every other ResizeObserver use in this codebase is (JSDOM, this
    // file's own unit-test environment, has no ResizeObserver at all) —
    // `rows()` below already falls back to a single fixed-side row
    // whenever `stripWidth()` is still 0, so skipping the observer
    // entirely in that environment is correct, not just tolerated.
    //
    // Observes `.agent-composer-strip-rows` — NOT the outer
    // `.agent-composer-strip` (see `stripRef` below) — for two reasons,
    // both found via real dev-build screenshot verification at the narrow
    // tier (task #48), not by inspection alone:
    //
    //   - `entry.contentRect` is reported in the element's own LOCAL CSS
    //     pixels (unaffected by any CSS `zoom` on an ancestor — e.g. the
    //     agent view's own `zoom: 0.8`), while the per-slot widths
    //     measured below use `getBoundingClientRect()`, which IS
    //     zoom-scaled (viewport-relative). Comparing those two units
    //     directly made the "does everything fit on one line" check
    //     systematically over-generous under any non-1 ancestor zoom.
    //     Reading `getBoundingClientRect()` off the observed element
    //     itself instead keeps both sides of that comparison in the same
    //     coordinate space regardless of ancestor zoom.
    //   - `.agent-composer-strip`'s own `getBoundingClientRect().width` is
    //     its BORDER-BOX width, which includes its own horizontal padding
    //     (`padding: var(--space-1) var(--space-2)`, 16px total) — slot
    //     widths have no padding component, so comparing them against
    //     that inflated width overstated how much fits on one line the
    //     same way the zoom mismatch above did (reagent P1, PR #2812).
    //     `.agent-composer-strip-rows` has no padding/border of its own,
    //     so its border-box and content-box widths are identical — its
    //     rect gives the real available content width directly, still
    //     via `getBoundingClientRect()` for the zoom-consistency reason
    //     above.
    let stripRef: HTMLDivElement | undefined;
    let rowsRef: HTMLDivElement | undefined;
    const [stripWidth, setStripWidth] = createSignal(0);
    onMount(() => {
        if (!rowsRef || typeof ResizeObserver === "undefined") return;
        const ro = new ResizeObserver((entries) => {
            for (const entry of entries) {
                setStripWidth(entry.target.getBoundingClientRect().width);
            }
        });
        ro.observe(rowsRef);
        onCleanup(() => ro.disconnect());
    });

    // A `getComputedStyle(...)`-read length (e.g. a `column-gap` or CSS
    // custom property) is reported in the element's own LOCAL CSS pixels,
    // unaffected by an ancestor's CSS `zoom` — the same property that made
    // `entry.contentRect` wrong above, for the same reason. Every width in
    // this file that feeds a fit/pairing decision is measured via
    // `getBoundingClientRect()` instead, which IS zoom-scaled. A raw
    // `getComputedStyle` gap value mixed directly into one of those totals
    // re-introduces the exact zoom-unit mismatch this file's own Rev 7
    // fix eliminated for width — just via the gap term instead (found via
    // a real regression report after PR #2812 shipped: the widest tier
    // started splitting into 2 lines, and the narrow tier grew MORE
    // one-sided rows than necessary, both explained by systematically
    // overestimating how much space gaps need). `zoom` applies uniformly
    // to a whole subtree, so the ratio measured off ANY element inside it
    // (here, the always-mounted `stripRef`) is the correct multiplier for
    // a `getComputedStyle` length read anywhere else in that same
    // subtree — multiply by this wherever the two measurement styles mix.
    const zoomRatio = (): number => {
        if (!stripRef || !stripRef.clientWidth) return 1;
        return stripRef.getBoundingClientRect().width / stripRef.clientWidth;
    };

    // Rev 6 — real measured widths per slot, keyed by slot.key. Populated
    // by the effect below from each slot's `display: contents` ref
    // wrapper (see the render output further down); a plain mutable
    // object, not a signal, since it's only ever read inside that same
    // effect right after the DOM it describes has committed — no other
    // reactive consumer needs it directly.
    let measureRefs: Record<string, HTMLElement | undefined> = {};
    const [slotWidths, setSlotWidths] = createSignal<Record<string, number>>({});

    // Re-measures every current slot whenever the pool changes shape OR
    // any slot's own content changes width (both flow through `slots()`
    // recomputing — e.g. ctx text ticking with token counts). Each
    // child's OWN width is zone-independent (neither -controls nor
    // -right applies any width-affecting rule to children beyond
    // flex-wrap), but the GAP BETWEEN a multi-child slot's own children
    // (e.g. ctx's 3 sub-elements, hostShell's 2) is not — reagent P2 on
    // PR #2808: summing only `getBoundingClientRect().width` ignored the
    // real `gap` the current zone applies between them, systematically
    // under-measuring multi-child slots by ~1-2 gaps' worth of pixels,
    // comparable in size to the 8px rounding bucket below. Reading the
    // wrapper's own PARENT's computed `column-gap` (not a hardcoded
    // `--space-1`/`--space-1-5` constant, which would silently drift if
    // the SCSS values ever change) gets the real value for whichever
    // zone this slot currently sits in. This converges to a stable split
    // after one settle pass instead of oscillating between two different
    // "correct" widths.
    //
    // Also depends on `stripWidth()` — regression found post-merge: a
    // slot's OWN rendered width isn't purely a function of its content;
    // this file's own SCSS shed-content queries (`.agent-composer-strip-
    // auth`, `.agent-composer-strip-process-badge` collapsing to
    // `display:none` below a container-width threshold) make it a
    // function of the CONTAINER width too. Without this dependency, a
    // pure resize crossing a shed threshold never re-ran this effect —
    // `slots()` (prop-driven) hadn't changed — so `slotWidths` kept
    // whatever a slot's width was BEFORE it got shed, indefinitely, until
    // some unrelated prop happened to retrigger a remeasurement. The
    // Codex P1 shed-slot fix (which excludes zero-width slots from
    // pairing) never even ran for a slot that measurement never told it
    // was zero — the exact same "stale measurement crosses a structural
    // transition" failure mode already fixed for `statsWidth` above, one
    // more measurement path where it had gone unnoticed.
    createEffect(() => {
        stripWidth();
        const keys = slots().map((s) => s.key);
        const widths: Record<string, number> = {};
        for (const key of keys) {
            const el = measureRefs[key];
            if (!el) continue;
            const children = Array.from(el.children);
            let total = 0;
            for (const child of children) {
                total += child.getBoundingClientRect().width;
            }
            if (children.length > 1 && el.parentElement) {
                // `* zoomRatio()` — see that function's own doc comment;
                // `columnGap` is a `getComputedStyle` (local/unzoomed)
                // read, but `total` above is built from
                // `getBoundingClientRect()` (zoomed) sums.
                const gapPx = (parseFloat(getComputedStyle(el.parentElement).columnGap) || 0) * zoomRatio();
                total += gapPx * (children.length - 1);
            }
            // Rounded UP to an 8px bucket — the live turn ticker (elapsed
            // time, token counts) can shift a slot's text width by a
            // pixel or two every second; without bucketing, the balance
            // below could flip-flop every tick even though nothing
            // meaningful about the content actually changed. `ceil`, not
            // `round` (user-reported regression, 2026-08-26): nearest-8
            // can UNDER-report a slot by up to 4px, making the fit check
            // in `computeComposerRows` optimistic — the row's own CSS
            // flex-wrap safety net then overflows a couple of pixels
            // BEFORE the JS split threshold, so a resize shows two
            // distinct layout breaks within a few px of each other.
            // Rounding up keeps the anti-jitter bucketing while
            // guaranteeing the JS decision always fires at-or-before the
            // point real layout would overflow.
            widths[key] = Math.ceil(total / 8) * 8;
        }
        setSlotWidths(widths);
    });

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
    // Depends on `rightText()` (the stats zone's own content changing
    // width) AND `stripWidth()`/`slotWidths()` (ReAgent P1, PR #2812,
    // re-review) — NOT on `layout()` or `statsWidth()` itself, which
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
        rightText();
        stripWidth();
        slotWidths();
        const zone = statsRefs.single?.isConnected ? statsRefs.single : statsRefs.multi?.isConnected ? statsRefs.multi : undefined;
        const content = zone?.firstElementChild;
        // ceil, not round — same optimistic-fit reasoning as the slot
        // measurement effect above.
        setStatsWidth(content ? Math.ceil(content.getBoundingClientRect().width / 8) * 8 : 0);
    });

    // reagent P1 on PR #2808: `<For>` (below) reconciles by referential
    // identity of each item in its `each` array — passing `slots()`
    // entries directly gives every one a brand-new `{key, side, render}`
    // object on EVERY recompute (any processCount/authStatus/ctxText
    // change, which ticks every second during an active turn). With no
    // stable identity to compare against, `<For>` would treat that as
    // "every slot removed and re-added," remounting `AgentRuntimeDropup`
    // — which owns its own `open`/`selectedOptIndex` signals — and
    // silently collapsing an open Mode/Model/Effort dropdown on a
    // completely unrelated slot's change. Plain string keys compare by
    // VALUE, not reference, so `<For>` over row-derived key arrays (see
    // `rows` below) correctly preserves DOM/component identity for any
    // slot whose row/side didn't change. `slotByKey` is read once inside
    // each `<For>` item's own template callback via `untrack` — Solid
    // only re-invokes that callback when the KEY ITSELF changes, not on
    // every parent recompute, and `untrack` stops the LOOKUP itself from
    // being a tracked dependency of that position (a bare
    // `{slotByKey().get(key)?.render()}` would still re-invoke `render()`
    // on every `slots()` change even with a stable key, defeating the
    // whole point) — so the returned JSX stays reactive to that slot's
    // OWN signal reads regardless (the standard `<For>` idiom). A slot
    // that genuinely changes row or side (a real rebalance decision, not
    // an unrelated prop change) still gets destroyed in one `<For>` and
    // recreated in the other — an inherent limit of separate `<For>`
    // blocks, not something a key can paper over, but a real row/side
    // change is a far rarer event than "any slot's content changed."
    const slotByKey = createMemo(() => {
        const map = new Map<string, { key: string; side: "left" | "right"; interactive: boolean; render: (rowSide: "left" | "right") => JSX.Element }>();
        for (const s of slots()) map.set(s.key, s);
        return map;
    });

    // Rev 7 — replaces `zones()`/`leftKeys()`/`rightKeys()`. Builds an
    // explicit list of ROWS (see `computeComposerRows`'s own doc comment
    // and docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md)
    // instead of two independently-wrapping zones, so every rendered line
    // has content on both sides — the actual requirement six prior
    // revisions all missed (docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md).
    //
    // Falls back to a SINGLE row via the fixed `side` field (Rev 5/6's
    // own fallback, unchanged) whenever real measurement isn't available:
    // first paint (before the effect above has run once), any layout
    // engine that reports 0-width elements (this file's own JSDOM-based
    // unit tests), or the ResizeObserver above hasn't delivered its first
    // callback yet (`stripWidth() === 0`) — never an arbitrary/empty
    // split, and never a spurious multi-row flash before the real width
    // is known.
    // Returns BOTH the row list and `statsInline` — whether the stats
    // zone shares the single row's line (true only when slots-plus-stats
    // genuinely fit together). Separated from row membership
    // (user-reported regression, 2026-08-26): feeding `statsWidth` into
    // `computeComposerRows`'s fit check made a too-wide stats zone split
    // the SLOTS — the strip jumped from 1 visual line straight to 3
    // (2 slot rows + the stats' own dedicated line), skipping the
    // strictly-better middle tier this decision now creates: slots stay
    // on one line and only the stats move to their own line (2 lines).
    const layout = createMemo((): { rows: ComposerRow[]; statsInline: boolean } => {
        const list = slots();
        const widths = slotWidths();
        const allMeasured = list.every((s) => widths[s.key] !== undefined);
        const totalMeasured = Object.values(widths).reduce((sum, w) => sum + w, 0);
        const width = stripWidth();

        // Edge priority (see `orderKeysForEdgePriority`) — applied to
        // EVERY return path below, fallback included, as the final step:
        // pure reordering within a side, after all row-membership and
        // shed decisions are made, so it can't interact with any of them.
        const interactiveByKey = new Map(list.map((s) => [s.key, s.interactive]));
        const edgeOrdered = (r: ComposerRow): ComposerRow => ({
            left: orderKeysForEdgePriority(r.left, "left", (k) => interactiveByKey.get(k) ?? false),
            right: orderKeysForEdgePriority(r.right, "right", (k) => interactiveByKey.get(k) ?? false),
        });

        if (!allMeasured || totalMeasured === 0 || width === 0) {
            const left = list.filter((s) => s.side === "left").map((s) => s.key);
            const right = list.filter((s) => s.side === "right").map((s) => s.key);
            // statsInline true in the fallback — matches the pre-split
            // rendering every environment without measurement (JSDOM,
            // first paint) always had.
            if (left.length === 0 && right.length > 0) {
                return { rows: [edgeOrdered({ left: [right[0]], right: right.slice(1) })], statsInline: true };
            }
            return { rows: [edgeOrdered({ left, right })], statsInline: true };
        }

        // The real column-gap `.agent-composer-strip-row` applies between
        // its own left/right (and, single-row, stats) children — reading
        // it (not a hardcoded constant, which would silently drift if the
        // SCSS value ever changes) matches reagent P2 on PR #2808's same
        // rationale for the per-slot measurement effect above. (Reagent
        // P2, PR #2812: this used to read `--space-1-5`, the row's OWN
        // internal `-row-right` gap, not `--space-2`, the actual
        // `column-gap` between the row's left/right/stats children.)
        //
        // `* zoomRatio()` — regression found post-merge: `getComputedStyle`
        // returns this in LOCAL/unzoomed pixels, but `width`/`widths`
        // below are all `getBoundingClientRect()` (zoomed) values. Without
        // the correction, `gapPx` was too LARGE relative to everything
        // else under any non-1 ancestor zoom, overestimating how much
        // space is needed — the widest tier wrongly split into 2 lines,
        // and the per-pair capacity check (§3.2 step 6) rejected more
        // pairs than necessary as the pane narrowed, reproducing the very
        // one-sided-lines pattern this file exists to prevent. See
        // `zoomRatio`'s own doc comment above.
        const gapPx = rowsRef ? (parseFloat(getComputedStyle(rowsRef).getPropertyValue("--space-2")) || 8) * zoomRatio() : 8;

        // Codex P1, PR #2812: a slot hidden via this file's own SCSS
        // shed-content queries (e.g. `.agent-composer-strip-auth`,
        // `.agent-composer-strip-process-badge` collapsing to
        // `display:none` below a container-width threshold) measures a
        // real 0px — correct, not a measurement bug. Feeding it into
        // `computeComposerRows` anyway let it consume a pairing partner
        // as if it were real content: a row could come back with both
        // `left`/`right` non-empty while one side renders nothing at all,
        // the exact visual bug this revision exists to prevent, just
        // relabeled as a "shed slot" instead of a wrapping zone. Excluded
        // from the pairing math below — never from rendering, since it
        // still needs a stable DOM home to stay measurable and pick back
        // up automatically once the container widens past its own
        // threshold again — then folded back into the last row afterward
        // via its own fallback `side`, where its zero width can never
        // visually unbalance anything.
        const visible = list.filter((s) => s.key === "hostShell" || (widths[s.key] ?? 0) > 0);
        const shed = list.filter((s) => s.key !== "hostShell" && (widths[s.key] ?? 0) === 0);

        const built = computeComposerRows(
            visible.map((s) => ({ key: s.key, width: widths[s.key] ?? 0 })),
            "hostShell",
            width,
            gapPx,
            "runtime",
        );

        // Stats share the single row's line only when slots PLUS stats
        // (plus the stats' own gap) genuinely fit — the check Codex P1
        // (PR #2812) originally put inside computeComposerRows, now the
        // answer to a different question: not "how do slots split" but
        // "where do the stats go." On failure the stats move to their
        // own line above the rows (the multi mount position) while the slots stay
        // exactly where the slot-only decision put them. See
        // `computeStatsInline`'s own doc comment.
        const slotsTotal = visible.reduce((sum, s) => sum + (widths[s.key] ?? 0), 0) + Math.max(0, visible.length - 1) * gapPx;
        const statsInline = computeStatsInline(built.length, slotsTotal, statsWidth(), gapPx, width);

        if (shed.length === 0) return { rows: built.map(edgeOrdered), statsInline };
        const target = built[built.length - 1] ?? { left: [], right: [] };
        for (const s of shed) {
            if (s.side === "left") target.left = [...target.left, s.key];
            else target.right = [...target.right, s.key];
        }
        return {
            rows: (built.length === 0 ? [target] : [...built.slice(0, -1), target]).map(edgeOrdered),
            statsInline,
        };
    });

    const rows = (): ComposerRow[] => layout().rows;
    const statsInline = (): boolean => layout().statsInline;

    // Row COUNT rarely changes across recomputes — plain number primitives
    // compare by value, so `<For>` over indices preserves each row's own
    // subtree (and everything under it) across recomputes that don't
    // change how many rows exist. Iterating `rows()` objects directly
    // would repeat the exact reagent-P1 identity bug the slot-level fix
    // above already fixed, one level up.
    const rowIndices = createMemo(() => rows().map((_, i) => i));

    const renderSlot = (key: string, rowSide: "left" | "right") => {
        // untrack: see `slotByKey`'s own doc comment above — without
        // this, a bare `{slotByKey().get(key)?.render(...)}` would read
        // the reactive `slotByKey()` memo and re-invoke `.render()` (so
        // reconstructing e.g. AgentRuntimeDropup, resetting its own
        // open/selectedOptIndex signals) on every `slots()` recompute,
        // defeating the point of keying by string at all.
        const rendered = untrack(() => slotByKey().get(key)?.render(rowSide));
        return (
            // `display: contents` (agent-composer-strip-slot-measure,
            // _composer-strip.scss) — invisible to layout, so this
            // wrapper changes nothing about how the slot's own children
            // flex/wrap; it exists only to give the measurement effect
            // above a stable per-slot anchor.
            <span class="agent-composer-strip-slot-measure" ref={(el) => (measureRefs[key] = el)}>
                {rendered}
            </span>
        );
    };

    // Stats zone — token/elapsed stats. Always centered (this zone's
    // identity at every tier). Rendered in ONE of two places, mutually
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
    // `layout` memo's `statsInline` decision above (since 2026-08-26;
    // previously `computeComposerRows`'s now-removed `reservedWidth`
    // param, Codex P1 PR #2812), which picks between these two mount
    // positions so the shared line is never overflowed. `variant`
    // tags which of the two mutually-exclusive call sites below is
    // mounting this instance, so the measurement effect above can tell
    // them apart (see that effect's own doc comment).
    const statsZone = (variant: "single" | "multi") => (
        <span class="agent-composer-strip-stats-zone" ref={(el) => (statsRefs[variant] = el)}>
            <Show when={rightText()}>
                <span class="agent-composer-strip-stats">
                    {rightText()}
                </span>
            </Show>
        </span>
    );

    return (
        <div
            class="agent-composer-strip"
            classList={{ "agent-composer-strip--expanded": props.logOpen }}
            ref={stripRef}
        >
            {/* Rev 7 — one <div class="agent-composer-strip-row"> per entry
                in `rows()`, each with its own left- and right-anchored
                span, so every rendered line has content on both sides
                (see `computeComposerRows`'s own doc comment and
                docs/specs/SPEC_COMPOSER_STRIP_ROW_BASED_LAYOUT_2026_08_26.md).
                Replaces the old fixed `-controls`/`-right` zone spans,
                which independently decided when to wrap onto their own
                dedicated one-sided lines — the actual bug this revision
                answers (docs/retro/retro-composer-strip-one-sided-lines-misdiagnosis-2026-08-26.md). */}
            {/* Evicted stats render ABOVE the rows (user-directed,
                2026-08-26): as the pane narrows, the centered token
                stats float UP off the shared line, keeping the
                interactive slot rows anchored at the bottom next to the
                composer instead of being pushed down by a stats line. */}
            <Show when={!statsInline()}>{statsZone("multi")}</Show>

            <div class="agent-composer-strip-rows" ref={rowsRef}>
                <For each={rowIndices()}>
                    {(rowIndex) => (
                        <div class="agent-composer-strip-row">
                            <span class="agent-composer-strip-row-left">
                                <For each={rows()[rowIndex]?.left ?? []}>{(key) => renderSlot(key, "left")}</For>
                            </span>
                            <Show when={statsInline()}>{statsZone("single")}</Show>
                            <span class="agent-composer-strip-row-right">
                                <For each={rows()[rowIndex]?.right ?? []}>{(key) => renderSlot(key, "right")}</For>
                            </span>
                        </div>
                    )}
                </For>
            </div>
        </div>
    );
};

AgentComposerStrip.displayName = "AgentComposerStrip";
