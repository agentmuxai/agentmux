// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentComposerStrip — slim 28-32px status row that sits directly above
 * the textarea in the agent pane composer region.
 *
 * Redesigned per SPEC_AGENT_COMPOSER_STRIP_REDESIGN_2026_06_23.md:
 *   - LEFT: Model <select> · Effort <select> · Shell toggle button
 *   - RIGHT: tokens (↑in ↓out) · elapsed · ⚙N process badge ·
 *            permission pill · context text (12.1k / 64k) · chevron
 *
 * The prior left-zone spinner + tool name (⟳ bash) has been removed —
 * AgentWorkingRow (rendered just above this strip) is the canonical
 * in-flight status indicator.
 *
 * ARIA contract: outer strip is a layout container only (no role/tabIndex).
 * Model/effort selects and Shell/chevron buttons own their own focus
 * and keyboard contracts. Body click toggles details for mouse-only
 * convenience; keyboard users use Tab → target → Enter.
 */

import { Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { compactionThreshold } from "@/app/store/agent-pane-state/context-window";
import { getRuntimeConfig } from "../buildRuntimeArgs";
import { applyRuntimeChange } from "../runtime-apply";
import { getProvider } from "../providers";
import type { AgentRuntimeConfig, EffortLevel, ModelChoice, PermissionMode, SessionStats, TurnTokens } from "../types";

// ── Constants ──────────────────────────────────────────────────────────────

const PERMISSION_LABELS: Record<PermissionMode, string> = {
    bypass: "Bypass",
    auto: "Auto",
    acceptEdits: "Accept Edits",
    plan: "Plan",
    default: "Default",
};

const PERMISSION_COLORS: Record<PermissionMode, string> = {
    bypass: "var(--error-color, #ef4444)",
    auto: "var(--accent-color, #3b82f6)",
    acceptEdits: "var(--warning-color, #eab308)",
    plan: "var(--success-color, #22c55e)",
    default: "var(--main-text-color)",
};

const MODEL_OPTIONS = [
    { value: "", label: "Default" },
    { value: "opus", label: "Opus" },
    { value: "sonnet", label: "Sonnet" },
    { value: "haiku", label: "Haiku" },
] as const;

const EFFORT_OPTIONS = [
    { value: "", label: "Default" },
    { value: "low", label: "Low" },
    { value: "medium", label: "Med" },
    { value: "high", label: "High" },
    { value: "xhigh", label: "X-High" },
    { value: "max", label: "Max" },
] as const;

// ── Helpers ────────────────────────────────────────────────────────────────

function fmtTokens(t: TurnTokens): string {
    const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
    return `↑${fmt(t.input)} ↓${fmt(t.output)}`;
}

function fmtElapsed(ms: number): string {
    const s = Math.floor(ms / 1000);
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}

function fmtBadgeCount(n: number): string {
    return n > 9 ? "9+" : String(n);
}

function fmtK(n: number): string {
    return `${Math.round(n / 100) / 10}k`;
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
        return `Context: ${tokens.toLocaleString()} tokens`;
    }
    const pct = ((tokens / contextWindow) * 100).toFixed(1);
    return (
        `Context window: ${tokens.toLocaleString()} / ${contextWindow.toLocaleString()} tokens (${pct}%)\n` +
        `This is the total conversation history sent to the model on each turn.\n` +
        `Auto-compacts around ${compactionThreshold(contextWindow).toLocaleString()} tokens.`
    );
}

// ── Props ──────────────────────────────────────────────────────────────────

