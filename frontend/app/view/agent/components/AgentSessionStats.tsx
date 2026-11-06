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
 * Panel mechanics (open/close, outside-click, focus-out, Escape) deliberately
 * mirror `AgentRuntimeDropup`, the strip's other panel, with the same
 * "listeners live only while open" effect scoping. Placement, chrome zoom and
 * the airspace cut are AnchoredPopover's.
 */

import { createEffect, createSignal, onCleanup, Show, type Accessor, type JSX, type Setter } from "solid-js";

import { AnchoredPopover } from "@/app/element/anchored-popover";
import { Button } from "@/app/element/ui/Button";
import { formatCompactNumber, formatExactNumber } from "@/util/format-count";
import { formatDurationShort, formatTimeAgo } from "@/util/format-time";

import { tip } from "@/app/element/tip";
import type { SessionStats } from "../../../agent-model/types";
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

interface AgentSessionStatsProps {
    blockId: string;
    blockAtom: BlockAccessor;
    providerId: string;
    /** Tokens in context (the strip's validated reading). */
    contextTokens?: number | null;
    /** The model's context window. undefined = unknown. */
    contextWindow?: number;
    /** Where those numbers come from (`contextReadingNote`). */
    contextNote?: string;
    /** Cumulative cost/tokens/duration across this pane's completed turns. */
    sessionTotals?: SessionStats | null;
    /** The strip's own ctx band class, applied to the trigger. */
    ctxClass: string;
    /** Hover text the strip already computed for the reading. */
    title?: string;
    /** The reading itself, e.g. `104k / 200k`. */
    label: string;
}

/** `3m ago` / `2h ago` / `—` when never. */
function formatAgo(ts: number): string {
    return ts ? formatTimeAgo(ts) : "—";
}

/**
 * What the "Processed" row counts. Every API call re-sends the whole
 * conversation, so these totals grow far faster than the context and are
 * not a context size (the Context row above is).
 */
const PROCESSED_TITLE =
    "Tokens sent and received across every API call in this pane's turns. Each call re-sends the conversation, " +
    "so input here counts the context once per call: it tracks cost, not how full the context is.";

/** `$2.41`, or `—` when the provider reports no cost (codex/gemini). */
function formatCost(usd: number | undefined): string {
    if (usd == null) return "—";
    return usd < 0.01 ? "<$0.01" : `$${usd.toFixed(2)}`;
}

/** The panel's open flag, the busy flags of its three actions, the two elements it measures focus against, and the
 *  listeners that live only while it is open (kept here so the same functions are added and removed). */
interface StatsState {
    props: AgentSessionStatsProps;
    open: Accessor<boolean>;
    setOpen: Setter<boolean>;
    archiveBusy: Accessor<boolean>;
    setArchiveBusy: Setter<boolean>;
    exportBusy: Accessor<boolean>;
    setExportBusy: Setter<boolean>;
    restoreBusy: Accessor<boolean>;
    setRestoreBusy: Setter<boolean>;
    referenceEl?: HTMLButtonElement;
    floatingEl?: HTMLDivElement;
    onMouseDown: (e: MouseEvent) => void;
    onKeyDown: (e: KeyboardEvent) => void;
    onFocusIn: () => void;
}

function createStatsState(props: AgentSessionStatsProps): StatsState {
    const [open, setOpen] = createSignal(false);
    const [archiveBusy, setArchiveBusy] = createSignal(false);
    const [exportBusy, setExportBusy] = createSignal(false);
    const [restoreBusy, setRestoreBusy] = createSignal(false);
    const s: StatsState = {
        props,
        open,
        setOpen,
        archiveBusy,
        setArchiveBusy,
        exportBusy,
        setExportBusy,
        restoreBusy,
        setRestoreBusy,
        onMouseDown: (e) => handleClickOutside(s, e),
        onKeyDown: (e) => handleKeyDown(s, e),
        onFocusIn: () => handleFocusChange(s),
    };
    return s;
}

