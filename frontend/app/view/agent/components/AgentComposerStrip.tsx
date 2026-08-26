// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentComposerStrip — status row that sits directly above the textarea in
 * the agent pane composer region. Three zones, left/center/right: controls
 * (left), stats (center, always true-centered), everything else (right).
 * Organic flex-wrap reflow — NOT deliberate pixel-breakpoint tiers (that
 * design, and why it was abandoned, is below) — so the strip grows from 1
 * line up to however many it actually needs as the pane narrows, based on
 * each zone's real rendered width, not a guessed magic number.
 *
 * Misc elements (everything except the centered stats zone) are pooled
 * into an ordered list of "slots" and split between the left (controls)
 * and right zones by a fixed semantic `side` on each slot (see `slots`
 * below): left is "what agent, what mode, how much context is left" (the
 * runtime trigger + the context group); right is "status indicators + the
 * one real action" (process badge, auth tag, then HOST/SANDBOX+Shell,
 * Shell always outermost). The only override: `zones` (below) borrows one
 * right-side slot over to the left if that's the only way to avoid a
 * completely empty left zone (e.g. a non-Claude agent with no context
 * tracked yet, where the left zone's only occupants are both hidden).
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
 *     always reports 0-width elements) — see `zones()` further down.
 *

 * Stats zone (center, unaffected by the above): tokens (↑in ↓out) ·
 * elapsed, or the live "Compacting…"/"Reconnecting…" readout.
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
import type { CompactionState, ResumeRetryState } from "@/app/store/agent-pane-state/types";
import { formatCompactNumber, formatExactNumber } from "@/util/format-count";
import { formatElapsedCompact } from "@/util/format-time";
import { For, Show, createEffect, createMemo, createSignal, untrack, type JSX } from "solid-js";
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
        if (leftKeys.size === 0) continue;
        const rightWidth = fixedRightWidth + (totalMovable - leftWidth);
        const diff = Math.abs(leftWidth - rightWidth);
        if (diff < bestDiff) {
            bestDiff = diff;
            best = leftKeys;
        }
    }
    return best;
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
     * `PreCompact` hook's `compaction_started` signal). While set, the
     * center stats zone shows "Compacting…" plus a live elapsed
     * counter instead of the normal turn/session stats — Tier 1/2 of
     * docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md.
     */
    compacting?: CompactionState | null;
    /**
     * Live "reconnecting after a stale `--resume` session id" state, or
     * null. While set, the center stats zone shows "Reconnecting…" plus a
     * live elapsed counter — same treatment as `compacting`, so a stale-
     * resume recovery (usually seconds, occasionally tens of seconds) never
     * reads as a silent hang. See
     * docs/status/STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md §6.2.
     */
    reconnecting?: ResumeRetryState | null;
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

    // Live elapsed time since compaction started — a real stopwatch via
    // Date.now() deltas (ticks every second through the same `useTick`
    // this strip already uses for the turn-elapsed display). Once the
    // authoritative `compact_boundary` event lands, `compacting` clears
    // and the finalized transcript node shows the backend's real
    // `durationMs` instead — this live reading is only ever the
    // in-progress approximation. Tier 2 of
    // docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md.
    const compactingElapsedMs = createMemo(() => {
        const c = props.compacting;
        return c ? (tick(), Date.now() - c.startedAt) : 0;
    });

    // Live elapsed time since a stale-`--resume` retry began — same
    // stopwatch pattern as compactingElapsedMs above. Clears the moment
    // `reconnecting` clears (the retry's outcome — Fresh or Resumed — is
    // known); there's no separate finalized-duration node to hand off to,
    // unlike compaction's transcript node.
    const reconnectingElapsedMs = createMemo(() => {
        const r = props.reconnecting;
        return r ? (tick(), Date.now() - r.startedAt) : 0;
    });

    const rightText = createMemo((): string => {
        // Mutually exclusive with `compacting` by construction (a stale-
        // resume retry only fires once the underlying process has already
        // exited, at which point compaction can't still be in progress),
        // but checked first regardless — a silent-gap recovery is the more
        // easily mistaken-for-a-hang state, so it takes priority if both
        // were ever somehow set.
        if (props.reconnecting) {
            return `Reconnecting…  ${formatElapsedCompact(reconnectingElapsedMs())}`;
        }
        if (props.compacting) {
            return `Compacting…  ${formatElapsedCompact(compactingElapsedMs())}`;
        }
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
    const slots = createMemo((): { key: string; side: "left" | "right"; render: () => JSX.Element }[] => {
        const out: { key: string; side: "left" | "right"; render: () => JSX.Element }[] = [];

        if (showControls()) {
            out.push({
                key: "runtime",
                side: "left",
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
                // immediately right of the context text. Grouped with the
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
                render: () => (
                    <>
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
                    </>
                ),
            });
        }

        // Always present (Shell has no visibility gate) and always last
        // in pool order — see `zones` below for why that keeps it on the
        // right except in the fully degenerate one-slot-total case.
        out.push({
            key: "hostShell",
            side: "right",
            render: () => (
                <span class="agent-composer-strip-host-shell">
                    <Show when={props.agentMode === "host" || props.agentMode === "container"}>
                        <RuntimeBadge runtime={props.agentMode!} size="tag" />
                    </Show>
                    <button
                        type="button"
                        class="agent-composer-strip-log-btn"
                        classList={{ "agent-composer-strip-log-btn--active": props.logOpen }}
                        title={props.logOpen ? "Hide the shell" : "Show the shell"}
                        onClick={() => props.onToggleLog()}
                    >
                        Shell
                    </button>
                </span>
            ),
        });

        return out;
    });

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
    createEffect(() => {
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
                const gapPx = parseFloat(getComputedStyle(el.parentElement).columnGap) || 0;
                total += gapPx * (children.length - 1);
            }
            // Rounded to the nearest 8px — the live turn ticker (elapsed
            // time, token counts) can shift a slot's text width by a
            // pixel or two every second; without this, the balance below
            // could flip-flop every tick even though nothing meaningful
            // about the content actually changed.
            widths[key] = Math.round(total / 8) * 8;
        }
        setSlotWidths(widths);
    });

    // Split left/right, with exactly one override applied uniformly to
    // whichever split resulted: if that leaves the left zone with
    // literally zero slots while the right zone has any, borrow the
    // first right-side slot over to the left instead of showing a dead
    // zone. Single source of truth (this one memo, not two independently
    // derived ones) so the override can't be applied inconsistently
    // between leftKeys/rightKeys.
    //
    // Two paths, chosen per render:
    //   - Real measurement available (every current slot has a
    //     just-measured, non-zero-summing width): use
    //     `computeBalancedLeftKeys` — see that function's own doc
    //     comment for why this is a real-width search, not a repeat of
    //     Rev 2/3's guessed-weight attempts.
    //   - Otherwise (first paint, before the effect above has run once;
    //     or a real layout engine simply isn't present — e.g. this
    //     file's own unit tests run under JSDOM, which always reports
    //     0-width elements): fall back to the fixed semantic `side`
    //     field each slot already carries (Rev 5's pairing) rather than
    //     an arbitrary or empty split.
    const zones = createMemo(() => {
        const list = slots();
        const widths = slotWidths();
        const allMeasured = list.every((s) => widths[s.key] !== undefined);
        const totalMeasured = Object.values(widths).reduce((sum, w) => sum + w, 0);

        let left: typeof list;
        let right: typeof list;
        if (allMeasured && totalMeasured > 0) {
            const hostShellWidth = widths["hostShell"] ?? 0;
            const movable = list
                .filter((s) => s.key !== "hostShell")
                .map((s) => ({ key: s.key, width: widths[s.key] ?? 0 }));
            const leftKeys = computeBalancedLeftKeys(movable, hostShellWidth);
            left = list.filter((s) => leftKeys.has(s.key));
            right = list.filter((s) => !leftKeys.has(s.key));
        } else {
            left = list.filter((s) => s.side === "left");
            right = list.filter((s) => s.side === "right");
        }
        if (left.length === 0 && right.length > 0) {
            return { left: [right[0]], right: right.slice(1) };
        }
        return { left, right };
    });

    // reagent P1 on PR #2808: `<For>` (below) reconciles by referential
    // identity of each item in its `each` array — passing `zones().left`/
    // `.right` directly, as this used to, handed it a brand-new
    // `{key, side, render}` object for every slot on EVERY recompute of
    // `slots()` (any processCount/authStatus/ctxText change, which ticks
    // every second during an active turn). With no stable identity to
    // compare against, `<For>` treated that as "every slot removed and
    // re-added," remounting `AgentRuntimeDropup` — which owns its own
    // `open`/`selectedOptIndex` signals — and silently collapsing an open
    // Mode/Model/Effort dropdown on a completely unrelated slot's change.
    // Plain string keys compare by VALUE, not reference, so `<For>` over
    // `leftKeys()`/`rightKeys()` correctly preserves DOM/component
    // identity for any slot whose zone didn't change. `slotByKey` is read
    // once inside each `<For>` item's own template callback — Solid only
    // re-invokes that callback when the KEY ITSELF changes, not on every
    // parent recompute — so the returned JSX stays reactive to that
    // slot's own signal reads regardless (the standard `<For>` idiom).
    // A slot that genuinely changes zone (a real rebalance decision, not
    // an unrelated prop change) still gets destroyed in one `<For>` and
    // recreated in the other — an inherent limit of two separate `<For>`
    // blocks, not something a key can paper over, but a real zone change
    // is a far rarer event than "any slot's content changed."
    const slotByKey = createMemo(() => {
        const map = new Map<string, { key: string; side: "left" | "right"; render: () => JSX.Element }>();
        for (const s of slots()) map.set(s.key, s);
        return map;
    });
    const leftKeys = createMemo(() => zones().left.map((s) => s.key));
    const rightKeys = createMemo(() => zones().right.map((s) => s.key));

    return (
        <div class="agent-composer-strip" classList={{ "agent-composer-strip--expanded": props.logOpen }}>
            {/* Controls zone — renders whichever slots the dynamic pooling
                above (`leftKeys`) assigned to the left half. See the
                file-header comment and docs/specs/
                SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md for why
                this is computed rather than a fixed set of children, and
                `slotByKey`'s own doc comment for why `<For>` iterates
                plain string keys here rather than the slot objects
                directly. */}
            <span class="agent-composer-strip-controls">
                <For each={leftKeys()}>
                    {(key) => {
                        // untrack: `<For>` calls this template callback
                        // ONCE per stable key, but a bare
                        // `{slotByKey().get(key)?.render()}` inside JSX
                        // would still read the `slotByKey()` MEMO reactively
                        // — re-invoking `.render()` (and so reconstructing
                        // AgentRuntimeDropup, resetting its own open/
                        // selectedOptIndex signals) every time `slots()`
                        // recomputes for ANY reason, defeating the whole
                        // point of keying by string above. Capturing the
                        // rendered JSX once, outside tracking, is what
                        // actually makes this a one-time call — the
                        // returned JSX stays reactive to its OWN internal
                        // signal reads regardless (the standard `<For>`
                        // idiom this file's history already discusses).
                        const rendered = untrack(() => slotByKey().get(key)?.render());
                        return (
                            // `display: contents` (agent-composer-strip-slot-measure,
                            // _composer-strip.scss) — invisible to layout, so this
                            // wrapper changes nothing about how the slot's own
                            // children flex/wrap; it exists only to give the
                            // measurement effect above a stable per-slot anchor.
                            <span
                                class="agent-composer-strip-slot-measure"
                                ref={(el) => (measureRefs[key] = el)}
                            >
                                {rendered}
                            </span>
                        );
                    }}
                </For>
            </span>

            {/* Stats zone — token/elapsed stats. Always centered (this
                zone's identity at every tier, matching its widest-tier
                true-centered position) — forced alone onto its own line
                below 280px, rejoins the controls line (pinned to its right
                edge) at 280-481px, true-centered between the controls and
                right zones at the widest tier (see _composer-strip.scss).
                The wrapper span always renders (even with no stats yet) so
                this zone's presence in the flow order — and therefore
                where the right zone wraps to — stays stable whether or not
                stats are populated yet. */}
            <span class="agent-composer-strip-stats-zone">
                <Show when={rightText()}>
                    <span
                        class="agent-composer-strip-stats"
                        classList={{
                            "agent-composer-strip-stats--compacting": !!props.compacting,
                            "agent-composer-strip-stats--reconnecting": !!props.reconnecting,
                        }}
                    >
                        {rightText()}
                    </span>
                </Show>
            </span>

            {/* Right zone — renders whichever slots the dynamic pooling
                above (`rightKeys`) assigned to the right half. The
                hostShell slot (HOST/SANDBOX tag fused to the Shell toggle,
                see its `render` above) is always last in the pool and
                lands here except in the degenerate one-slot-total case —
                see the file-header comment. Always right-anchored (this
                zone's identity at every tier, matching its widest-tier
                `justify-content: flex-end` pin) — forced alone onto its
                own full-width line below 482px, its own flex-basis-0 half
                of the row at the widest tier (see _composer-strip.scss). */}
            <span class="agent-composer-strip-right">
                <For each={rightKeys()}>
                    {(key) => {
                        // See the left zone's identical pattern above for
                        // why `untrack` is required here, not optional.
                        const rendered = untrack(() => slotByKey().get(key)?.render());
                        return (
                            <span
                                class="agent-composer-strip-slot-measure"
                                ref={(el) => (measureRefs[key] = el)}
                            >
                                {rendered}
                            </span>
                        );
                    }}
                </For>
            </span>
        </div>
    );
};

AgentComposerStrip.displayName = "AgentComposerStrip";
