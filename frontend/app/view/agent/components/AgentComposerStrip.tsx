// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentComposerStrip — slim 28-32px status row that sits directly above
 * the textarea in the agent pane composer region.
 *
 * Replaces the prior `AgentStatusLine` (loading-bar slab + cycling
 * "Working…" phrase) AND the always-visible `AgentControlBar`
 * (permission/model/effort dropdowns + Archive/Export). Both fold
 * into:
 *   - LEFT segment:  small circular spinner + latest activity-log line
 *                    (truncated). Idle = empty. Stopping = decelerating
 *                    spinner + "Stopping…". Done-just-now = ✓ fade.
 *   - RIGHT segment: tokens (↑in ↓out) + elapsed/total + ⚙N process
 *                    badge + non-default permission pill (color-coded).
 *   - CHEVRON:       `▾` collapsed / `▴` expanded; superscript count
 *                    when unread activity-log entries accumulate while
 *                    collapsed (`▾³`).
 *
 * State is reducer-owned: the chevron's expanded state and unread count
 * are both atoms projected from `AgentPaneState.detailsOpen` /
 * `composerUnreadCount`. The view dispatches `DetailsToggle` /
 * `DetailsCollapse` to flip them — no local Solid signals, no parallel
 * state machine.
 *
 * Spec: docs/specs/SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md.
 */

import { Show, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";

import type { PermissionMode, SessionStats, TurnTokens } from "../types";

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

interface AgentComposerStripProps {
    /**
     * DOM id of the details panel this strip toggles. Used for the
     * `aria-controls` attribute on the strip and the matching `id` on
     * the panel. Per-pane uniqueness is the caller's responsibility —
     * a fixed id would collide when multiple agent panes are open in
     * the same tab. Codex P2 on PR #1069.
     */
    detailsPanelId: string;
    /** True while a turn is in flight (Submitting / Streaming / Interrupting). */
    loading?: boolean;
    /** True after Esc → SIGINT, until session_end arrives. */
    stopping?: boolean;
    /** Name of the tool currently executing (e.g. "bash", "edit"). */
    currentTool?: string | null;
    /** Final session totals; non-null after the first TurnEnd. */
    sessionStats?: SessionStats | null;
    /** Live tokens for the in-flight turn. */
    turnTokens?: TurnTokens | null;
    /** Count of OS processes tracked for this agent block. */
    processCount?: number;
    /** Fires when the user clicks the ⚙N process badge. */
    onProcessBadgeClick?: () => void;
    /**
     * Latest activity-log entry. Surfaced in the left segment when no
     * `currentTool` is set so users can see what the agent's doing
     * without expanding the details panel.
     */
    latestLogLine?: string;
    /**
     * Permission mode (`auto` / `plan` / `bypass` / `acceptEdits` /
     * `default`). Renders an inline color-coded pill ONLY when the
     * mode is NOT `auto` — `auto` (AI classifier) is the quiet
     * baseline most users sit in; any other mode (including the
     * conservative `default = prompt all`) is information worth
     * surfacing in the strip without an expand. An unknown/legacy
     * value also hides the pill — see `showPermissionPill`.
     */
    permissionMode?: PermissionMode;
    /** Reducer-projected: details panel open/closed. */
    expanded: boolean;
    /** Reducer-projected: unread activity-log entries while collapsed. */
    unreadCount: number;
    /** Dispatches `DetailsToggle` to the pane reducer. */
    onToggleExpanded: () => void;
}

export const AgentComposerStrip = (props: AgentComposerStripProps): JSX.Element => {
    // Live elapsed timer for the current turn. Resets to 0 when loading
    // flips false → true; ticks every second while loading. Same shape
    // as the old AgentStatusLine's elapsedMs signal.
    const [elapsedMs, setElapsedMs] = createSignal(0);
    createEffect(() => {
        if (!props.loading) return;
        const start = Date.now();
        setElapsedMs(0);
        const id = setInterval(() => setElapsedMs(Date.now() - start), 1000);
        onCleanup(() => clearInterval(id));
    });

    // Left segment — what's the agent doing right now.
    const leftText = createMemo((): string => {
        if (props.stopping) return "Stopping…";
        if (props.currentTool) return props.currentTool;
        if (props.loading && props.latestLogLine) return props.latestLogLine;
        if (props.loading) return "Working…";
        if (props.latestLogLine) return props.latestLogLine;
        return "";
    });

    // Right segment — tokens + elapsed (live during turn, session
    // totals after).
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

    // Permission pill only renders when:
    //   - the mode is defined AND known (defends against legacy / typo
    //     values from block meta — an unrecognized key would render an
    //     empty pill with `undefined` label/color), AND
    //   - the mode is NOT `auto` (the quiet baseline; AI classifier).
    // `default` (explicit prompt-all) DOES render the pill — users
    // expect to see that they're in the conservative mode. Spec §3.2
    // table row "Permission != Auto". Reagent P2 on PR #1069 caught
    // the doc/code mismatch + the missing validity gate.
    const showPermissionPill = (): boolean => {
        const m = props.permissionMode;
        return m != null && m !== "auto" && m in PERMISSION_LABELS;
    };

    // ARIA contract (codex P2 round 3 on PR #1069):
    //   • The outer strip is JUST a layout container — no `role`,
    //     no `tabIndex`, no `aria-*`. A `role="button"` div containing
    //     a real `<button>` (the process badge) is nested-interactive-
    //     control which assistive tech may misreport or skip.
    //   • The CHEVRON is the canonical toggle: a real `<button>` that
    //     owns `aria-expanded` + `aria-controls` + the keyboard
    //     contract. Screen readers announce "expand/collapse composer
    //     details, button" — semantically precise.
    //   • The strip body still toggles on mouse click for sighted-
    //     user convenience (delegated `onClick` checks the target
    //     wasn't a nested button), but it's a pointer-only affordance
    //     — keyboard users use Tab → chevron → Enter.
    const eventTargetIsNestedButton = (e: MouseEvent): boolean => {
        const t = e.target as HTMLElement | null;
        if (!t) return false;
        return t.closest("button") != null;
    };

    return (
        <div
            class="agent-composer-strip"
            classList={{ "agent-composer-strip--expanded": props.expanded }}
            onClick={(e) => {
                // Body click expands (mouse-only convenience). Nested
                // buttons own their own activation; don't double-fire.
                if (!eventTargetIsNestedButton(e)) {
                    props.onToggleExpanded();
                }
            }}
        >
            <span class="agent-composer-strip-left">
                <Show when={props.loading || props.stopping}>
                    <span
                        class="agent-composer-strip-spinner"
                        classList={{ "agent-composer-strip-spinner--decelerating": props.stopping }}
                        aria-hidden="true"
                    >
                        {/* SVG circle stroke-dasharray animation —
                            replaces the old spinner-dot loading-bar
                            slab. 16px diameter, 60rpm. */}
                        <svg viewBox="0 0 16 16" width="14" height="14">
                            <circle
                                cx="8"
                                cy="8"
                                r="6"
                                fill="none"
                                stroke="currentColor"
                                stroke-width="2"
                                stroke-dasharray="9 30"
                                stroke-linecap="round"
                            />
                        </svg>
                    </span>
                </Show>
                <Show when={leftText()}>
                    <span class="agent-composer-strip-left-text">{leftText()}</span>
                </Show>
            </span>
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