const blockAtom = (s: StatsState) => s.props.blockAtom();
const archived = (s: StatsState) => isSessionArchived(() => blockAtom(s));
const lineCount = (s: StatsState) => sessionLineCount(() => blockAtom(s));
// Same gate AgentControlBar used: session management is Claude-only.
const canManage = (s: StatsState) => s.props.providerId === "claude";

const totals = (s: StatsState) => s.props.sessionTotals ?? null;

const contextPct = (s: StatsState): number | null => {
    const t = s.props.contextTokens;
    const w = s.props.contextWindow;
    if (t == null || t <= 0 || w == null || w <= 0) return null;
    return Math.min(100, (t / w) * 100);
};

const cacheShare = (s: StatsState): number | null => {
    const t = totals(s);
    if (!t?.input_tokens || t.cache_read_input_tokens == null) return null;
    return (t.cache_read_input_tokens / t.input_tokens) * 100;
};

const focusWithin = (s: StatsState): boolean => {
    const active = document.activeElement;
    if (!active) return false;
    return !!(s.referenceEl?.contains(active) || s.floatingEl?.contains(active));
};

const handleClickOutside = (s: StatsState, e: MouseEvent) => {
    const target = e.target as Node;
    if (s.referenceEl?.contains(target) || s.floatingEl?.contains(target)) return;
    s.setOpen(false);
};

const handleKeyDown = (s: StatsState, e: KeyboardEvent) => {
    if (e.key === "Escape") {
        e.preventDefault();
        s.setOpen(false);
        s.referenceEl?.focus();
    }
};

const handleFocusChange = (s: StatsState) => {
    if (!focusWithin(s)) s.setOpen(false);
};

/** Listens for presses, keys and focus only while the panel is open. */
function watchOpen(s: StatsState) {
    createEffect(() => {
        if (!s.open()) return;
        document.addEventListener("mousedown", s.onMouseDown);
        document.addEventListener("keydown", s.onKeyDown, true);
        document.addEventListener("focusin", s.onFocusIn);
        onCleanup(() => {
            document.removeEventListener("mousedown", s.onMouseDown);
            document.removeEventListener("keydown", s.onKeyDown, true);
            document.removeEventListener("focusin", s.onFocusIn);
            s.floatingEl = undefined;
        });
    });
}

