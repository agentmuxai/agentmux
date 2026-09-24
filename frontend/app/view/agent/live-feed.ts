// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The live feed: the agent pane keeps the turn in flight plus the last few
 * finished turns; older finished turns ROLL OFF — they leave the pane's
 * stores and stay readable in History, which reads the same transcript
 * (SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §0, §6.9).
 *
 * This module is the pure part: splitting the document into turns and
 * deciding which ranges may go. The pane supplies what is on screen and
 * dispatches the result as one `RollOff` reducer command.
 */

import type { AgentPaneLayoutState } from "@/app/store/agent-pane-layout/types";
import { positions, windowRangeOf } from "@/app/store/agent-pane-layout/reducer";
import type { DocumentNode } from "./types";
import { isNodeInProgress, nodeBytes } from "./virtualization/streaming-buffer";

/** Finished turns kept by default (`agent:livefeedturns`). */
export const LIVE_FEED_DEFAULT_TURNS = 3;
/** Finished turns kept are also capped by size; at least one always stays. */
export const LIVE_FEED_MAX_FINISHED_BYTES = 1_000_000;

/**
 * Panes whose transcript carries everything the feed shows, user messages
 * included: Claude under the PERSISTENT controller (it writes each stdin line
 * back, Phase 5a-3c), and the Gemini family (its CLI echoes the message into
 * its own output; rendered since #3620). Claude run by the per-turn
 * subprocess controller (muxcode, container agents), Codex, Kimi and ACP
 * never persist the user's message, so rolling their turns off would lose
 * it: they keep today's behaviour until the journal lands (§6.9).
 */
export function liveFeedSupported(outputFormat: string | undefined, controller?: string): boolean {
    if (outputFormat === "gemini-json") return true;
    if (outputFormat === "claude-stream-json") return controller === "persistent";
    return false;
}

/** Setting → finished turns kept; anything but a positive integer is the default. */
export function resolveLiveFeedTurns(setting: unknown): number {
    return typeof setting === "number" && Number.isInteger(setting) && setting >= 1
        ? setting
        : LIVE_FEED_DEFAULT_TURNS;
}

/** A turn: nodes `[start, end)`. */
export interface Turn {
    start: number;
    end: number;
}

/**
 * Turns start at a `user_message` (parser-produced or optimistic); nodes
 * before the first one form a leading turn. Same rule as the turn-scoped
 * tail (`turnScopedFrontier`, Phase 3b).
 */
export function splitTurns(nodes: readonly DocumentNode[]): Turn[] {
    const turns: Turn[] = [];
    let start = 0;
    for (let i = 1; i < nodes.length; i++) {
        if (nodes[i].type === "user_message") {
            turns.push({ start, end: i });
            start = i;
        }
    }
    if (nodes.length > 0) turns.push({ start, end: nodes.length });
    return turns;
}

/**
 * Content the transcript can't rebuild, so its turn must stay (§6.9):
 * AgentMux's in-pane shell runs (a memory ring on the backend — not the
 * agent's Bash tool, which is an ordinary tool node), and an AskUserQuestion
 * whose answer was filled in optimistically (replay has no source for it).
 * Live-only decoration rows (stderr, notifications, "Interrupted", heuristic
 * compaction markers) are ephemeral, not content: they roll off with their
 * turn, as any reload drops them today.
 */
export function blocksRollOff(node: DocumentNode): boolean {
    if (node.type === "shell") return true;
    if (node.type === "tool") {
        return node.answerText != null || node.questionText != null || node.timeoutNote != null;
    }
    return false;
}

export interface RollOffInput {
    /** Finished turns to keep (≥ 1). */
    keepTurns: number;
    maxFinishedBytes?: number;
    /** Ids of nodes intersecting the viewport. */
    visibleIds: ReadonlySet<string>;
    /** Ids whose turns stay regardless (nodes the user pinned). */
    keepIds?: ReadonlySet<string>;
    /** Whether the reader follows the bottom. */
    pinned: boolean;
}

export interface RollOffPlan {
    /** Node index ranges `[start, end)` to remove (ascending, non-adjacent) and the whole turns in each. */
    ranges: Array<{ start: number; end: number; turns: number }>;
    /** Whole turns removed. */
    turns: number;
    /** Nodes removed. */
    nodes: number;
    /** Older turns kept only because they hold content the transcript can't rebuild. */
    blockedTurns: number;
}

