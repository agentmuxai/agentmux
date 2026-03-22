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

    onMount(() => {
        if (!enabled || !blockId) return;

        // Reset state on new subscription
        lineBuffer = "";
        translator = createTranslator(outputFormat);
        parser = new ClaudeCodeStreamParser();
        nodeIdSet = new Set();

        setStreaming((prev) => ({ ...prev, active: true, lastEventTime: Date.now() }));

        const fileSubject = getFileSubject(blockId, OutputFileName);

        const subscription = fileSubject.subscribe((msg: { fileop: string; data64: string }) => {
            if (msg.fileop === "truncate") {
                // Terminal was cleared — reset document
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

            // Process complete lines

            const newNodes: DocumentNode[] = [];
            const updatedNodes: DocumentNode[] = [];

            for (const line of lines) {
                const trimmed = line.trim();
                if (!trimmed) continue;

                // Try to parse as JSON
                let rawEvent: any;
                try {
                    rawEvent = JSON.parse(trimmed);
                } catch {
                    // Not valid JSON — skip (could be raw CLI output during init)
                    continue;
                }

                // Handle stderr events from subprocess
                if (rawEvent.type === "stderr" && rawEvent.text) {
                    // Skip benign CLI warnings
                    const text = rawEvent.text.trim();
                    if (text.includes("Fast mode is not available") ||
                        text.includes("[WARN]") && text.length < 200) {
                        console.log("[useAgentStream] suppressed stderr:", text);
                        continue;
                    }
                    const stderrNode: DocumentNode = {
                        id: `stderr-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
                        type: "markdown",
                        content: `**stderr:** ${text}`,
                    };
                    newNodes.push(stderrNode);
                    continue;
                }

                // Translate provider-specific format → StreamEvent[]
                const streamEvents = translator.translate(rawEvent);

                // Convert StreamEvents → DocumentNodes
                for (const event of streamEvents) {
                    const node = parser.parseLine(JSON.stringify(event));
                    if (!node) continue;

                    if (nodeIdSet.has(node.id)) {
                        // Existing node — append content for text/thinking, replace for others
                        updatedNodes.push(node);
                    } else {
                        nodeIdSet.add(node.id);
                        newNodes.push(node);
                    }
                }
            }

            // Batch update the document signal
            if (newNodes.length > 0 || updatedNodes.length > 0) {
                setDocument((prev) => {
                    let result = [...prev];

                    // Apply updates to existing nodes
                    for (const updated of updatedNodes) {
                        const idx = result.findIndex((n) => n.id === updated.id);
                        if (idx !== -1) {
                            const existing = result[idx];
                            if (existing.type === "markdown" && updated.type === "markdown") {
                                // Replace with accumulated content — the stream parser
                                // already accumulates deltas into the full text, so we
                                // must NOT append here or content gets duplicated.
                                result[idx] = {
                                    ...existing,
                                    content: updated.content,
                                };
                            } else {
                                // Replace for other node types (tool_result completing tool_call)
                                result[idx] = updated;
                            }
                        }
                    }

                    // Append new nodes
                    if (newNodes.length > 0) {
                        result = [...result, ...newNodes];
                    }

                    return result;
                });

                setStreaming((prev) => ({
                    ...prev,
                    lastEventTime: Date.now(),
                    bufferSize: prev.bufferSize + newNodes.length,
                }));
            }
        });

        onCleanup(() => {
            subscription.unsubscribe();
            setStreaming((prev) => ({ ...prev, active: false }));
        });
    });
}
