// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentComposerStrip — slim 28-32px status row that sits directly above
 * the textarea in the agent pane composer region.
 *
 * LEFT:  Mode · Model · Effort drop-ups (open upward) · Log toggle
 * RIGHT: tokens (↑in ↓out) · elapsed · ⚙N process badge ·
 *        context text (12.1k / 64k)
 *
 * The strip bar itself is not clickable. "Log" is the sole toggle for
 * the ActivityLogPanel. Chevron and shell history panel removed per
 * SPEC_COMPOSER_STRIP_AND_HOST_POLISH_2026_06_25.md. Mode/Model/Effort are
 * top-level dropdowns here (Mode was previously a read-only pill + a nested
 * control in the Log region) per SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02.
 */

import { Show, createEffect, createMemo, createSignal, type JSX } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { compactionThreshold } from "@/app/store/agent-pane-state/context-window";
import { getRuntimeConfig } from "../buildRuntimeArgs";
import { applyRuntimeChange } from "../runtime-apply";
import { getProvider } from "../providers";
import { FlyoutMenu } from "@/app/element/flyoutmenu";
import type { AgentRuntimeConfig, EffortLevel, ModelChoice, PermissionMode, SessionStats, TurnTokens } from "../types";

// ── Constants ──────────────────────────────────────────────────────────────

const PERMISSION_COLORS: Record<PermissionMode, string> = {
    bypass: "var(--error-color, #ef4444)",
    auto: "var(--accent-color, #3b82f6)",
    acceptEdits: "var(--warning-color, #eab308)",
    plan: "var(--success-color, #22c55e)",
    default: "var(--main-text-color)",
};

const MODEL_OPTIONS = [
    { value: "opus", label: "Opus" },
    { value: "sonnet", label: "Sonnet" },
    { value: "haiku", label: "Haiku" },
] as const;

const EFFORT_OPTIONS = [
    { value: "low", label: "Low" },
    { value: "medium", label: "Med" },
    { value: "high", label: "High" },
    { value: "xhigh", label: "X-High" },
    { value: "max", label: "Max" },
] as const;

// Mode: trigger shows the short `label`; the menu shows `menuLabel` (the
// descriptive form) when present.
const MODE_OPTIONS = [
    { value: "bypass", label: "Bypass", menuLabel: "Bypass (no prompts)" },
    { value: "auto", label: "Auto", menuLabel: "Auto (AI classifier)" },
    { value: "acceptEdits", label: "Accept Edits" },
    { value: "plan", label: "Plan", menuLabel: "Plan (read-only)" },
    { value: "default", label: "Default", menuLabel: "Default (prompt all)" },
] as const;

// ── Drop-up select ───────────────────────────────────────────────────────────

/**
 * A strip control rendered as a **drop-up**: the trigger is the same slim pill
 * as before, but the popup opens *upward* (via FlyoutMenu `placement="top-start"`)
 * with the app's own menu styling instead of the browser's native `<select>`
 * chrome. Each row is a uniform-height menu item; the selected row carries a
 * radio check. Effort uses `horizontal` to lay its options out in a single row.
 * See SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02 Fix 7.
 */
function StripSelect(props: {
    current: string | undefined;
    options: readonly { value: string; label: string; menuLabel?: string }[];
    onSelect: (value: string) => void;
    title: string;
    /** BEM modifier suffix on the trigger, e.g. "--mode" | "--model" | "--effort". */
    modifier: string;
    /** Color stripe on the trigger's left edge (Mode → permission color). */
    leftColor?: string;
    /** Lay the popup options out horizontally (Effort). */
    horizontal?: boolean;
}): JSX.Element {
    const currentLabel = (): string =>
        props.options.find((o) => o.value === props.current)?.label ?? props.current ?? "";
    // Reactive: Solid wraps this prop expression in a getter, so FlyoutMenu's
    // <For each={items}> re-reads it when `current` changes — the check mark
    // and label stay in sync after a selection.
    const items = (): MenuItem[] =>
        props.options.map((o) => ({
            label: o.menuLabel ?? o.label,
            checked: o.value === props.current,
            onClick: () => props.onSelect(o.value),
        }));
    return (
        <FlyoutMenu
            placement="top-start"
            className={`strip-flyout${props.horizontal ? " strip-flyout--horizontal" : ""}`}
            items={items()}
        >
            <button
                type="button"
                class={`agent-composer-strip-select agent-composer-strip-select${props.modifier}`}
                title={props.title}
                style={props.leftColor ? { "border-left": `3px solid ${props.leftColor}` } : undefined}
            >
                <span class="agent-composer-strip-select-label">{currentLabel()}</span>
                <span class="agent-composer-strip-select-caret" aria-hidden="true">▴</span>
            </button>
        </FlyoutMenu>
    );
}

