// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pane's recent conversation in the form the ambient digest uses
 * (`crates/srv/src/ambient/digest.rs`): one entry per message, tool call or
 * error, oldest first. Built from the document, which the pane has already
 * translated from its provider's stream, so the server can suggest a next
 * message for a Codex or Gemini pane as well as a Claude one. A hidden memory
 * reinjection turn never reaches the document, so it never reaches this list.
 * (`btw.ts` renders the same document for `/btw`, in a fuller, labelled form
 * meant for a model answering a question rather than for this fixed contract.)
 * docs/reports/REPORT_AMBIENT_FRAMEWORK_REASSESSMENT_2026_10_08.md section 6.5.
 */

import type { DocumentNode } from "./types";

/** More than the server keeps (14 entries, 6,000 characters): it does the final cut. */
const MAX_ENTRIES = 40;
/** The server keeps 700 characters of an entry; the tail, where a question sits. */
const MAX_ENTRY_CHARS = 1_000;

function clipTail(entry: string): string {
    if (entry.length <= MAX_ENTRY_CHARS) return entry;
    const tagEnd = entry.indexOf("] ") + 2;
    return `${entry.slice(0, tagEnd)}…${entry.slice(entry.length - (MAX_ENTRY_CHARS - tagEnd - 1))}`;
}

function entriesFor(node: DocumentNode): string[] {
    switch (node.type) {
        case "user_message":
            return node.isStartup ? [] : [`[user] ${node.message.trim()}`];
        case "agent_message":
        case "jekt_message":
            return node.direction === "incoming" ? [`[user] ${node.message.trim()}`] : [];
        case "markdown":
            return node.metadata?.thinking ? [] : [`[assistant] ${node.content.trim()}`];
        case "tool": {
            const name = node.toolName ?? node.tool;
            return node.status === "failed" ? [`[tool] ${name}`, `[error] ${name} failed`] : [`[tool] ${name}`];
        }
        case "agent_error":
            return [`[error] ${node.message.trim()}`];
        default:
            return [];
    }
}

export function recentActivityEntries(nodes: readonly DocumentNode[]): string[] {
    const newestFirst: string[] = [];
    for (let i = nodes.length - 1; i >= 0 && newestFirst.length < MAX_ENTRIES; i--) {
        // An entry with nothing after its tag says nothing.
        const entries = entriesFor(nodes[i]).filter((e) => !e.endsWith("] "));
        newestFirst.push(...entries.reverse().map(clipTail));
    }
    return newestFirst.slice(0, MAX_ENTRIES).reverse();
}
