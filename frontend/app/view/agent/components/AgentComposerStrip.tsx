// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentComposerStrip — slim 28-32px status row that sits directly above
 * the textarea in the agent pane composer region.
 *
 * LEFT:   AgentRuntimeDropup (single Mode · Model · Effort trigger)
 * CENTER: tokens (↑in ↓out) · elapsed — true-centered in the bar
 * RIGHT:  ⚙N process badge · context text (12.1k / 64k) · Shell toggle
 *         (Shell is rightmost — SPEC_COMPOSER_STRIP_LAYOUT_MIC_CENTER_MODEL_DEFAULTS_2026_07_10.md)
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
import { Show, createEffect, createMemo, createSignal, type JSX } from "solid-js";
import type { SessionStats, TurnTokens } from "../types";
import { AgentRuntimeDropup } from "./AgentRuntimeDropup";
import { RuntimeBadge } from "./RuntimeBadge";

// ── Helpers ────────────────────────────────────────────────────────────────

function fmtTokens(t: TurnTokens): string {
    return `↑${formatCompactNumber(t.input)} ↓${formatCompactNumber(t.output)}`;
}

function fmtElapsed(ms: number): string {
    const s = Math.floor(ms / 1000);
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
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
    return (
        `Context window: ${formatExactNumber(tokens)} / ${formatExactNumber(contextWindow)} tokens (${pct}%)\n` +
        `This is the total conversation history sent to the model on each turn.\n` +
        `Auto-compacts around ${formatExactNumber(compactionThreshold(contextWindow))} tokens.`
    );
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

    const rightText = createMemo((): string => {
        if (props.compacting) {
            return `Compacting…  ${fmtElapsed(compactingElapsedMs())}`;
        }
        const parts: string[] = [];
        if (props.loading) {
            if (props.turnTokens) parts.push(fmtTokens(props.turnTokens));
            parts.push(fmtElapsed(elapsedMs()));
        } else if (props.sessionTotals) {
            const s = props.sessionTotals;
            if (s.input_tokens != null || s.output_tokens != null) {
                parts.push(fmtTokens({ input: s.input_tokens ?? 0, output: s.output_tokens ?? 0 }));
            }
            if (s.duration_ms != null) {
                parts.push(fmtElapsed(s.duration_ms));
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

    return (
        <div class="agent-composer-strip" classList={{ "agent-composer-strip--expanded": props.logOpen }}>
            {/* Controls zone — the consolidated Mode/Model/Effort trigger +
                panel, plus the Log button. Reads/writes block meta directly
                (blockId/blockAtom/providerId), so the displayed state matches
                the flags the agent actually runs with from first paint. */}
            <span class="agent-composer-strip-controls">
                <Show when={showControls()}>
                    <AgentRuntimeDropup
                        blockId={props.blockId ?? ""}
                        blockAtom={props.blockAtom ?? (() => undefined)}
                        providerId={props.providerId ?? ""}
                    />
                </Show>
                {/* HOST/SANDBOX tag — not gated by showControls(): agent mode
                    applies to every provider, not just Claude. Replaces the
                    old "Host — full system access" / "Container — isolated
                    Docker sandbox" pane row (confirmed inert — no click
                    handler) with a compact label reusing the same
                    RuntimeBadge component MyAgentsList/HostPopover already
                    use elsewhere for this distinction (size="tag" — see
                    RuntimeBadge.tsx). */}
                <Show when={props.agentMode === "host" || props.agentMode === "container"}>
                    <RuntimeBadge runtime={props.agentMode!} size="tag" />
                </Show>
            </span>

            {/* Center zone — token/elapsed stats, true-centered in the bar via
                the grid's middle column (see _composer-strip.scss). The
                wrapper span always renders (even with no stats yet) so the
                grid keeps 3 children and the right zone stays in the
                rightmost column instead of sliding into the middle one. */}
            <span class="agent-composer-strip-stats-zone">
                <Show when={rightText()}>
                    <span
                        class="agent-composer-strip-stats"
                        classList={{ "agent-composer-strip-stats--compacting": !!props.compacting }}
                    >
                        {rightText()}
                    </span>
                </Show>
            </span>

            {/* Right zone — process badge + context text + Shell toggle, in
                that order so Shell is the rightmost element in the bar. */}
            <span class="agent-composer-strip-right">
                <Show when={(props.processCount ?? 0) > 0}>
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
                </Show>
                <Show when={ctxText()}>
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
                </Show>
                <Show when={props.authStatus === "authenticated" || props.authStatus === "unauthenticated"}>
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
        </div>
    );
};

AgentComposerStrip.displayName = "AgentComposerStrip";
