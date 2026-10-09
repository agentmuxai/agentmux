// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The raw stream frames the pane reads itself, before the provider
 * translator: frames the translator has no StreamEvent shape for (a
 * compaction boundary, AgentMux's own session outcome and notices) and the
 * ones it would show as something else (Claude Code's compaction summary
 * would read as a user message).
 *
 * The live stream (useAgentStream.ts) and history replay
 * (parseHistoryLines.ts) both run every frame through `interceptFrame`, so
 * they match frames in the same order and do the shared steps the same way:
 * close the parser's open block and place the jekts it held before an
 * intercepted node, and end a hidden memory re-injection's suppression at a
 * session boundary. What each path then does with a frame (dispatch to the
 * pane, or put the node in a list) is its sink's.
 */

import { parseCliNoticeFrame } from "./cli-notice";
import { parseCompactBoundaryFrame, type CompactBoundaryData } from "./compact-boundary";
import type { CompactionSummaryTracker } from "./context-delivery";
import { isMemoryInjectedFrame } from "./memory-injected";
import { parseSessionOutcomeFrame, type SessionOutcomeData } from "@/app/store/agent-document/session-outcome";
import type { ClaudeCodeStreamParser } from "./stream-parser";
import type { TaskWakeDetector } from "./task-wake";
import type { DocumentNode } from "./types";

/** The per-stream state the intercepts read and advance. */
export interface FrameInterceptState {
    parser: ClaudeCodeStreamParser;
    compactionSummaries: CompactionSummaryTracker;
    detectTaskWake: TaskWakeDetector;
}

/** What a path does with what the intercepts found. */
export interface FrameSink {
    /** Add a node. `update`: a later frame with its id carries newer state (a
     *  CLI install's progress), so it replaces the node already there; else a
     *  node already there (a frame seen again after a resubscribe) is kept. */
    node(node: DocumentNode, update: boolean): void;
    /** Place the jekts the parser released, before the next node. */
    placeReleased(): void;
    /** A compaction boundary; `data` is null when its metadata didn't parse. */
    compactBoundary(data: CompactBoundaryData | null, raw: Record<string, unknown>): void;
    /** AgentMux's session-outcome marker; `data` is null when it didn't parse. */
    sessionOutcome(data: SessionOutcomeData | null, raw: Record<string, unknown>): void;
    /** The notice for memory the `SessionStart` hook delivered. The path closes
     *  the parser's block itself: live skips a card its hidden turn will show. */
    memoryInjected(raw: Record<string, unknown>): void;
}

/**
 * Run one frame through the intercepts, in order. Returns true when an
 * intercept consumed it (the caller stops there), false when it goes on to
 * the translator. `at` is the frame's receive time (unix ms), when known.
 */
export function interceptFrame(raw: Record<string, unknown>, st: FrameInterceptState, sink: FrameSink, at: number | undefined): boolean {
    // Close the open text/thinking block, so what follows an intercepted
    // frame never merges into what came before it (#1104's bug class), and
    // place the jekts the block held.
    const closeBlock = (): void => {
        st.parser.flushPending();
        sink.placeReleased();
    };

    // Observe only: the frame keeps flowing.
    const wake = st.detectTaskWake(raw, at);
    if (wake) sink.node(wake, false);

    if (raw.type === "system" && raw.subtype === "compact_boundary") {
        // Closed even when the metadata doesn't parse: it is still a real
        // boundary in the conversation.
        closeBlock();
        const data = parseCompactBoundaryFrame(raw);
        if (data) st.compactionSummaries.noteBoundary(data);
        sink.compactBoundary(data, raw);
        return true;
    }

    if (raw.type === "system" && raw.subtype === "agentmux_session_outcome") {
        closeBlock();
        // Any session boundary ends a hidden memory re-injection's
        // suppression: left on, a re-injection that was a session's last turn
        // would hide every event of the next session (#3502).
        st.parser.clearHiddenReinjectionState();
        sink.sessionOutcome(parseSessionOutcomeFrame(raw), raw);
        return true;
    }

    // Claude Code's compaction summary: a card, not a user message
    // (SPEC_CONTEXT_DELIVERY_2026_09_30.md §3.3).
    const summary = st.compactionSummaries.take(raw, at ?? 0);
    if (summary) {
        closeBlock();
        sink.node(summary, false);
        return true;
    }

    // CLI install / version-change notices. An install's later frame has the
    // same id, so it updates the "installing" row.
    const cliNode = parseCliNoticeFrame(raw, at ?? 0);
    if (cliNode) {
        closeBlock();
        sink.node(cliNode, true);
        return true;
    }

    // SPEC_GLOBAL_MEMORY_DELIVERY_2026_09_27.md §7 P2.
    if (isMemoryInjectedFrame(raw)) {
        sink.memoryInjected(raw);
        return true;
    }

    return false;
}
