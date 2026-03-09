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
 *
 * During bootstrap (before JSON streaming begins), non-JSON PTY output
 * (shell prompt, npm install, CLI startup) is captured as TerminalOutputNode
 * blocks so the user sees live feedback instead of a blank spinner.
 */

import { useEffect, useRef } from "react";
import { useSetAtom } from "jotai";
import type { PrimitiveAtom } from "jotai";
import { getFileSubject } from "@/app/store/wps";
import { base64ToArray } from "@/util/util";
import { createTranslator } from "./providers/translator-factory";
import { ClaudeCodeStreamParser } from "./stream-parser";
import type { DocumentNode, StreamEvent, StreamingState, TerminalOutputNode } from "./types";

const TermFileName = "term";

/**
 * Strip ANSI escape sequences and control characters from PTY text.
 *
 * Handles CSI sequences (colors, cursor), OSC sequences (window title,
 * directory tracking), character set designators, and stray control chars.
 * Preserves \t, \n, \r which are needed for line processing.
 */
function stripAnsi(text: string): string {
    return text
        .replace(/\x1b\][^\x07]*\x07/g, "") // OSC sequences (ESC ] ... BEL)
        .replace(/\x1b\][^\x1b]*\x1b\\/g, "") // OSC sequences (ESC ] ... ST)
        .replace(/\x1b\[[0-9;?]*[A-Za-z]/g, "") // CSI sequences (ESC [ ... letter)
        .replace(/\x1b[()][0-9A-Za-z]/g, "") // Character set designators
        .replace(/\x1b[#=<>78]/g, "") // Other single-char ESC sequences
        .replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, ""); // Control chars (keep \t \n \r)
}

/** Delay before flushing partial (no-newline) buffer content as terminal output */
const BUFFER_FLUSH_MS = 250;

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
 *
 * Non-JSON lines (bootstrap output) are captured as TerminalOutputNode blocks.
 * A debounced flush ensures partial lines (no trailing newline) appear promptly.
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
    // Terminal output accumulation for non-JSON lines (bootstrap output)
    const terminalNodeIdRef = useRef<string | null>(null);
    const terminalNodeCounterRef = useRef(0);
    // Whether we've seen at least one valid JSON line (marks end of bootstrap)
    const jsonStartedRef = useRef(false);
    // Timer for flushing partial (no-newline) buffer content
    const flushTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    useEffect(() => {
        if (!enabled || !blockId) return;

        // Reset state on new subscription
        lineBufferRef.current = "";
        translatorRef.current = createTranslator(outputFormat);
        parserRef.current = new ClaudeCodeStreamParser();
        nodeIdSetRef.current = new Set();
        terminalNodeIdRef.current = null;
        terminalNodeCounterRef.current = 0;
        jsonStartedRef.current = false;
        if (flushTimerRef.current) clearTimeout(flushTimerRef.current);
        flushTimerRef.current = null;

        setStreaming((prev) => ({ ...prev, active: true, lastEventTime: Date.now() }));

        /**
         * Flush the line buffer as terminal output even without a trailing newline.
         * Called on a timer when PTY data arrives without newlines (e.g., shell prompt).
         * Only flushes during bootstrap (before JSON streaming starts).
         */
        const flushBufferAsTerminal = () => {
            flushTimerRef.current = null;
            const buffered = lineBufferRef.current.trim();
            if (!buffered || jsonStartedRef.current) return;

            if (terminalNodeIdRef.current === null) {
                terminalNodeIdRef.current = `term_${terminalNodeCounterRef.current++}`;
            }

            const termId = terminalNodeIdRef.current;
            const isNew = !nodeIdSetRef.current.has(termId);
            const content = buffered + "\n";

            // Clear the buffer since we're flushing it
            lineBufferRef.current = "";

            if (isNew) {
                nodeIdSetRef.current.add(termId);
                setDocument((prev) => [
                    ...prev,
                    {
                        type: "terminal_output" as const,
                        id: termId,
                        content,
                        complete: false,
                    },
                ]);
            } else {
                setDocument((prev) => {
                    const idx = prev.findIndex((n) => n.id === termId);
                    if (idx === -1) return prev;
                    const existing = prev[idx] as TerminalOutputNode;
                    const result = [...prev];
                    result[idx] = { ...existing, content: existing.content + content };
                    return result;
                });
            }

            setStreaming((prev) => ({
                ...prev,
                lastEventTime: Date.now(),
                bufferSize: prev.bufferSize + (isNew ? 1 : 0),
            }));
        };

        const fileSubject = getFileSubject(blockId, TermFileName);

        const subscription = fileSubject.subscribe((msg: { fileop: string; data64: string }) => {
            if (msg.fileop === "truncate") {
                // Terminal was cleared — reset document
                setDocument([]);
                lineBufferRef.current = "";
                translatorRef.current.reset();
                parserRef.current.reset();
                nodeIdSetRef.current = new Set();
                terminalNodeIdRef.current = null;
                terminalNodeCounterRef.current = 0;
                jsonStartedRef.current = false;
                if (flushTimerRef.current) clearTimeout(flushTimerRef.current);
                flushTimerRef.current = null;
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
                    // Not valid JSON — capture as terminal output (bootstrap/init lines)
                    // If JSON has already started and we get a non-JSON line,
                    // start a new terminal node (separates bootstrap from mid-stream errors)
                    if (jsonStartedRef.current) {
                        terminalNodeIdRef.current = null;
                    }

                    if (terminalNodeIdRef.current === null) {
                        terminalNodeIdRef.current = `term_${terminalNodeCounterRef.current++}`;
                    }

                    const termId = terminalNodeIdRef.current;
                    const termNode: TerminalOutputNode = {
                        type: "terminal_output",
                        id: termId,
                        content: trimmed + "\n",
                        complete: false,
                    };

                    if (nodeIdSetRef.current.has(termId)) {
                        // Update existing terminal node — accumulate content
                        updatedNodes.push(termNode);
                    } else {
                        nodeIdSetRef.current.add(termId);
                        newNodes.push(termNode);
                    }
                    continue;
                }

                // First successful JSON parse — mark bootstrap terminal node as complete
                if (!jsonStartedRef.current) {
                    jsonStartedRef.current = true;
                    // Cancel any pending buffer flush
                    if (flushTimerRef.current) {
                        clearTimeout(flushTimerRef.current);
                        flushTimerRef.current = null;
                    }
                    if (terminalNodeIdRef.current !== null) {
                        updatedNodes.push({
                            type: "terminal_output",
                            id: terminalNodeIdRef.current,
                            content: "", // content preserved by updater
                            complete: true,
                        });
                        terminalNodeIdRef.current = null;
                    }
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
                    // Append new nodes FIRST so updates can find them in the same batch
                    let result = newNodes.length > 0 ? [...prev, ...newNodes] : [...prev];

                    // Apply updates to existing nodes (now includes newly appended ones)
                    for (const updated of updatedNodes) {
                        const idx = result.findIndex((n) => n.id === updated.id);
                        if (idx !== -1) {
                            if (updated.type === "terminal_output") {
                                const existing = result[idx] as TerminalOutputNode;
                                if (updated.complete) {
                                    // Mark as complete, preserve accumulated content
                                    result[idx] = { ...existing, complete: true };
                                } else {
                                    // Accumulate content into existing node
                                    result[idx] = {
                                        ...existing,
                                        content: existing.content + updated.content,
                                    };
                                }
                            } else {
                                result[idx] = updated;
                            }
                        }
                    }

                    return result;
                });

                setStreaming((prev) => ({
                    ...prev,
                    lastEventTime: Date.now(),
                    bufferSize: prev.bufferSize + newNodes.length,
                }));
            }

            // Schedule buffer flush for partial lines during bootstrap.
            // If PTY data arrives without newlines (e.g., shell prompt), the content
            // sits in lineBufferRef. This timer ensures it surfaces as terminal output
            // after a short delay rather than staying invisible.
            if (!jsonStartedRef.current && lineBufferRef.current.trim()) {
                if (flushTimerRef.current) clearTimeout(flushTimerRef.current);
                flushTimerRef.current = setTimeout(flushBufferAsTerminal, BUFFER_FLUSH_MS);
            }
        });

        return () => {
            subscription.unsubscribe();
            if (flushTimerRef.current) clearTimeout(flushTimerRef.current);
            setStreaming((prev) => ({ ...prev, active: false }));
        };
    }, [blockId, outputFormat, enabled, setDocument, setStreaming]);
}
