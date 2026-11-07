// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a markdown row keeps: the throttled view of its streaming text, and the peek tooltip's text and hover.
 */

import { estimateTokenCount, formatCompactNumber } from "@/util/format-count";
import { formatExactTime, formatTimeAgo } from "@/util/format-time";
import { createEffect, createMemo, createSignal, onCleanup, type Accessor } from "solid-js";
import { useTick } from "@/app/hook/useTick";
import { useNodePeek } from "../hooks/useNodePeek";
import { useAgentDormant } from "../agent-dormancy";
import type { MarkdownNode, ViewModeTag } from "../../../agent-model/types";

export interface MarkdownBlockProps {
    node: MarkdownNode & ViewModeTag;
    /** A foldable thought (canceled, or folded by the pane's view): the user opened it (`documentState.pinnedNodes`). */
    pinned?: boolean;
    /** A foldable thought: toggle `pinned` (the header click). */
    onTogglePin?: () => void;
}

export type StreamedView = { text: string; streaming: boolean };

// During streaming the message content grows ~60x/s. Re-parsing the whole
// document (including syntax highlighting) on every frame is O(n^2) and
// starves keystrokes (see ANALYSIS_AGENT_PANE_TYPING_LATENCY_2026_05_30.md).
// Coalesce: commit at most one intermediate render per window while content
// keeps arriving, marked `streaming` so <Markdown> renders the open trailing
// block without syntax highlighting, then one settled render once updates
// stop. "Settled" is a guess from a quiet window — real streams pause longer
// than this mid-message — so it must stay cheap to be wrong: flipping
// `streaming` re-renders only the trailing block, never the closed blocks
// before it (MarkdownBlock.stream-pauses.test.tsx). This is a perf rate-limit
// on an expensive render, not a timer papering over a race.
const STREAM_RENDER_MS = 90;

/**
 * Throttled view of the streaming content + whether it is still streaming.
 * A settled/static message renders fully (highlighted) immediately; a
 * fast stream renders cheap intermediates and a full final.
 */
export function createStreamedView(props: MarkdownBlockProps): Accessor<StreamedView> {
    // Value-based equality: `Markdown` subscribes to this signal via the
    // `streaming` prop, so a fresh-but-equal object would needlessly re-parse
    // static / history blocks on mount (and on the trailing no-op write).
    // With this, a same-value setView is a no-op.
    const [view, setView] = createSignal<StreamedView>(
        { text: props.node.content, streaming: false },
        { equals: (a, b) => a.text === b.text && a.streaming === b.streaming },
    );
    let lastCommitAt = 0;
    let streaming = false;
    let trailing: ReturnType<typeof setTimeout> | undefined;

    // Keep-alive leaves a backgrounded agent tab MOUNTED (visibility, not
    // existence — `pane-leaf-chrome.tsx`), so without this gate every
    // backgrounded-but-streaming pane keeps parsing markdown on the same main
    // thread as the pane being typed into: four busy agents, four panes' worth
    // of work, one thread. See
    // `docs/analysis/ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md` §4.
    //
    // Deferred, not dropped. Only the render is skipped — the document store
    // behind `props.node` stays current, so nothing is lost and off-pane
    // activity indicators keep updating. Reading `dormant()` inside the effect
    // keeps it a dependency, so becoming visible re-runs this and commits
    // whatever arrived meanwhile without needing a further update.
    const dormant = useAgentDormant();

    let wasDormant = false;
    createEffect(() => {
        const text = props.node.content; // dep: re-runs on each streamed update
        if (dormant()) {
            // Don't leave a trailing render armed to paint an invisible pane.
            if (trailing) clearTimeout(trailing);
            wasDormant = true;
            return;
        }
        const now = performance.now();
        if (trailing) clearTimeout(trailing);
        if (wasDormant) {
            // Shown again: what arrived meanwhile is drawn once, settled, rather
            // than drawn and then drawn again highlighted a moment later, which
            // changed the row's height twice
            // (SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §3.3).
            wasDormant = false;
            streaming = false;
            lastCommitAt = now;
            setView({ text, streaming: false });
            return;
        }
        if (now - lastCommitAt >= STREAM_RENDER_MS) {
            // Leading edge: cheap intermediate (open tail un-highlighted
            // mid-stream). The first update after a settle is not yet known to
            // be a stream, so it renders full, as a one-off change would.
            lastCommitAt = now;
            setView({ text, streaming });
        }
        streaming = true;
        // Trailing edge: once updates stop for a window, render settled.
        trailing = setTimeout(() => {
            streaming = false;
            lastCommitAt = performance.now();
            setView({ text: props.node.content, streaming: false });
        }, STREAM_RENDER_MS);
    });
    onCleanup(() => {
        if (trailing) clearTimeout(trailing);
    });
    return view;
}

/**
 * Peek tooltip (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md §2.4,
 * extended to every markdown block — thinking clumps AND regular
 * assistant text — by SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25;
 * "regular assistant text... remain out of scope" no longer holds).
 * Mirrors ToolBlock.tsx's time + estimate pattern. No duration line —
 * deriving one from the next node's timestamp needs a new prop threaded
 * through the virtualization list's windowed-rows/streaming-buffer
 * machinery (AgentDocumentVirtualList.tsx), which is performance-critical
 * and carefully tuned; deferred as a follow-up rather than risked here
 * for a nice-to-have (§4 resolution 1 of that spec still calls for it
 * eventually).
 *
 * reagent P2 on PR #2392 (3rd round): every MOUNTED block used to call
 * peekTick() regardless of hover state — a memo only subscribes to what
 * it actually reads during a given run, so gating on peekPanelVisible()
 * BEFORE reading peekTick() (not after) means only the block whose panel
 * is showing subscribes to the shared 1s ticker (panelVisible, not
 * isPeeking: the panel can stay open after the row is left). Still applies now that every
 * markdown block (not just thinking ones) participates.
 */
export function createMarkdownPeek(props: MarkdownBlockProps) {
    const peekTick = useTick(1000);
    const { isPeeking, panelVisible: peekPanelVisible, rowEl: peekRowEl, setRowEl: setPeekRowEl, handlePeekEnter, handlePeekLeave } = useNodePeek();
    const peekTimeText = createMemo(() => {
        if (!peekPanelVisible()) return null;
        peekTick();
        const ts = props.node.timestamp;
        if (ts == null) return null;
        return `${formatExactTime(ts)} · ${formatTimeAgo(ts)}`;
    });
    const peekEstimateText = createMemo(() => {
        const count = estimateTokenCount(props.node.content);
        return count > 0 ? `~${formatCompactNumber(count)} tok (est.)` : null;
    });
    return { isPeeking, peekRowEl, setPeekRowEl, handlePeekEnter, handlePeekLeave, peekTimeText, peekEstimateText };
}

export type MarkdownPeek = ReturnType<typeof createMarkdownPeek>;