// ── Helpers ────────────────────────────────────────────────────────────────

function fmtTokens(t: TurnTokens): string {
    const fmt = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
    return `↑${fmt(t.input)} ↓${fmt(t.output)}`;
}

function fmtElapsed(ms: number): string {
    const s = Math.floor(ms / 1000);
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
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

    // Mode/model/effort — derived from blockAtom meta when available.
    const runtime = () =>
        props.blockAtom ? getRuntimeConfig(props.blockAtom()?.meta) : null;

    const updateRuntime = async (patch: { permissionMode?: PermissionMode; model?: ModelChoice; effort?: EffortLevel }): Promise<void> => {
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
        if (w == null) return `${fmtK(t)} ctx`;
        return `${fmtK(t)} / ${fmtK(w)}`;
    };

    return (
        <div
            class="agent-composer-strip"
            classList={{ "agent-composer-strip--expanded": props.logOpen }}
        >
            {/* Controls zone — mode/model/effort selects + Log button.
                All three runtime controls now live here at the strip's top
                level (Mode was previously a read-only pill + a nested control
                inside the Log details region — see
                SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02). Each reads its
                value from `runtime()` (which falls back to
                DEFAULT_RUNTIME_CONFIG), so the displayed state matches the
                flags the agent actually runs with from first paint. */}
            <span class="agent-composer-strip-controls">
                <Show when={showControls()}>
                    {/* Drop-ups (open upward) replace the native <select>s — see
                        SPEC_COMPOSER_STRIP_MODE_TOPLEVEL Fix 7. Model options are
                        registry-driven (single source with /model), so an
                        API-sourced catalog surfaces new labels automatically. */}
                    <StripSelect
                        current={runtime()?.permissionMode ?? "default"}
                        options={MODE_OPTIONS}
                        onSelect={(v) => void updateRuntime({ permissionMode: v as PermissionMode })}
                        title="Permission mode — how the agent asks before running tools. Applies on the next turn."
                        modifier="--mode"
                        leftColor={PERMISSION_COLORS[runtime()?.permissionMode ?? "default"]}
                    />
                    <StripSelect
                        current={runtime()?.model}
                        options={getProvider(props.providerId)?.models ?? MODEL_OPTIONS}
                        onSelect={(v) => void updateRuntime({ model: v as ModelChoice })}
                        title="Model — which model the agent uses. Applies on the next turn."
                        modifier="--model"
                    />
                    <StripSelect
                        current={runtime()?.effort}
                        options={EFFORT_OPTIONS}
                        onSelect={(v) => void updateRuntime({ effort: v as EffortLevel })}
                        title="Reasoning effort — how hard the model thinks per turn. Applies on the next turn."
                        modifier="--effort"
                        horizontal
                    />
                </Show>
                <button
                    type="button"
                    class="agent-composer-strip-log-btn"
                    classList={{ "agent-composer-strip-log-btn--active": props.logOpen }}
                    title={props.logOpen ? "Hide the activity log" : "Show the activity log (launch, tools, errors)"}
                    onClick={() => props.onToggleLog()}
                >
                    Log
                </button>
            </span>

            {/* Right zone — stats + process badge + context text */}
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
                        onClick={() => props.onProcessBadgeClick?.()}
                    >
                        <span aria-hidden="true">⚙</span>
                        <span>{props.processCount}</span>
                    </button>
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
            </span>
        </div>
    );
};

AgentComposerStrip.displayName = "AgentComposerStrip";
