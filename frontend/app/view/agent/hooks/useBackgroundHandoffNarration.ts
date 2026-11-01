// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The lines the pane says when a Bash call moves to the background and when it finishes, in the words the
 * model gave the call (its `description`), so the user is told what is running and that the agent is free.
 * Fixed text, not a model call: it is there at once and names the same phrase the dock row shows. A call
 * with no description keeps the model-written narration (stream-flush-queue.ts).
 *
 * Only launches seen live are narrated: a launch older than `RECENT_MS` when first seen is history being
 * loaded, and a finish is narrated only for a launch this pane narrated.
 */

import { createEffect, on } from "solid-js";

import { backgroundCompletions, bashDescription, isAcceptedBackgroundLaunch } from "../activity/tool-adapter";
import type { AmbientNarrationNode, DocumentNode, ToolNode } from "../types";

/** A launch first seen later than this after it started is replayed history, not news. */
export const RECENT_MS = 60_000;

export interface HandoffState {
    /** Launch id to the phrase narrated for it and when the launch started. */
    started: Map<string, { phrase: string; at: number }>;
    finished: Set<string>;
}

export const newHandoffState = (): HandoffState => ({ started: new Map(), finished: new Set() });

const FINISH_WORDS = { done: "Finished", error: "Failed", stopped: "Stopped" } as const;

function line(id: string, text: string, now: number): AmbientNarrationNode {
    return { type: "ambient_narration", id, kind: "background_task", text, timestamp: now };
}

/** The nodes from the end back to the first one older than `floor`, in document order. Nodes arrive in time order, so
 *  this is the recent tail, not the whole document: this runs on every flush, streamed text included. */
function recentTail(nodes: ReadonlyArray<DocumentNode>, floor: number): DocumentNode[] {
    const tail: DocumentNode[] = [];
    for (let i = nodes.length - 1; i >= 0; i--) {
        const at = (nodes[i] as { timestamp?: number }).timestamp;
        if (at != null && at < floor) break;
        tail.push(nodes[i]);
    }
    return tail.reverse();
}

/** The narrations the document now calls for, given what was narrated before. Updates `state`. */
export function handoffNarrations(nodes: ReadonlyArray<DocumentNode>, state: HandoffState, now: number): AmbientNarrationNode[] {
    const out: AmbientNarrationNode[] = [];
    for (const n of recentTail(nodes, now - RECENT_MS)) {
        if (n.type !== "tool" || n.tool !== "Bash" || n.timestamp == null) continue;
        const tool = n as ToolNode;
        if (state.started.has(tool.id) || !isAcceptedBackgroundLaunch(tool)) continue;
        const phrase = bashDescription(tool);
        if (!phrase) continue;
        state.started.set(tool.id, { phrase, at: tool.timestamp });
        out.push(line(`handoff-start-${tool.id}`, `Moved to the background: ${phrase}. The agent is free; it picks this up when it finishes.`, now));
    }
    // A finish is looked for only back to the oldest launch still waiting for one.
    const waiting = [...state.started].filter(([id]) => !state.finished.has(id));
    if (waiting.length === 0) return out;
    const since = Math.min(...waiting.map(([, w]) => w.at));
    for (const [id, completion] of backgroundCompletions(recentTail(nodes, since))) {
        const started = state.started.get(id);
        if (started === undefined || state.finished.has(id)) continue;
        state.finished.add(id);
        const word = FINISH_WORDS[completion.status as keyof typeof FINISH_WORDS] ?? "Stopped";
        out.push(line(`handoff-end-${id}`, `${word} in the background: ${started.phrase}.`, now));
    }
    return out;
}

/** Narrates each hand-off to the background, and its end, into the pane's document as it happens. */
export function useBackgroundHandoffNarration(
    documentNodes: () => ReadonlyArray<DocumentNode>,
    onNarration: (nodes: AmbientNarrationNode[]) => void
): void {
    const state = newHandoffState();
    createEffect(
        on(documentNodes, (nodes) => {
            const lines = handoffNarrations(nodes, state, Date.now());
            if (lines.length > 0) onNarration(lines);
        })
    );
}
