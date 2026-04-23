// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentStream — SolidJS hook that subscribes to a block's subprocess output,
 * pipes it through the provider translator + stream parser, and feeds
 * the resulting DocumentNodes into SolidJS signals.
 */

import { getFileSubject, waveEventSubscribe } from "@/app/store/wps";
import * as WOS from "@/app/store/wos";
import { base64ToArray } from "@/util/util";
import { createEffect, onCleanup, onMount, untrack } from "solid-js";
import { createTranslator } from "./providers/translator-factory";
import type { PendingMessage, SignalPair } from "./state";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { DocumentNode, SessionStats, StreamingState, TurnTokens, UserMessageNode } from "./types";

const OutputFileName = "output";

interface UseAgentStreamOpts {
    blockId: string;
    outputFormat: string;
    documentAtom: SignalPair<DocumentNode[]>;
    streamingStateAtom: SignalPair<StreamingState>;
    sessionStatsAtom: SignalPair<SessionStats | null>;
    currentToolAtom: SignalPair<string | null>;
    turnTokensAtom: SignalPair<TurnTokens | null>;
    turnActiveAtom: SignalPair<boolean>;
    /**
     * True while a user-initiated stop (Esc → SIGINT) is pending. When
     * session_end arrives and this is true, the hook appends an
     * "⏹ Interrupted by user" markdown node so the user has durable
     * visual confirmation that the stop landed. Always cleared on
     * session_end regardless of whether the row was appended.
     */
    stoppingAtom?: SignalPair<boolean>;
    /**
     * Pending queue shared with the composer's `sendMessage` path. On
     * `agent-message-accepted` events, the hook removes the matching
     * entry and promotes it to a `user_message` document node — this is
     * the visible "accepted" transition for the user.
     */
    pendingMessagesAtom?: SignalPair<PendingMessage[]>;
    enabled: boolean;
    /**
     * Version signal bumped by external document mutations (e.g. history
     * load or prepend). When this changes, the hook rebuilds its internal
     * `nodeIdSet` and `nodeIndexMap` from the current documentAtom so live
     * updates continue to target the correct nodes.
     */
    documentVersion?: () => number;
}

/**
 * Subscribe to subprocess output and parse it into styled DocumentNodes.
 */
