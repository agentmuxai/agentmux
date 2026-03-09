// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentStream — React hook that subscribes to a block's PTY output,
 * pipes it through the provider translator + stream parser, and feeds
 * the resulting DocumentNodes into Jotai atoms.
 *
 * Data flow:
 *   PTY FileSubject → base64 decode → UTF-8 text → NDJSON lines
 *     → OutputTranslator.translate() → StreamEvent[]
 *     → ClaudeCodeStreamParser.parseLine() → DocumentNode
 *     → Jotai documentAtom (append or update)
 */

import { useEffect, useRef } from "react";
import { useSetAtom } from "jotai";
import type { PrimitiveAtom } from "jotai";
import { getFileSubject } from "@/app/store/wps";
import { base64ToArray } from "@/util/util";
import { createTranslator } from "./providers/translator-factory";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { DocumentNode, StreamEvent, StreamingState } from "./types";

const TermFileName = "term";

/**
 * Strip ANSI escape sequences from text.
 *
 * PTY data may contain cursor control, color codes, and other terminal
 * escape sequences from CLI startup before JSON streaming begins.
 * These corrupt JSON parsing if embedded in NDJSON lines.
 */
function stripAnsi(text: string): string {
    // Matches CSI sequences, OSC sequences, and other common escape codes
    return text.replace(
        // eslint-disable-next-line no-control-regex
        /[\x1b\x9b][[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><~]/g,
        ""
    );
}

interface UseAgentStreamOpts {
    blockId: string;
    outputFormat: string;
    documentAtom: PrimitiveAtom<DocumentNode[]>;
    streamingStateAtom: PrimitiveAtom<StreamingState>;
    enabled: boolean;
}

/**
 * Subscribe to PTY output and parse it into styled DocumentNodes.
 *
 * The hook manages its own line buffer to handle NDJSON lines split across
 * multiple PTY data events. Each complete line is:
 *   1. Parsed as JSON
 *   2. Fed through the provider translator (e.g. ClaudeTranslator)
 *   3. Fed through ClaudeCodeStreamParser to produce DocumentNodes
 *   4. Written to the Jotai documentAtom
 */
export function useAgentStream({
    blockId,
    outputFormat,
    documentAtom,
    streamingStateAtom,
    enabled,
}: UseAgentStreamOpts): void {
    const setDocument = useSetAtom(documentAtom);
    const setStreaming = useSetAtom(streamingStateAtom);

    // Refs for mutable state that persists across renders but doesn't trigger re-renders
    const lineBufferRef = useRef("");
    const translatorRef = useRef(createTranslator(outputFormat));
    const parserRef = useRef(new ClaudeCodeStreamParser());
    // Track node IDs we've seen so we can update (tool_result) vs append
    const nodeIdSetRef = useRef(new Set<string>());

    useEffect(() => {
        if (!enabled || !blockId) return;

        // Reset state on new subscription
        lineBufferRef.current = "";
        translatorRef.current = createTranslator(outputFormat);
        parserRef.current = new ClaudeCodeStreamParser();
        nodeIdSetRef.current = new Set();

        setStreaming((prev) => ({ ...prev, active: true, lastEventTime: Date.now() }));

        const fileSubject = getFileSubject(blockId, TermFileName);

        const subscription = fileSubject.subscribe((msg: { fileop: string; data64: string }) => {
            if (msg.fileop === "truncate") {
                // Terminal was cleared — reset document
                setDocument([]);
                lineBufferRef.current = "";
                translatorRef.current.reset();
                parserRef.current.reset();
                nodeIdSetRef.current = new Set();
                return;
            }

            if (msg.fileop !== "append" || !msg.data64) return;

            // Decode base64 PTY data to UTF-8 text, stripping ANSI escapes
            const bytes = base64ToArray(msg.data64);
            const rawText = new TextDecoder().decode(bytes);
            const text = stripAnsi(rawText);

            // Accumulate into line buffer and process complete lines
            lineBufferRef.current += text;
            const lines = lineBufferRef.current.split("\n");
            lineBufferRef.current = lines.pop() || ""; // Keep incomplete line

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

                // Translate provider-specific format → StreamEvent[]
                const streamEvents = translatorRef.current.translate(rawEvent);

                // Convert StreamEvents → DocumentNodes
                for (const event of streamEvents) {
                    const node = parserRef.current.parseStreamEvent(event as StreamEvent);
                    if (!node) continue;

                    if (nodeIdSetRef.current.has(node.id)) {
                        // Update existing node (e.g. tool_result completing a tool_call)
                        updatedNodes.push(node);
                    } else {
                        nodeIdSetRef.current.add(node.id);
                        newNodes.push(node);
                    }
                }
            }

            // Batch update the document atom
            if (newNodes.length > 0 || updatedNodes.length > 0) {
                setDocument((prev) => {
                    let result = [...prev];

                    // Apply updates to existing nodes
                    for (const updated of updatedNodes) {
                        const idx = result.findIndex((n) => n.id === updated.id);
                        if (idx !== -1) {
                            result[idx] = updated;
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

        return () => {
            subscription.unsubscribe();
            setStreaming((prev) => ({ ...prev, active: false }));
        };
    }, [blockId, outputFormat, enabled, setDocument, setStreaming]);
}
