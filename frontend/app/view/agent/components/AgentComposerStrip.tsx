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

    // Split by fixed semantic side (see each slot's `side` above), with
    // exactly one override: if that leaves the left zone with literally
    // zero slots while the right zone has any, borrow the first right-
    // side slot over to the left instead of showing a dead zone. Single
    // source of truth (this one memo, not two independently-derived
    // ones) so the override can't be applied inconsistently between
    // leftSlots/rightSlots.
    //
    // This is deliberately simple — no computed "weight," no search over
    // possible splits. Two earlier attempts (a count-based split, then a
    // weight-balanced subset-partition search over a hand-guessed integer
    // "weight" per slot) both tried to solve visual balance at this
    // layer, in JS, and both produced their own new bugs (see
    // docs/specs/SPEC_COMPOSER_STRIP_DYNAMIC_BALANCE_2026_08_24.md Rev 3
    // for the subset-partition version's own failure). The actual dead-
    // space bug those attempts were chasing lives in the CSS (both zones
    // forced to equal width regardless of content — see
    // _composer-strip.scss's removal of the `flex: 1 1 0` widest-tier
    // rule), not in which slot renders on which side. This rule only
    // needs to guarantee the ORIGINAL, simpler requirement: never a
    // completely empty zone when there's more than one slot to show.
    const zones = createMemo(() => {
        const list = slots();
        const left = list.filter((s) => s.side === "left");
        const right = list.filter((s) => s.side === "right");
        if (left.length === 0 && right.length > 0) {
            return { left: [right[0]], right: right.slice(1) };
        }
        return { left, right };
    });
    const leftSlots = createMemo(() => zones().left);
    const rightSlots = createMemo(() => zones().right);

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