const run = async (busy: () => boolean, setBusy: (v: boolean) => void, what: string, fn: () => Promise<void>) => {
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

export const AgentSessionStats = (props: AgentSessionStatsProps): JSX.Element => {
    const s = createStatsState(props);
    watchOpen(s);

    return (
        <>
            <Button
                variant="link"
                ref={(el) => {
                    s.referenceEl = el;
                }}
                class={`agent-composer-strip-ctx agent-session-stats-trigger ${props.ctxClass}`}
                data-strip-button
                title={props.title}
                aria-haspopup="dialog"
                aria-expanded={s.open()}
                onClick={() => s.setOpen(!s.open())}
            >
                {props.label}
            </Button>
            <Show when={s.open()}>
                <StatsPanel s={s} />
            </Show>
        </>
    );
};

AgentSessionStats.displayName = "AgentSessionStats";

function StatsPanel(props: { s: StatsState }): JSX.Element {
    const s = props.s;
    return (
        <AnchoredPopover
            anchor={s.referenceEl}
            placement="top-start"
            shellClass="anchored-popover--menu"
            class="menu agent-session-stats-panel"
            role="dialog"
            aria-label="Session stats"
            ref={(el) => {
                s.floatingEl = el;
            }}
        >
            <div class="agent-session-stats-section">Session</div>

            <ContextRows s={s} />

            <TotalsRows s={s} />

            <HistoryRows s={s} />

            <Show when={canManage(s) && (lineCount(s) > 0 || archived(s))}>
                <SessionActions s={s} />
            </Show>
        </AnchoredPopover>
    );
}

/** The context reading, with where its numbers come from. */
function ContextRows(props: { s: StatsState }): JSX.Element {
    const s = props.s;
    return (
        <>
            <Show
                when={contextPct(s) != null}
                fallback={
                    <Show when={(s.props.contextTokens ?? 0) > 0}>
                        {statRow(
                            "Context",
                            <span {...tip(s.props.contextNote)}>
                                {formatCompactNumber(s.props.contextTokens ?? 0)} (window unknown)
                            </span>
                        )}
                    </Show>
                }
            >
                {statRow(
                    "Context",
                    <span {...tip(s.props.contextNote)}>
                        {formatCompactNumber(s.props.contextTokens ?? 0)} /{" "}
                        {formatCompactNumber(s.props.contextWindow ?? 0)} ({contextPct(s)!.toFixed(0)}%)
                    </span>
                )}
            </Show>
            <Show when={s.props.contextNote}>
                <div class="agent-session-stats-note">{s.props.contextNote}</div>
            </Show>
        </>
    );
}

/** Cumulative cost, turns and time, and the tokens processed. */
function TotalsRows(props: { s: StatsState }): JSX.Element {
    const s = props.s;
    return (
        <Show when={totals(s)}>
            {statRow(
                "Cost",
                <>
                    {formatCost(totals(s)?.cost_usd)}
                    <Show when={totals(s)?.num_turns}>
                        {" · "}
                        {totals(s)!.num_turns} turns
                    </Show>
                    <Show when={totals(s)?.duration_ms}>
                        {" · "}
                        {formatDurationShort(totals(s)!.duration_ms!)}
                    </Show>
                </>
            )}
            {statRow(
                "Processed",
                <span {...tip(PROCESSED_TITLE)}>
                    {formatCompactNumber(totals(s)?.input_tokens ?? 0)} in
                    <Show when={cacheShare(s) != null}> ({cacheShare(s)!.toFixed(0)}% cached)</Show>
                    {" · "}
                    {formatCompactNumber(totals(s)?.output_tokens ?? 0)} out
                </span>
            )}
        </Show>
    );
}

/** How long the session's history is and when it was archived. */
function HistoryRows(props: { s: StatsState }): JSX.Element {
    const s = props.s;
    return (
        <>
            <Show when={lineCount(s) > 0}>
                {statRow(
                    "History",
                    <span {...tip(`${formatExactNumber(lineCount(s))} lines`)}>
                        {formatCompactNumber(lineCount(s))} lines ·{" "}
                        {formatAgo(sessionLastActivityMs(() => blockAtom(s)))}
                    </span>
                )}
            </Show>

            <Show when={archived(s)}>
                {statRow("Archived", new Date(sessionArchivedAt(() => blockAtom(s))).toLocaleString())}
            </Show>
        </>
    );
}

/** Archive or Restore, and Export: each shows it is working and cannot be pressed again meanwhile. */
function SessionActions(props: { s: StatsState }): JSX.Element {
    const s = props.s;
    return (
        <div class="agent-session-stats-actions">
            <Show
                when={archived(s)}
                fallback={
                    <Button
                        density="compact"
                        disabled={s.archiveBusy()}
                        onClick={() =>
                            void run(s.archiveBusy, s.setArchiveBusy, "archive", () => archiveSession(s.props.blockId))
                        }
                        title="Compress and archive this session's history to free disk space, then start fresh"
                    >
                        {s.archiveBusy() ? "Archiving…" : "Archive"}
                    </Button>
                }
            >
                <Button
                    density="compact"
                    disabled={s.restoreBusy()}
                    onClick={() =>
                        void run(s.restoreBusy, s.setRestoreBusy, "restore", () => restoreSession(s.props.blockId))
                    }
                    title="Restore this session's history from the archive"
                >
                    {s.restoreBusy() ? "Restoring…" : "Restore"}
                </Button>
            </Show>
            <Button
                density="compact"
                disabled={s.exportBusy()}
                onClick={() => void run(s.exportBusy, s.setExportBusy, "export", () => exportSession(s.props.blockId))}
                title="Download this session's history as a .jsonl file"
            >
                {s.exportBusy() ? "Exporting…" : "Export"}
            </Button>
        </div>
    );
}
