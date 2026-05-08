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
import { createEffect, onCleanup, onMount } from "solid-js";
import { createTranslator } from "./providers/translator-factory";
import type { PendingMessage, SignalPair } from "./state";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { DocumentNode, SessionStats, StreamingState, TurnTokens, UserMessageNode } from "./types";
import { recordTurn } from "@/store/token-usage";
import {
    dispatch as dispatchDoc,
    getNodeIdSet,
} from "@/app/store/agent-document-store";
import { dispatch as dispatchPane } from "@/app/store/agent-pane-state-store";

const OutputFileName = "output";

/**
 * Watchdog tick rate. Every 5s the hook dispatches a StreamWatchdogTick
 * to the pane-state reducer; the reducer compares against
 * `STUCK_THRESHOLD_MS` (45s) and emits a `stream-stuck` event when the
 * subscribed stream has been silent that long. Issue #728 gap 3.
 */
const WATCHDOG_INTERVAL_MS = 5_000;

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
     * Provider id (from CLI_CATALOG — "claude", "codex", "gemini", …).
     * Used to attribute completed-turn tokens to the right row in the
     * status-bar token-usage store. Optional for back-compat; missing
     * provider means tokens aren't aggregated (the per-pane stats still
     * work). Per SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §5.1.
     */
    provider?: string;
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
    provider,
}: UseAgentStreamOpts): void {
    // Read-side accessors only — all writes route through dispatchPane.
    const [getTurnTokens] = turnTokensAtom;
    const getStopping = stoppingAtom?.[0];

    // Mutable state that doesn't trigger re-renders
    let lineBuffer = "";
    let translator = createTranslator(outputFormat);
    let parser = new ClaudeCodeStreamParser();
    // nodeIdSet remains for fast in-batch dedup (the reducer also dedups,
    // but checking here avoids enqueuing already-seen nodes into pendingNew).
    let nodeIdSet = new Set<string>();

    // Batching: accumulate parsed nodes between RAF flushes
    let pendingNew: DocumentNode[] = [];
    let pendingUpdates: DocumentNode[] = [];
    let flushRafId: number | null = null;

    function flushPendingNodes() {
        flushRafId = null;
        if (pendingNew.length === 0 && pendingUpdates.length === 0) return;

        const batchNew = pendingNew;
        const batchUpdates = pendingUpdates;
        pendingNew = [];
        pendingUpdates = [];

        // Document mutation — the reducer owns dedup, in-place updates,
        // and the markdown-content merge. See agent-document-store.ts.
        dispatchDoc(blockId, {
            type: "StreamFlush",
            newNodes: batchNew,
            updatedNodes: batchUpdates,
        });
        // Lifecycle counter bump — agent-pane-state owns streaming
        // metadata (active flag + bufferSize + lastEventTime).
        dispatchPane(blockId, {
            type: "StreamFlushObserved",
            addedCount: batchNew.length,
            at: Date.now(),
        });
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
        pendingNew = [];
        pendingUpdates = [];

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
            // Snapshot live turn tokens into SessionStats before nulling
            // the live signal. The Worked footer reads from sessionStats
            // (since turnTokens is cleared on session_end); without this
            // merge the headline tokens-in-Worked feature shows nothing.
            // Per PR #549 reagent/codex P1.
            const tokens = getTurnTokens();
            // Aggregate the completed turn's tokens into the global
            // session-local token-usage store so the status bar's
            // indicator + breakdown popover stay up to date. Guarded
            // against double-counting by recordTurn's own no-op-on-zero
            // check — see SPEC_STATUSBAR_TOKEN_USAGE_2026_04_24.md §5.1.
            if (provider && tokens) {
                recordTurn(provider, tokens);
            }
            // The reducer's TurnEnd handler does the cross-atom cleanup
            // in one shot: merges live tokens into stats, clears tool/
            // tokens/turnActive, AND clears stopping (the latter cascade
            // replaces the explicit setStopping(false) below).
            const wasStopping = getStopping?.() === true;
            dispatchPane(blockId, { type: "TurnEnd", stats });
            if (wasStopping) {
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
            }
        };

        // Fallback timer: if the user presses Esc and the CLI doesn't
        // emit `session_end` within 1.5s (normal for a killed subprocess
        // — TerminateProcess skips any final output), run the same
        // finalization locally so the UI doesn't hang on "Stopping…".
        let stopFallbackTimer: number | null = null;
        if (getStopping) {
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
            const [getPending] = pendingMessagesAtom;
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
                    // Reducer removes the entry + emits pending-accepted.
                    dispatchPane(blockId, {
                        type: "PendingMessageAccepted",
                        id: messageId,
                    });
                    // A new turn is now running (either the first one,
                    // which already flipped turnActive via handleSendMessage,
                    // or one drained from the queue — in that case
                    // turnActive went false on the *previous* session_end
                    // and nothing else flips it back, leaving the status
                    // line stuck on "Worked" with no running animation
                    // even though the CLI is processing the next message).
                    dispatchPane(blockId, { type: "TurnStart", at: Date.now() });
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

        // Seed the in-batch dedup cache from the reducer-maintained
        // index. Issue #728 gap 4 — replaces the per-mount scan of
        // `doc()` that could miss in-flight events arriving between
        // mount and scan. The reducer keeps `nodeIdSet` in lockstep
        // with `nodes[]` so this read is always current.
        nodeIdSet = new Set(getNodeIdSet(blockId));

        // Two reducers signaled in lockstep: pane-state owns the streaming
        // metadata (active flag), agent-document owns the session phase
        // gate that drives truncate suppression.
        const subscribedAt = Date.now();
        dispatchPane(blockId, { type: "StreamSubscribe", at: subscribedAt });
        dispatchDoc(blockId, { type: "SessionStart", at: subscribedAt });

        // Stuck-stream watchdog (issue #728 gap 3). The reducer evaluates
        // each tick against `lastEventMs` and emits a `stream-stuck`
        // event when the gap exceeds `STUCK_THRESHOLD_MS`. The interval
        // cleans up via the same effect cleanup as the subscription.
        const watchdogId = setInterval(() => {
            dispatchPane(blockId, {
                type: "StreamWatchdogTick",
                nowMs: Date.now(),
            });
        }, WATCHDOG_INTERVAL_MS);
        onCleanup(() => clearInterval(watchdogId));

        const fileSubject = getFileSubject(blockId, OutputFileName);

        console.debug(`[useAgentStream] subscribed blockId=${blockId} format=${outputFormat}`);
        const subscription = fileSubject.subscribe((msg: { fileop: string; data64: string }) => {
            if (msg.fileop === "truncate") {
                // Reducer decides whether to honor — late truncates after
                // a socket-reconnect race are suppressed. Only reset the
                // hook-local parser/stats/etc. when the truncate is actually
                // honored; if suppressed, the live stream is still flowing
                // and resetting would corrupt in-flight parse state.
                const events = dispatchDoc(blockId, {
                    type: "StreamTruncate",
                    reason: "fileop",
                });
                const honored = events.some((e) => e.type === "truncate-applied");
                if (!honored) return;
                if (flushRafId != null) { cancelAnimationFrame(flushRafId); flushRafId = null; }
                pendingNew = [];
                pendingUpdates = [];
                lineBuffer = "";
                translator.reset();
                parser.reset();
                nodeIdSet = new Set();
                // Reducer clears sessionStats/currentTool/turnTokens/
                // turnActive/stopping in one shot.
                dispatchPane(blockId, { type: "TurnReset" });
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
                            dispatchPane(blockId, { type: "TokensIn", input: inputTok });
                        }
                    } else if (inner?.type === "message_delta") {
                        const outputTok = inner.usage?.output_tokens as number | undefined;
                        if (outputTok != null) {
                            dispatchPane(blockId, { type: "TokensOut", output: outputTok });
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
                        // Preserve prior semantic: missing tool name means
                        // "no tool" (currentTool=null), NOT "tool with empty
                        // name". Pre-reducer code did setCurrentTool(event.tool ?? null);
                        // route through ToolEnd in the missing-name case.
                        if (event.tool) {
                            dispatchPane(blockId, { type: "ToolStart", name: event.tool });
                        } else {
                            dispatchPane(blockId, { type: "ToolEnd" });
                        }
                    } else if (event.type === "tool_result") {
                        dispatchPane(blockId, { type: "ToolEnd" });
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
            // StreamUnsubscribe also force-clears turnActive (so a crash
            // or exit without session_end doesn't leave "Working…" stuck).
            const at = Date.now();
            dispatchPane(blockId, { type: "StreamUnsubscribe", at });
            dispatchDoc(blockId, { type: "SessionEnd", at });
        });
    });
}