/**
 * Which finished turns roll off now, or `null` for none.
 *
 * Kept: the turn in flight (the last), the newest `keepTurns` finished turns
 * (fewer if they exceed the byte cap, never fewer than one), any turn with a
 * node on screen, any turn still in progress, and blocked turns. Of the rest:
 * while pinned, all go (the pin holds the bottom, so nothing on screen
 * moves); while the reader is scrolled up, only those wholly BELOW what they
 * are reading go — removing content below the viewport moves nothing, and
 * turns above it wait until they follow the bottom again (new turns only
 * arrive below, so what waits doesn't grow).
 */
export function planRollOff(nodes: readonly DocumentNode[], input: RollOffInput): RollOffPlan | null {
    const turns = splitTurns(nodes);
    if (turns.length < 2) return null;
    const maxBytes = input.maxFinishedBytes ?? LIVE_FEED_MAX_FINISHED_BYTES;
    const keepTurns = Math.max(1, input.keepTurns);

    const holds = (t: Turn, pred: (n: DocumentNode) => boolean): boolean => {
        for (let i = t.start; i < t.end; i++) if (pred(nodes[i])) return true;
        return false;
    };
    const turnBytes = (t: Turn): number => {
        let b = 0;
        for (let i = t.start; i < t.end; i++) b += nodeBytes(nodes[i]);
        return b;
    };

    // The newest finished turns kept, walking back from the one before the
    // turn in flight.
    let firstKept = turns.length - 1;
    let kept = 0;
    let bytes = 0;
    for (let k = turns.length - 2; k >= 0 && kept < keepTurns; k--) {
        const b = turnBytes(turns[k]);
        if (kept > 0 && bytes + b > maxBytes) break;
        kept++;
        bytes += b;
        firstKept = k;
    }

    let lastVisible = -1;
    for (let i = 0; i < nodes.length; i++) if (input.visibleIds.has(nodes[i].id)) lastVisible = i;
    // Scrolled up with nothing of the older turns on screen (reading the turn
    // in flight, or the view isn't laid out yet): nothing is below the reader.
    if (!input.pinned && lastVisible < 0) return null;

    const ranges: Array<{ start: number; end: number; turns: number }> = [];
    let removedTurns = 0;
    let removedNodes = 0;
    let blockedTurns = 0;
    for (let k = 0; k < firstKept; k++) {
        const t = turns[k];
        if (holds(t, isNodeInProgress)) continue;
        if (holds(t, blocksRollOff)) {
            blockedTurns++;
            continue;
        }
        if (holds(t, (n) => input.visibleIds.has(n.id) || input.keepIds?.has(n.id) === true)) continue;
        if (!input.pinned && t.start <= lastVisible) continue;
        const prev = ranges[ranges.length - 1];
        if (prev && prev.end === t.start) {
            prev.end = t.end;
            prev.turns++;
        } else {
            ranges.push({ start: t.start, end: t.end, turns: 1 });
        }
        removedTurns++;
        removedNodes += t.end - t.start;
    }
    if (removedTurns === 0) return null;
    return { ranges, turns: removedTurns, nodes: removedNodes, blockedTurns };
}

/**
 * Ids of the virtualized rows intersecting the viewport, from the layout
 * store's own positions and scroll offset — no DOM read. Finished turns live
 * in the virtualized head (Phase 3b: the always-mounted tail holds only the
 * turn in flight, which is never rolled off), so these are the rows that
 * matter.
 */
export function visibleIdsOf(state: AgentPaneLayoutState | null): Set<string> {
    const out = new Set<string>();
    if (!state || state.viewportPx <= 0) return out;
    const pos = positions(state);
    const range = windowRangeOf(pos, state.scrollTop, state.viewportPx, 0);
    for (let i = range.startIndex; i <= range.endIndex; i++) out.add(pos[i].nodeId);
    return out;
}

/** The history-link row's text (top of the feed, or a gap between kept turns). */
export function historyLinkLabel(node: { earlierTurns?: number; gap?: boolean }): string {
    if (node.gap) return "Earlier turns are in History —";
    if (node.earlierTurns != null && node.earlierTurns > 0) {
        return `${node.earlierTurns} earlier ${node.earlierTurns === 1 ? "turn" : "turns"} —`;
    }
    return "Earlier conversations preserved —";
}