interface AgentComposerStripProps {
    /**
     * DOM id of the details panel this strip toggles. Used for
     * `aria-controls` on the chevron button.
     */
    detailsPanelId: string;
    /** True while a turn is in flight. */
    loading?: boolean;
    /** Final session totals; non-null after the first TurnEnd. */
    sessionStats?: SessionStats | null;
    /** Live tokens for the in-flight turn. */
    turnTokens?: TurnTokens | null;
    /** Count of OS processes tracked for this agent block. */
    processCount?: number;
    /** Fires when the user clicks the ⚙N process badge. */
    onProcessBadgeClick?: () => void;
    /** Permission mode — renders a color-coded pill when not `auto`. */
    permissionMode?: PermissionMode;
    /** Reducer-projected: details panel open/closed. */
    expanded: boolean;
    /** Reducer-projected: unread activity-log entries while collapsed. */
    unreadCount: number;
    /** Dispatches `DetailsToggle` to the pane reducer. */
    onToggleExpanded: () => void;
    /** Current context fill in tokens (from message_start). */
    contextTokens?: number | null;
    /** Provider's max context window size. undefined = unknown. */
    contextWindow?: number;

    // ── Shell panel ────────────────────────────────────────────────
    /** Whether the shell history panel is open. */
    shellOpen?: boolean;
    /** Dispatches `ShellToggle` to the pane reducer. */
    onToggleShell?: () => void;

    // ── Inline model / effort controls ────────────────────────────
    /** Block id — needed for applyRuntimeChange. */
    blockId?: string;
    /** Block atom — reads current model/effort from meta. */
    blockAtom?: () => Block | undefined;
    /** Provider id — needed for applyRuntimeChange. */
    providerId?: string;
}

// ── Component ──────────────────────────────────────────────────────────────

