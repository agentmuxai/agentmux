// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Context deliveries: content the agent is given without the user typing it,
 * shown as one card with a row per item instead of as raw text.
 * docs/specs/SPEC_CONTEXT_DELIVERY_2026_09_30.md.
 *
 * CD1 covers Claude Code's compaction summary. After `/compact` (or an
 * auto-compact) the CLI writes its own summary of the conversation and echoes
 * it on stdout as a plain-string user frame, straight after the
 * `compact_boundary`. The translator used to turn that into a `user_message`,
 * so the pane showed a 20 KB summary as if the user had typed it. AgentMux
 * sends nothing here: the summary is already in the model's context, and this
 * is display only (spec §3.3).
 *
 * Shared by useAgentStream.ts (live) and parseHistoryLines.ts (replay) so the
 * two can't drift: both intercept the raw frame before the translator, which
 * also keeps it from ending a hidden-turn window in the stream parser.
 */

import { estimateTokenCount } from "@/util/format-count";
import { contextCompactedNodeId, type CompactBoundaryData } from "./compact-boundary";
import type { ContextDeliveryNode } from "./types";

/** How Claude Code opens the summary. One of two signals; see `isSummaryFrame`. */
export const COMPACTION_SUMMARY_PREFIX = "This session is being continued from a previous conversation";

const EXCERPT_MAX = 160;

const REASON_LABEL: Record<ContextDeliveryNode["reason"], string> = {
    startup: "new session",
    clear: "after /clear",
    compaction: "after compaction",
    resume_fresh: "session continued",
};

/** The card's one-line title (spec §3.2). */
export function contextDeliveryTitle(node: ContextDeliveryNode): string {
    const only = node.items.length === 1 ? node.items[0] : null;
    if (only?.kind === "compaction_summary") {
        const how = node.trigger === "auto" ? "auto-compact" : node.trigger === "manual" ? "manual compact" : null;
        return how ? `Agent given a summary of the conversation (${how})` : "Agent given a summary of the conversation";
    }
    const n = node.items.length;
    return `Given to the agent · ${REASON_LABEL[node.reason]} · ${n} ${n === 1 ? "item" : "items"}`;
}

/**
 * Stable id for a summary card: the frame's own `uuid` when it has one, so the
 * live copy and every replayed copy (whichever history page it lands on) share
 * it; otherwise derived from its boundary, or its own timestamp and size.
 */
export function compactionSummaryNodeId(rawEvent: any, boundary: CompactBoundaryData | null, text: string): string {
    if (typeof rawEvent?.uuid === "string" && rawEvent.uuid) return `context-delivery-compaction-${rawEvent.uuid}`;
    if (boundary) return `context-delivery-compaction-${contextCompactedNodeId(boundary)}`;
    const ts = typeof rawEvent?.timestamp === "string" ? rawEvent.timestamp : "notime";
    return `context-delivery-compaction-${ts}-${text.length}`;
}

/**
 * The first line of real content, for the collapsed card: skips the
 * "This session is being continued…" preamble, labels such as "Summary:",
 * and numbered headings such as "1. Primary Request and Intent:".
 */
export function compactionSummaryExcerpt(text: string): string {
    const lines = text.split(/\r?\n/).map((l) => l.trim());
    const line = lines.find(
        (l) =>
            l.length > 0 &&
            !l.startsWith(COMPACTION_SUMMARY_PREFIX) &&
            !l.startsWith("This session is being continued") &&
            // Headings and labels end with a colon: "Summary:", "1. Intent:".
            !l.endsWith(":") &&
            !l.startsWith("#"),
    );
    if (!line) return "";
    const plain = line.replace(/^[-*]\s+/, "").replace(/\*\*/g, "");
    return plain.length <= EXCERPT_MAX ? plain : `${plain.slice(0, EXCERPT_MAX - 1).trimEnd()}…`;
}

function stringContent(rawEvent: any): string | null {
    if (rawEvent?.type !== "user") return null;
    const content = rawEvent.message?.content;
    return typeof content === "string" ? content : null;
}

/**
 * Two independent signals, so a change to either the CLI's wording or its
 * flag doesn't bring the raw summary back (spec §3.3). Right after a boundary
 * either is enough. Without one in view both are required: history pages are
 * parsed separately, newest first, so a summary at the top of a page can't
 * see the boundary that ends the page before it.
 */
function isSummaryFrame(rawEvent: any, text: string, afterBoundary: boolean): boolean {
    const flagged = rawEvent.isSynthetic === true;
    const worded = text.startsWith(COMPACTION_SUMMARY_PREFIX);
    return afterBoundary ? flagged || worded : flagged && worded;
}

function parseTime(value: unknown): number {
    return typeof value === "string" ? Date.parse(value) : NaN;
}

/**
 * Recognises Claude Code's summary frame. Feed it every boundary
 * (`noteBoundary`) and every other raw frame (`take`); `take` returns a node
 * for the summary and null for everything else. A boundary is forgotten once
 * the next user-text frame or the assistant arrives, so a later user message
 * can never be paired with it.
 */
export class CompactionSummaryTracker {
    private pending: CompactBoundaryData | null = null;

    noteBoundary(boundary: CompactBoundaryData): void {
        this.pending = boundary;
    }

    /** `now` stamps a frame when neither it nor its boundary has a timestamp. */
    take(rawEvent: any, now: number): ContextDeliveryNode | null {
        if (!rawEvent || typeof rawEvent !== "object") return null;
        if (rawEvent.type === "assistant") {
            this.pending = null;
            return null;
        }
        const text = stringContent(rawEvent);
        // Tool results (array content) and system frames don't end the wait.
        if (text == null) return null;
        const boundary = this.pending;
        this.pending = null;
        if (!isSummaryFrame(rawEvent, text, boundary != null)) return null;

        const own = parseTime(rawEvent.timestamp);
        const fromBoundary = parseTime(boundary?.frameTimestamp);
        return {
            type: "context_delivery",
            id: compactionSummaryNodeId(rawEvent, boundary, text),
            reason: "compaction",
            ...(boundary ? { trigger: boundary.trigger } : {}),
            timestamp: !Number.isNaN(own) ? own : !Number.isNaN(fromBoundary) ? fromBoundary : now,
            items: [
                {
                    kind: "compaction_summary",
                    name: "Conversation summary (written by Claude Code)",
                    sizeBytes: new TextEncoder().encode(text).length,
                    tokens: estimateTokenCount(text),
                    excerpt: compactionSummaryExcerpt(text),
                    body: text,
                },
            ],
        };
    }
}
