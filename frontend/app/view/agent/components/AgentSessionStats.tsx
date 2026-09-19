// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSessionStats — the composer strip's context reading (`104k / 200k`) as
 * a button, plus the popover it opens.
 *
 * The strip used to render that reading as inert text whose only affordance
 * was a hover tooltip, while the numbers behind it — cumulative cost, turns,
 * duration, token totals with their cache split (`sessionTotals`) — were
 * computed every turn and rendered nowhere, having lost their only consumer
 * when SPEC_COMPOSER_STRIP_DROP_CENTER_STATS_2026_08_31.md removed the strip's
 * center stats. The label is the natural entry point to them, so it opens a
 * panel that shows them along with the session's Archive / Export / Restore
 * actions (which moved here out of the Shell drawer).
 *
 * See `docs/specs/SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md` §3.1.
 *
 * Panel mechanics (open/close, outside-click, focus-out, Escape, floating
 * placement) deliberately mirror `AgentRuntimeDropup`, the strip's other
 * panel — same `computeMenuPosition` primitive, same `data-pane-overlay`, and
 * the same "listeners live only while open" effect scoping.
 */

import { autoUpdate } from "@floating-ui/dom";
import { createEffect, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";

import { assertMenuInPaintableArea, computeMenuPosition } from "@/app/util/menu-position";
import { formatCompactNumber, formatExactNumber } from "@/util/format-count";

import {
    archiveSession,
    exportSession,
    isSessionArchived,
    restoreSession,
    sessionArchivedAt,
    sessionLastActivityMs,
    sessionLineCount,
    type BlockAccessor,
} from "../session-actions";
import type { SessionStats } from "../types";

/** Serialize a MenuPositionResult.style the same way flyoutmenu.tsx does. */
function styleToString(s: JSX.CSSProperties): string {
    return `position:${s.position};left:${s.left};top:${s.top}`;
}

interface AgentSessionStatsProps {
    blockId: string;
    blockAtom: BlockAccessor;
    providerId: string;
    /** Current context fill in tokens (from message_start). */
    contextTokens?: number | null;
    /** Provider's max context window size. undefined = unknown. */
    contextWindow?: number;
    /** Cumulative cost/tokens/duration across this pane's completed turns. */
    sessionTotals?: SessionStats | null;
    /** The strip's own ctx band class, applied to the trigger. */
    ctxClass: string;
    /** Hover text the strip already computed for the reading. */
    title?: string;
    /** The reading itself, e.g. `104k / 200k`. */
    label: string;
}

/** `93s` / `4m 12s` / `2h 14m` — duration, not a clock time. */
function formatDuration(ms: number): string {
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s`;
    const m = Math.floor(s / 60);
    // No trailing "0s" on a round minute (ReAgent P2 on #3436).
    if (m < 60) return s % 60 === 0 ? `${m}m` : `${m}m ${s % 60}s`;
    const h = Math.floor(m / 60);
    return `${h}h ${m % 60}m`;
}

/** `3m ago` / `2h ago` / `—` when never. */
function formatAgo(ts: number): string {
    if (!ts) return "—";
    const delta = Date.now() - ts;
    if (delta < 60_000) return "just now";
    return `${formatDuration(delta).split(" ")[0]} ago`;
}

/** `$2.41`, or `—` when the provider reports no cost (codex/gemini). */
function formatCost(usd: number | undefined): string {
    if (usd == null) return "—";
    return usd < 0.01 ? "<$0.01" : `$${usd.toFixed(2)}`;
}

export const AgentSessionStats = (props: AgentSessionStatsProps): JSX.Element => {
    const [open, setOpen] = createSignal(false);
    const [floatingStyle, setFloatingStyle] = createSignal("");
    const [archiveBusy, setArchiveBusy] = createSignal(false);
    const [exportBusy, setExportBusy] = createSignal(false);
    const [restoreBusy, setRestoreBusy] = createSignal(false);
    let referenceEl: HTMLButtonElement | undefined;
    let floatingEl: HTMLDivElement | undefined;
    let cleanupAutoUpdate: (() => void) | undefined;

    const blockAtom = () => props.blockAtom();
    const archived = () => isSessionArchived(blockAtom);
    const lineCount = () => sessionLineCount(blockAtom);
    // Same gate AgentControlBar used: session management is Claude-only.
    const canManage = () => props.providerId === "claude";

    const totals = () => props.sessionTotals ?? null;

    const contextPct = (): number | null => {
        const t = props.contextTokens;
        const w = props.contextWindow;
        if (t == null || t <= 0 || w == null || w <= 0) return null;
        return Math.min(100, (t / w) * 100);
    };

    const cacheShare = (): number | null => {
        const t = totals();
        if (!t?.input_tokens || t.cache_read_input_tokens == null) return null;
        return (t.cache_read_input_tokens / t.input_tokens) * 100;
    };

    const focusWithin = (): boolean => {
        const active = document.activeElement;
        if (!active) return false;
        return !!(referenceEl?.contains(active) || floatingEl?.contains(active));
    };

    const handleClickOutside = (e: MouseEvent) => {
        const target = e.target as Node;
        if (referenceEl?.contains(target) || floatingEl?.contains(target)) return;
        setOpen(false);
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            e.preventDefault();
            setOpen(false);
            referenceEl?.focus();
        }
    };

    const handleFocusChange = () => {
        if (!focusWithin()) setOpen(false);
    };

    createEffect(() => {
        if (!open()) return;
        document.addEventListener("mousedown", handleClickOutside);
        document.addEventListener("keydown", handleKeyDown, true);
        document.addEventListener("focusin", handleFocusChange);
        onCleanup(() => {
            document.removeEventListener("mousedown", handleClickOutside);
            document.removeEventListener("keydown", handleKeyDown, true);
            document.removeEventListener("focusin", handleFocusChange);
            // Tied to the OPEN lifetime, not the component's: the portaled
            // panel is gone once closed, so leaving autoUpdate armed would
            // keep scroll/resize observers repositioning a detached element
            // for the rest of the pane's life (Codex P2 on #3435).
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = undefined;
            floatingEl = undefined;
        });
    });
    onCleanup(() => cleanupAutoUpdate?.());

    const updatePosition = async () => {
        if (!referenceEl || !floatingEl) return;
        const pos = await computeMenuPosition(
            { anchor: referenceEl, placement: "top-start", avoidNativePanes: false },
            floatingEl
        );
        setFloatingStyle(styleToString(pos.style));
    };

    const registerFloating = (el: HTMLDivElement) => {
        floatingEl = el;
        requestAnimationFrame(() => {
            if (!(referenceEl instanceof Element) || !(floatingEl instanceof Element)) return;
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(referenceEl, floatingEl, updatePosition);
            assertMenuInPaintableArea(el, "agent-session-stats");
        });
    };

    const run = async (
        busy: () => boolean,
        setBusy: (v: boolean) => void,
        what: string,
        fn: () => Promise<void>
    ) => {
        if (busy()) return;
        setBusy(true);
        try {
            await fn();
        } catch (e) {
            console.error(`session:${what} failed:`, e);
        } finally {
            setBusy(false);
        }
    };

    const statRow = (label: string, value: JSX.Element) => (
        <div class="agent-session-stats-row">
            <span class="agent-session-stats-key">{label}</span>
            <span class="agent-session-stats-val">{value}</span>
        </div>
    );

    return (
        <>
            <button
                type="button"
                ref={referenceEl}
                class={`agent-composer-strip-ctx agent-session-stats-trigger ${props.ctxClass}`}
                data-strip-button
                title={props.title}
                aria-haspopup="dialog"
                aria-expanded={open()}
                onClick={() => setOpen(!open())}
            >
                {props.label}
            </button>
            <Show when={open()}>
                <Portal>
                    <div
                        ref={registerFloating}
                        class="menu agent-session-stats-panel"
                        style={floatingStyle()}
                        data-pane-overlay
                        role="dialog"
                        aria-label="Session stats"
                    >
                        <div class="agent-session-stats-section">Session</div>

                        <Show when={contextPct() != null}>
                            {statRow(
                                "Context",
                                <>
                                    {formatCompactNumber(props.contextTokens ?? 0)} /{" "}
                                    {formatCompactNumber(props.contextWindow ?? 0)} (
                                    {contextPct()!.toFixed(0)}%)
                                </>
                            )}
                        </Show>

                        <Show when={totals()}>
                            {statRow(
                                "Cost",
                                <>
                                    {formatCost(totals()?.cost_usd)}
                                    <Show when={totals()?.num_turns}>
                                        {" · "}
                                        {totals()!.num_turns} turns
                                    </Show>
                                    <Show when={totals()?.duration_ms}>
                                        {" · "}
                                        {formatDuration(totals()!.duration_ms!)}
                                    </Show>
                                </>
                            )}
                            {statRow(
                                "Tokens",
                                <>
                                    {formatCompactNumber(totals()?.input_tokens ?? 0)} in
                                    <Show when={cacheShare() != null}>
                                        {" "}
                                        ({cacheShare()!.toFixed(0)}% cached)
                                    </Show>
                                    {" · "}
                                    {formatCompactNumber(totals()?.output_tokens ?? 0)} out
                                </>
                            )}
                        </Show>

                        <Show when={lineCount() > 0}>
                            {statRow(
                                "History",
                                <span title={`${formatExactNumber(lineCount())} lines`}>
                                    {formatCompactNumber(lineCount())} lines ·{" "}
                                    {formatAgo(sessionLastActivityMs(blockAtom))}
                                </span>
                            )}
                        </Show>

                        <Show when={archived()}>
                            {statRow(
                                "Archived",
                                new Date(sessionArchivedAt(blockAtom)).toLocaleString()
                            )}
                        </Show>

                        <Show when={canManage() && (lineCount() > 0 || archived())}>
                            <div class="agent-session-stats-actions">
                                <Show
                                    when={archived()}
                                    fallback={
                                        <button
                                            class="agent-session-btn agent-session-btn-archive"
                                            disabled={archiveBusy()}
                                            onClick={() =>
                                                void run(archiveBusy, setArchiveBusy, "archive", () =>
                                                    archiveSession(props.blockId)
                                                )
                                            }
                                            title="Compress and archive this session's history to free disk space, then start fresh"
                                        >
                                            {archiveBusy() ? "Archiving…" : "Archive"}
                                        </button>
                                    }
                                >
                                    <button
                                        class="agent-session-btn agent-session-btn-restore"
                                        disabled={restoreBusy()}
                                        onClick={() =>
                                            void run(restoreBusy, setRestoreBusy, "restore", () =>
                                                restoreSession(props.blockId)
                                            )
                                        }
                                        title="Restore this session's history from the archive"
                                    >
                                        {restoreBusy() ? "Restoring…" : "Restore"}
                                    </button>
                                </Show>
                                <button
                                    class="agent-session-btn agent-session-btn-export"
                                    disabled={exportBusy()}
                                    onClick={() =>
                                        void run(exportBusy, setExportBusy, "export", () =>
                                            exportSession(props.blockId)
                                        )
                                    }
                                    title="Download this session's history as a .jsonl file"
                                >
                                    {exportBusy() ? "Exporting…" : "Export"}
                                </button>
                            </div>
                        </Show>
                    </div>
                </Portal>
            </Show>
        </>
    );
};

AgentSessionStats.displayName = "AgentSessionStats";