export const AgentComposerStrip = (props: AgentComposerStripProps): JSX.Element => {
    const [elapsedMs, setElapsedMs] = createSignal(0);
    createEffect(() => {
        if (!props.loading) return;
        const start = Date.now();
        setElapsedMs(0);
        const id = setInterval(() => setElapsedMs(Date.now() - start), 1000);
        onCleanup(() => clearInterval(id));
    });

    const rightText = createMemo((): string => {
        const parts: string[] = [];
        if (props.loading) {
            if (props.turnTokens) parts.push(fmtTokens(props.turnTokens));
            parts.push(fmtElapsed(elapsedMs()));
        } else if (props.sessionStats) {
            const s = props.sessionStats;
            if (s.input_tokens != null || s.output_tokens != null) {
                parts.push(fmtTokens({ input: s.input_tokens ?? 0, output: s.output_tokens ?? 0 }));
            }
            if (s.duration_ms != null) {
                parts.push(fmtElapsed(s.duration_ms));
            }
        }
        return parts.join("  ·  ");
    });

    const hasOwn = Object.prototype.hasOwnProperty;
    const showPermissionPill = (): boolean => {
        const m = props.permissionMode;
        return m != null && m !== "auto" && hasOwn.call(PERMISSION_LABELS, m);
    };

    // Guard clicks on interactive children from triggering the strip-body toggle.
    const eventTargetIsInteractive = (e: MouseEvent): boolean => {
        const t = e.target as HTMLElement | null;
        if (!t) return false;
        return t.closest("button, select") != null;
    };

    // Model/effort — derived from blockAtom meta when available.
    const runtime = () =>
        props.blockAtom ? getRuntimeConfig(props.blockAtom()?.meta) : null;

    const updateRuntime = async (patch: { model?: ModelChoice | ""; effort?: EffortLevel | "" }): Promise<void> => {
        const r = runtime();
        if (!r || !props.blockId || !props.providerId) return;
        try {
            await applyRuntimeChange(
                props.blockId,
                getProvider(props.providerId),
                { ...r, ...patch } as AgentRuntimeConfig,
            );
        } catch {
            // Silent — settings retry on next change.
        }
    };

    // Show model/effort controls only when blockAtom is wired up.
    const showControls = () => props.blockAtom != null;

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
        if (w == null) return `${fmtK(t)} ctx`;
        return `${fmtK(t)} / ${fmtK(w)}`;
    };

    return (
        <div
            class="agent-composer-strip"
            classList={{ "agent-composer-strip--expanded": props.expanded }}
            onClick={(e) => {
                if (!eventTargetIsInteractive(e)) {
                    props.onToggleExpanded();
                }
            }}
        >
            {/* Controls zone — model/effort selects + Shell button */}
            <span class="agent-composer-strip-controls">
                <Show when={showControls()}>
                    <select
                        class="agent-composer-strip-select"
                        title="Model"
                        value={runtime()?.model ?? ""}
                        onChange={(e) => void updateRuntime({ model: e.currentTarget.value as ModelChoice | "" })}
                        onClick={(e) => e.stopPropagation()}
                    >
                        {MODEL_OPTIONS.map((o) => (
                            <option value={o.value}>{o.label}</option>
                        ))}
                    </select>
                    <select
                        class="agent-composer-strip-select"
                        title="Effort"
                        value={runtime()?.effort ?? ""}
                        onChange={(e) => void updateRuntime({ effort: e.currentTarget.value as EffortLevel | "" })}
                        onClick={(e) => e.stopPropagation()}
                    >
                        {EFFORT_OPTIONS.map((o) => (
                            <option value={o.value}>{o.label}</option>
                        ))}
                    </select>
                </Show>
                <button
                    type="button"
                    class="agent-composer-strip-shell-btn"
                    classList={{ "agent-composer-strip-shell-btn--active": !!props.shellOpen }}
                    title={props.shellOpen ? "Close shell history" : "Open shell history"}
                    onClick={(e) => {
                        e.stopPropagation();
                        props.onToggleShell?.();
                    }}
                >
                    Shell
                </button>
            </span>

            {/* Right zone — stats + permission + context text + chevron */}
            <span class="agent-composer-strip-right">
                <Show when={rightText()}>
                    <span class="agent-composer-strip-stats">{rightText()}</span>
                </Show>
                <Show when={(props.processCount ?? 0) > 0}>
                    <button
                        type="button"
                        class="agent-composer-strip-process-badge"
                        data-strip-button
                        title={`${props.processCount} tracked ${props.processCount === 1 ? "process" : "processes"} — click to open swarm`}
                        onClick={(e) => {
                            e.stopPropagation();
                            props.onProcessBadgeClick?.();
                        }}
                    >
                        <span aria-hidden="true">⚙</span>
                        <span>{props.processCount}</span>
                    </button>
                </Show>
                <Show when={showPermissionPill()}>
                    <span
                        class="agent-composer-strip-perm-pill"
                        style={{ color: PERMISSION_COLORS[props.permissionMode!] }}
                        title={`Permission mode: ${PERMISSION_LABELS[props.permissionMode!]}`}
                    >
                        {PERMISSION_LABELS[props.permissionMode!]}
                    </span>
                </Show>
                <Show when={ctxText()}>
                    <span
                        class={`agent-composer-strip-ctx ${ctxClass()}`}
                        title={props.contextTokens != null
                            ? contextTitle(props.contextTokens, props.contextWindow)
                            : undefined}
                    >
                        {ctxText()}
                    </span>
                </Show>
                <button
                    type="button"
                    class="agent-composer-strip-chevron"
                    classList={{ "agent-composer-strip-chevron--expanded": props.expanded }}
                    aria-expanded={props.expanded}
                    aria-controls={props.detailsPanelId}
                    aria-label={
                        props.expanded
                            ? `Collapse composer details${props.unreadCount > 0 ? ` (${props.unreadCount} unread)` : ""}`
                            : `Expand composer details${props.unreadCount > 0 ? ` (${props.unreadCount} unread)` : ""}`
                    }
                    onClick={(e) => {
                        e.stopPropagation();
                        props.onToggleExpanded();
                    }}
                >
                    <span aria-hidden="true">{props.expanded ? "▴" : "▾"}</span>
                    <Show when={!props.expanded && props.unreadCount > 0}>
                        <sup class="agent-composer-strip-chevron-badge" aria-hidden="true">
                            {fmtBadgeCount(props.unreadCount)}
                        </sup>
                    </Show>
                </button>
            </span>
        </div>
    );
};

AgentComposerStrip.displayName = "AgentComposerStrip";
