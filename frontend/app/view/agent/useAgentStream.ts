// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentStream — SolidJS hook that subscribes to a block's subprocess output,
 * pipes it through the provider translator + stream parser, and feeds
 * the resulting DocumentNodes into SolidJS signals.
 */

import { getFileSubject } from "@/app/store/wps";
import { base64ToArray } from "@/util/util";
import { onCleanup, onMount } from "solid-js";
import { createTranslator } from "./providers/translator-factory";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { SignalPair } from "./state";
import type { DocumentNode, StreamingState } from "./types";

const OutputFileName = "output";

interface UseAgentStreamOpts {
    blockId: string;
    outputFormat: string;
    documentAtom: SignalPair<DocumentNode[]>;
    streamingStateAtom: SignalPair<StreamingState>;
    enabled: boolean;
}

/**
 * Subscribe to subprocess output and parse it into styled DocumentNodes.
 */
export function useAgentStream({
    blockId,
    outputFormat,
    documentAtom,
    streamingStateAtom,
    enabled,
}: UseAgentStreamOpts): void {
    const [, setDocument] = documentAtom;
    const [, setStreaming] = streamingStateAtom;

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

            // Append new nodes
            if (batchNew.length > 0) {
                const baseIdx = result.length;
                for (let i = 0; i < batchNew.length; i++) {
                    nodeIndexMap.set(batchNew[i].id, baseIdx + i);
                    result.push(batchNew[i]);
                }
                mutated = true;
            }

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

            for (const line of lines) {
                const trimmed = line.trim();
                if (!trimmed) continue;

                // Try to parse as JSON
                let rawEvent: any;
                try {
                    rawEvent = JSON.parse(trimmed);
                } catch {
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
                    });
                    scheduleFlush();
                    continue;
                }

                // Translate provider-specific format → StreamEvent[]
                const streamEvents = translator.translate(rawEvent);

                // Convert StreamEvents → DocumentNodes
                for (const event of streamEvents) {
                    const node = parser.parseLine(JSON.stringify(event));
                    if (!node) continue;

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
        });
    });
}
