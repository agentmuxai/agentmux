// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentComposerStrip — status row that sits directly above the textarea in
 * the agent pane composer region. Deliberate tiered split points (not
 * organic flex-wrap reflow), always edge-split — same left/center/right
 * visual language as the widest tier, extended to fewer zones per line as
 * the pane narrows, never a centered blob — see
 * docs/specs/SPEC_COMPOSER_STRIP_CENTERED_SMART_SPLIT_2026_08_14.md
 * (supersedes SPEC_COMPOSER_STRIP_LEFT_JUSTIFIED_TIERED_WRAP_2026_08_03.md's
 * "always left-justify below the widest tier" design, reverted per direct
 * user feedback; an intermediate "center everything as one group" revision
 * was itself corrected same day per further feedback). <280px: controls /
 * stats / right, each its own line, each keeping its identity (left /
 * center / right) exactly as it would sit at the widest tier, just
 * stacked. 280-481px: [controls left | stats right] on line 1, right
 * alone (right-anchored) on line 2. >=482px (live-measured — the real
 * 1-line/2-line wrap point, not an estimate): controls left / stats zone
 * true-centered / right right — see _composer-strip.scss for the actual
 * container queries.
 *
 * Misc elements (everything except the centered stats zone) are pooled into
 * an ordered list of "slots" and split DYNAMICALLY between the left
 * (controls) and right zones — see `slots`/`splitIndex`/`leftSlots`/
 * `rightSlots` below. Two things had to be fixed to get here, both same-day
 * (2026-08-24) corrections per direct user feedback:
 *
 *   (1) A static per-item zone assignment (badge+auth always left,
 *       everything else always right — see
 *       docs/specs/SPEC_COMPOSER_STRIP_LEFT_RIGHT_BALANCE_2026_08_24.md)
 *       looks balanced only when every item happens to be visible at once.
 *       `AgentRuntimeDropup` alone is gated to Claude (`showControls()`),
 *       and the badge/auth tag are each independently conditional — a
 *       non-Claude agent with no tracked processes and unknown auth status
 *       left the ENTIRE controls zone empty while the right zone still
 *       held Shell (which always renders). Fix: pool whichever elements
 *       are ACTUALLY visible right now and split that pool, instead of
 *       hardcoding which zone each element belongs to.
 *
 *   (2) A count-based split (`floor(pool.length/2)`) still isn't enough:
 *       it treats the 3-wide context group (ctx text + countdown +
 *       Compact) as the same "weight" as the 1-wide auth tag, so a
 *       2-left/3-right split by COUNT could still be lopsided by actual
 *       rendered width. Fix: each slot carries a `weight` (how many
 *       visually distinct sub-elements it's currently rendering).
 *
 *   (3) A weight-balanced PREFIX cut (`floor(N/2)`-style, just on
 *       cumulative weight instead of count) still isn't enough either: a
 *       contiguous "first k slots left" cut can't separate two heavy
 *       slots that happen to sit adjacent in pool order (the context
 *       group and HOST/SANDBOX+Shell both sit at the END of the pool and
 *       are usually the two heaviest) — every cut point either lumps them
 *       together or can't reach past them, capping the best achievable
 *       split at something like 2-vs-5 even though a better balance
 *       exists. This is the literal "we get 2 lines on the right, 1 on
 *       the left — that should be impossible" bug direct user feedback
 *       caught. Fix: `leftMask` (below) brute-forces every possible
 *       left/right SUBSET assignment (not just contiguous cuts — the pool
 *       is capped at 5 slots, so at most 32 combinations) and picks
 *       whichever minimizes the weight difference.
 *
 * Slot pool, in order (see `slots` below for the exact visibility gate and
 * weight of each):
 *   1. AgentRuntimeDropup (Mode · Model · Effort trigger) — Claude only.
 *      Weight 1.
 *   2. ⚙N process badge — when any process is tracked. Weight 1.
 *   3. Auth tag — once auth status is known. Weight 1.
 *   4. Context group — context text + countdown + Compact button, bundled
 *      as ONE slot (never split across zones) because Compact must sit
 *      immediately right of the context text (pre-existing constraint).
 *      Weight 1-3 depending on how many of the three are currently
 *      showing.
 *   5. HOST/SANDBOX tag + Shell toggle — ALWAYS present (Shell has no
 *      visibility gate) and ALWAYS the last slot, bundled as one atomic
 *      unit (never split across zones) per direct user request predating
 *      this change ("the HOST/SANDBOX indicator should be just to the left
 *      of the Shell button"). Weight 1 or 2 depending on whether
 *      HOST/SANDBOX is showing. Being last and `splitIndex`'s tie-break
 *      (prefers the smaller left-side cut when two cuts balance equally
 *      well) means this pair lands in the right zone whenever any other
 *      slot is also visible, and only crosses into the left zone in the
 *      degenerate case where it's the pool's ONLY slot (one item can't
 *      populate both zones — an unavoidable floor, not a bug).
 *
 * Stats zone (centered, unaffected by the above): tokens (↑in ↓out) ·
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
import { For, Show, createEffect, createMemo, createSignal, type JSX } from "solid-js";
import type { SessionStats, TurnTokens } from "../types";
import { AgentRuntimeDropup } from "./AgentRuntimeDropup";
import { RuntimeBadge } from "./RuntimeBadge";

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
    // computed fresh here every time, changed. `weight` is the number of
    // visually distinct sub-elements the slot renders — a plain count-based
    // split (`floor(slots.length/2)`) treats the 3-wide context group the
    // same as the 1-wide auth tag, which can leave one zone needing an
    // internal wrap to a 2nd line while the other sits on 1 (Codex/direct
    // user feedback, same day: "we get 2 lines on the right, 1 on the
    // left — that should be impossible"). The split point below balances
    // cumulative weight instead, so neither zone carries more visual mass
    // than the other by more than one slot's worth.
    const slots = createMemo((): { key: string; weight: number; render: () => JSX.Element }[] => {
        const out: { key: string; weight: number; render: () => JSX.Element }[] = [];

        if (showControls()) {
            out.push({
                key: "runtime",
                weight: 1,
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
                weight: 1,
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
                weight: 1,
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
            const compactVisible = props.providerId === "claude" && props.onCompact != null;
            out.push({
                key: "ctx",
                // ctx text (always) + countdown (conditional) + Compact
                // button (conditional) — this slot can't be split across
                // zones (Compact must stay immediately right of ctx text),
                // but its WEIGHT still reflects how many sub-elements it's
                // actually rendering right now, so the split point accounts
                // for its real width instead of counting it as "1 item"
                // regardless of whether it's showing 1 or 3 things.
                weight: 1 + (ctxCountdownText() != null ? 1 : 0) + (compactVisible ? 1 : 0),
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

        // Always present, always last — see file-header comment on why
        // this pair lands right unless it's the pool's only slot. Weight
        // 1 (Shell alone) or 2 (HOST/SANDBOX tag + Shell) depending on
        // whether agentMode is known.
        out.push({
            key: "hostShell",
            weight: props.agentMode === "host" || props.agentMode === "container" ? 2 : 1,
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

    // Weight-balanced SUBSET partition — NOT a prefix cut. A contiguous
    // "first k slots left, rest right" cut can't balance weight when the
    // heaviest slots happen to sit adjacent to each other in pool order:
    // e.g. runtime(1)+auth(1)+ctx(3)+hostShell(2) — the two heaviest
    // (ctx, hostShell) are adjacent at the end, so every possible cut
    // point either splits them apart (not allowed, they're atomic) or
    // lumps them together, capping the best achievable split at 2-vs-5.
    // That's the literal "1 line left, 2 lines right" bug direct user
    // feedback caught. Fix: brute-force every possible left/right subset
    // assignment (pool is capped at 5 slots, so at most 32 combinations —
    // trivial) and pick whichever assignment minimizes the weight
    // difference, with two tie-breaks: (a) prefer keeping the hostShell
    // slot (Shell — the strip's one real action) on the right, matching
    // its established outermost-right convention; (b) among remaining
    // ties, prefer fewer items on the left. Slots keep their original
    // pool order WITHIN whichever side they land on (leftSlots/rightSlots
    // below filter, not reorder), so relative order is still predictable
    // even though which SIDE a given slot lands on now depends on the
    // whole pool's weight distribution, not just its own fixed position.
    const leftMask = createMemo(() => {
        const list = slots();
        const n = list.length;
        const hostIdx = list.findIndex((s) => s.key === "hostShell");
        const total = list.reduce((sum, s) => sum + s.weight, 0);
        let bestMask = 0;
        let bestDiff = Infinity;
        let bestHostRight = false;
        let bestLeftCount = Infinity;
        for (let mask = 0; mask < (1 << n); mask++) {
            let leftWeight = 0;
            let leftCount = 0;
            for (let i = 0; i < n; i++) {
                if (mask & (1 << i)) {
                    leftWeight += list[i].weight;
                    leftCount++;
                }
            }
            const diff = Math.abs(leftWeight - (total - leftWeight));
            const hostRight = hostIdx === -1 || (mask & (1 << hostIdx)) === 0;
            const better =
                diff < bestDiff
                || (diff === bestDiff && hostRight && !bestHostRight)
                || (diff === bestDiff && hostRight === bestHostRight && leftCount < bestLeftCount);
            if (better) {
                bestMask = mask;
                bestDiff = diff;
                bestHostRight = hostRight;
                bestLeftCount = leftCount;
            }
        }
        return bestMask;
    });

    const leftSlots = createMemo(() => slots().filter((_, i) => (leftMask() & (1 << i)) !== 0));
    const rightSlots = createMemo(() => slots().filter((_, i) => (leftMask() & (1 << i)) === 0));

    return (
        <div class="agent-composer-strip" classList={{ "agent-composer-strip--expanded": props.logOpen }}>
            {/* Controls zone — renders whichever slots the dynamic pooling
                above (`leftSlots`) assigned to the left half. See the
                file-header comment and docs/specs/
                SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md for why
                this is computed rather than a fixed set of children. */}
            <span class="agent-composer-strip-controls">
                <For each={leftSlots()}>{(slot) => slot.render()}</For>
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
                above (`rightSlots`) assigned to the right half. The
                hostShell slot (HOST/SANDBOX tag fused to the Shell toggle,
                see its `render` above) is always last in the pool and
                lands here except in the degenerate one-slot-total case —
                see the file-header comment. Always right-anchored (this
                zone's identity at every tier, matching its widest-tier
                `justify-content: flex-end` pin) — forced alone onto its
                own full-width line below 482px, its own flex-basis-0 half
                of the row at the widest tier (see _composer-strip.scss). */}
            <span class="agent-composer-strip-right">
                <For each={rightSlots()}>{(slot) => slot.render()}</For>
            </span>
        </div>
    );
};

AgentComposerStrip.displayName = "AgentComposerStrip";