export function useAgentStream({
    blockId,
    outputFormat,
    documentAtom,
    streamingStateAtom,
    sessionStatsAtom,
    currentToolAtom,
    turnTokensAtom,
    turnActiveAtom,
    stoppingAtom,
    pendingMessagesAtom,
    enabled,
    documentVersion,
}: UseAgentStreamOpts): void {
    const [, setDocument] = documentAtom;
    const [, setStreaming] = streamingStateAtom;
    const [, setSessionStats] = sessionStatsAtom;
    const [, setTurnActive] = turnActiveAtom;
    const [, setCurrentTool] = currentToolAtom;
    const [, setTurnTokens] = turnTokensAtom;
    const getStopping = stoppingAtom?.[0];
    const setStopping = stoppingAtom?.[1];

    // Mutable state that doesn't trigger re-renders
    let lineBuffer = "";
    let translator = createTranslator(outputFormat);
    let parser = new ClaudeCodeStreamParser();
    let nodeIdSet = new Set<string>();

    // Batching: accumulate parsed nodes between RAF flushes
    let pendingNew: DocumentNode[] = [];
    let pendingUpdates: DocumentNode[] = [];
    let flushRafId: number | null = null;

    // Index for O(1) node lookups during updates
    let nodeIndexMap = new Map<string, number>();

    function flushPendingNodes() {
        flushRafId = null;
        if (pendingNew.length === 0 && pendingUpdates.length === 0) return;

        const batchNew = pendingNew;
        const batchUpdates = pendingUpdates;
        pendingNew = [];
        pendingUpdates = [];

        setDocument((prev) => {
            // Only copy the array once per flush, not per WebSocket message
            const result = prev.slice();
            let mutated = false;

            // Apply updates using index map for O(1) lookup
            for (const updated of batchUpdates) {
                const idx = nodeIndexMap.get(updated.id);
                if (idx != null && idx < result.length) {
                    const existing = result[idx];
                    if (existing.type === "markdown" && updated.type === "markdown") {
                        result[idx] = { ...existing, content: updated.content };
                    } else {
                        result[idx] = updated;
                    }
                    mutated = true;
                }
            }

            // Append new nodes — but guard against the race where a
            // documentVersion rebuild placed the node into the doc externally
            // (e.g. history load completed between the node entering pendingNew
            // and this RAF firing). If nodeIndexMap already has a valid entry
            // for the node, update in-place instead of appending to prevent
            // duplicates.
            if (batchNew.length > 0) {
                for (let i = 0; i < batchNew.length; i++) {
                    const n = batchNew[i];
                    const existingIdx = nodeIndexMap.get(n.id);
                    if (existingIdx != null && existingIdx < result.length) {
                        // Already in doc (placed by an external mutation while
                        // this node was queued). Update in-place.
                        result[existingIdx] = n;
                    } else {
                        nodeIndexMap.set(n.id, result.length);
                        result.push(n);
                    }
                }
                mutated = true;
            }

            // No cap on document size — `content-visibility: auto` on each
            // node wrapper lets the browser skip layout/paint for off-screen
            // nodes, so the DOM can grow to thousands without affecting
            // typing smoothness. Full history is preserved.
            // See docs/plans/agent-pane-ultra-long-sessions.md
            return mutated ? result : prev;
        });

        setStreaming((prev) => ({
            ...prev,
            lastEventTime: Date.now(),
            bufferSize: prev.bufferSize + batchNew.length,
        }));
    }

    function scheduleFlush() {
        if (flushRafId == null) {
            flushRafId = requestAnimationFrame(flushPendingNodes);
        }
    }

    onMount(() => {
        if (!enabled || !blockId) return;

        // Reset state on new subscription
        lineBuffer = "";
        translator = createTranslator(outputFormat);
        parser = new ClaudeCodeStreamParser();
        nodeIdSet = new Set();
        nodeIndexMap = new Map();
        pendingNew = [];
        pendingUpdates = [];

        const [doc] = documentAtom;

        /**
         * Shared finalization for "the turn is over." Called by both the
         * real `session_end` event from the CLI's result line AND the
         * fallback timer armed when the user presses Esc (killing the
         * subprocess prevents it from emitting its own result event).
         *
         * If `getStopping()` is true when this runs, the ending was user-
         * initiated — append a visible "⏹ Interrupted by user" row to
         * the document so the user has durable confirmation that the
         * stop landed.
         */
        const finalizeTurn = (stats: SessionStats | null) => {
            parser.flushPending();
            setSessionStats(stats);
            setCurrentTool(null);
            setTurnTokens(null);
            setTurnActive(false);
            if (getStopping?.()) {
                const interruptedNode: DocumentNode = {
                    type: "markdown",
                    id: `interrupted-${Date.now()}`,
                    content: "⏹ _Interrupted by user_",
                    timestamp: Date.now(),
                };
                if (!nodeIdSet.has(interruptedNode.id)) {
                    nodeIdSet.add(interruptedNode.id);
                    pendingNew.push(interruptedNode);
                    scheduleFlush();
                }
                setStopping?.(false);
            }
        };

        // Fallback timer: if the user presses Esc and the CLI doesn't
        // emit `session_end` within 1.5s (normal for a killed subprocess
        // — TerminateProcess skips any final output), run the same
        // finalization locally so the UI doesn't hang on "Stopping…".
        let stopFallbackTimer: number | null = null;
        if (getStopping && setStopping) {
            createEffect(() => {
                const stopping = getStopping();
                if (stopFallbackTimer != null) {
                    clearTimeout(stopFallbackTimer);
                    stopFallbackTimer = null;
                }
                if (stopping) {
                    stopFallbackTimer = window.setTimeout(() => {
                        stopFallbackTimer = null;
                        if (getStopping()) finalizeTurn(null);
                    }, 1500);
                }
            });
            onCleanup(() => {
                if (stopFallbackTimer != null) {
                    clearTimeout(stopFallbackTimer);
                    stopFallbackTimer = null;
                }
            });
        }

        // Subscribe to `agent-message-accepted`: when the backend picks
        // up a queued message, promote the matching entry out of the
        // pending zone into a real `user_message` document node. That
        // color shift (amber → accent blue) is the user's visible
        // "accepted" signal — the spec's core requirement.
        if (pendingMessagesAtom) {
            const [getPending, setPending] = pendingMessagesAtom;
            const acceptedUnsub = waveEventSubscribe({
                eventType: "agent-message-accepted",
                scope: WOS.makeORef("block", blockId),
                handler: (event) => {
                    const data = (event as any)?.data;
                    if (!data) return;
                    const messageId: string | undefined = data.message_id;
                    if (!messageId) return;
                    const pending = getPending().find((m) => m.id === messageId);
                    if (!pending) {
                        // Accepted event for an id we don't know about.
                        // Can legitimately happen if the entry was already
                        // promoted or the pane was re-mounted mid-queue.
                        return;
                    }
                    setPending((prev) => prev.filter((m) => m.id !== messageId));
                    // A new turn is now running (either the first one,
                    // which already flipped turnActive via handleSendMessage,
                    // or one drained from the queue — in that case
                    // turnActive went false on the *previous* session_end
                    // and nothing else flips it back, leaving the status
                    // line stuck on "Worked" with no running animation
                    // even though the CLI is processing the next message).
                    setTurnActive(true);
                    // Append as a normal user_message so it joins the
                    // conversation stream. Keeps the same id so the new
                    // node ties back to the pending entry 1:1.
                    const node: UserMessageNode = {
                        type: "user_message",
                        id: pending.id,
                        message: pending.text,
                        timestamp: Date.now(),
                        collapsed: false,
                        summary: "",
                    };
                    if (!nodeIdSet.has(node.id)) {
                        nodeIdSet.add(node.id);
                        pendingNew.push(node);
                        scheduleFlush();
                    }
                },
            });
            onCleanup(() => acceptedUnsub());
        }

        // Rebuild dedup set + index map from whatever is currently in the
        // document. `doc()` is wrapped in `untrack` so the createEffect
        // below only subscribes to documentVersion — NOT to the document
        // signal itself. Without this, the rebuild would fire on every
        // streaming flush (since flushPendingNodes calls setDoc), which
        // would be O(n) per live event.
        const rebuildIndicesFromDocument = () => {
            const existingNodes = untrack(() => doc());
            nodeIdSet = new Set();
            nodeIndexMap = new Map();
            for (let i = 0; i < existingNodes.length; i++) {
                nodeIdSet.add(existingNodes[i].id);
                nodeIndexMap.set(existingNodes[i].id, i);
            }
        };

        // Initial seed — covers history nodes that were loaded before this
        // hook mounted.
        rebuildIndicesFromDocument();

        // Reactive rebuild only when the caller bumps documentVersion after
        // an external prepend/load. The first invocation runs immediately
        // (SolidJS semantics) which is fine — it's idempotent.
        if (documentVersion != null) {
            createEffect(() => {
                documentVersion();
                // Re-add IDs from pending buffers AFTER rebuilding from the
                // document so subsequent stream events for in-flight nodes are
                // still routed as updates (not new nodes). Without this, a
                // rebuild clears their dedup protection and the next delta
                // creates a second entry for the same node.
                const beforePendingNew = pendingNew.slice();
                const beforePendingUpdates = pendingUpdates.slice();
                rebuildIndicesFromDocument();
                for (const n of beforePendingNew) nodeIdSet.add(n.id);
                for (const n of beforePendingUpdates) nodeIdSet.add(n.id);
            });
        }

        setStreaming((prev) => ({ ...prev, active: true, lastEventTime: Date.now() }));

        const fileSubject = getFileSubject(blockId, OutputFileName);

        console.debug(`[useAgentStream] subscribed blockId=${blockId} format=${outputFormat}`);
        const subscription = fileSubject.subscribe((msg: { fileop: string; data64: string }) => {
            if (msg.fileop === "truncate") {
                // Terminal was cleared — reset document
                if (flushRafId != null) { cancelAnimationFrame(flushRafId); flushRafId = null; }
                pendingNew = [];
                pendingUpdates = [];
                nodeIndexMap = new Map();
                setDocument([]);
                setSessionStats(null);
                setCurrentTool(null);
                setTurnTokens(null);
                setTurnActive(false);
                lineBuffer = "";
                translator.reset();
                parser.reset();
                nodeIdSet = new Set();
                return;
            }

            if (msg.fileop !== "append" || !msg.data64) return;

            // Decode base64 subprocess data to UTF-8 text
            const bytes = base64ToArray(msg.data64);
            const text = new TextDecoder().decode(bytes);

            // Accumulate into line buffer and process complete lines
            lineBuffer += text;
            const lines = lineBuffer.split("\n");
            lineBuffer = lines.pop() || ""; // Keep incomplete line
            // Safety: drop the buffer if it grows absurdly large without a
            // newline. This should never happen in well-formed stream-json
            // output, but protects against runaway memory if something
            // upstream is sending garbage.
            if (lineBuffer.length > 10_000_000) {
                console.warn(`[useAgentStream] line buffer exceeded 10MB, dropping`);
                lineBuffer = "";
            }

            for (const line of lines) {
                const trimmed = line.trim();
                if (!trimmed) continue;

                // Fast path: non-JSON lines (subprocess echoes, CLI warnings).
                // Skip without the cost of a try/catch on a huge string.
                if (!trimmed.startsWith("{")) continue;

                // Try to parse as JSON
                let rawEvent: any;
                try {
                    rawEvent = JSON.parse(trimmed);
                } catch {
                    // Don't log the full line — it may be 100KB+ (e.g. a Write
                    // tool call with a long file content) and forwarding it
                    // through the IPC log pipe stalls the main thread.
                    // See docs/analysis/v0-33-91-ndjson-parse-crash-2026-04-12.md
                    console.warn(`[useAgentStream] JSON parse failed, len=${trimmed.length}`);
                    continue;
                }

                // Handle stderr events from subprocess
                if (rawEvent.type === "stderr" && rawEvent.text) {
                    const text = rawEvent.text.trim();
                    if (text.includes("Fast mode is not available") ||
                        text.includes("[WARN]") && text.length < 200) {
                        continue;
                    }
                    pendingNew.push({
                        id: `stderr-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
                        type: "markdown",
                        content: `**stderr:** ${text}`,
                        timestamp: Date.now(),
                    });
                    scheduleFlush();
                    continue;
                }

                // Extract live token counts from Anthropic stream events before
                // the translator discards them. message_start carries input_tokens
                // for this turn; message_delta carries the running output_tokens.
                {
                    const inner = rawEvent.type === "stream_event" ? rawEvent.event : rawEvent;
                    if (inner?.type === "message_start") {
                        const inputTok = inner.message?.usage?.input_tokens as number | undefined;
                        if (inputTok != null) {
                            setTurnTokens((prev) => ({ input: inputTok, output: prev?.output ?? 0 }));
                        }
                    } else if (inner?.type === "message_delta") {
                        const outputTok = inner.usage?.output_tokens as number | undefined;
                        if (outputTok != null) {
                            setTurnTokens((prev) => ({ input: prev?.input ?? 0, output: outputTok }));
                        }
                    }
                }

                // Translate provider-specific format → StreamEvent[]
                const streamEvents = translator.translate(rawEvent);

                // Convert StreamEvents → DocumentNodes
                for (const event of streamEvents) {
                    // Handle session_end: store stats, clear loading state,
                    // and flush the parser's text/thinking accumulators so the
                    // NEXT turn creates fresh nodes instead of appending to the
                    // previous response (which sits above the user's message).
                    if (event.type === "session_end") {
                        finalizeTurn(event.stats ?? null);
                        continue;
                    }
                    // Track the currently-running tool for the status line
                    if (event.type === "tool_call") {
                        setCurrentTool(event.tool ?? null);
                    } else if (event.type === "tool_result") {
                        setCurrentTool(null);
                    }
                    const node = parser.parseLine(JSON.stringify(event));
                    if (!node) continue;

                    // Stamp a receive time on nodes that don't carry their own
                    // timestamp (markdown, tool, section, subagent_link).
                    // user_message and agent_message already have timestamps.
                    if (!("timestamp" in node) || (node as any).timestamp == null) {
                        (node as any).timestamp = Date.now();
                    }

                    if (nodeIdSet.has(node.id)) {
                        pendingUpdates.push(node);
                    } else {
                        nodeIdSet.add(node.id);
                        pendingNew.push(node);
                    }
                }
            }

            // Schedule a single flush per animation frame
            if (pendingNew.length > 0 || pendingUpdates.length > 0) {
                scheduleFlush();
            }
        });

        onCleanup(() => {
            if (flushRafId != null) { cancelAnimationFrame(flushRafId); flushRafId = null; }
            subscription.unsubscribe();
            setStreaming((prev) => ({ ...prev, active: false }));
            // Clear turn-active on teardown so a crash/exit without session_end
            // doesn't leave the status line showing "Working…" permanently.
            setTurnActive(false);
        });
    });
}
