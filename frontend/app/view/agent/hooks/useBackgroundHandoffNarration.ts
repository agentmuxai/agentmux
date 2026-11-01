// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The lines the pane says when a Bash call moves to the background and when it finishes, in the words the
 * model gave the call (its `description`), so the user is told what is running and that the agent is free.
 * Fixed text, not a model call: it is there at once and names the same phrase the dock row shows. A call
 * with no description keeps the model-written narration (stream-flush-queue.ts).
 *
 * Only launches seen live are narrated: one that started before this pane began watching, or more than
 * `RECENT_MS` before it was first seen, is history being loaded or restored, and a finish is narrated only
 * for a launch this pane narrated.
 */

import { createEffect, on } from "solid-js";

import { backgroundCompletions, bashDescription, isAcceptedBackgroundLaunch } from "../activity/tool-adapter";
import type { AmbientNarrationNode, DocumentNode, ToolNode } from "../types";

/** A launch first seen later than this after it started is replayed history, not news. */
export const RECENT_MS = 60_000;

export interface HandoffState {
    /** When this pane began watching: a launch that started before it is restored history, not news. */
    since: number;
    /** Launch id to the phrase narrated for it and when the launch started. */
    started: Map<string, { phrase: string; at: number }>;
    finished: Set<string>;
}

export const newHandoffState = (since = 0): HandoffState => ({ since, started: new Map(), finished: new Set() });

const FINISH_WORDS = { done: "Finished", error: "Failed", stopped: "Stopped" } as const;

function line(id: string, text: string, now: number): AmbientNarrationNode {
    return { type: "ambient_narration", id, kind: "background_task", text, timestamp: now };
}

/** How many of the newest nodes are looked at for a new launch. A launch is narrated as it lands, so it is among
 *  the last few; the count is by arrival order, not by timestamp, which is not sorted (a held message keeps the
 *  time it was sent). */
export const LAUNCH_WINDOW = 300;
/** The furthest back a finish is looked for. */
const FINISH_REACH = 5000;

/** The nodes from the end back to the oldest of `waiting` (by arrival order), in document order. */
function tailFrom(nodes: ReadonlyArray<DocumentNode>, waiting: ReadonlySet<string>): DocumentNode[] {
    const left = new Set(waiting);
    const tail: DocumentNode[] = [];
    for (let i = nodes.length - 1; i >= 0 && left.size > 0 && tail.length < FINISH_REACH; i--) {
        tail.push(nodes[i]);
        left.delete(nodes[i].id);
    }
    return tail.reverse();
}

/** The narrations the document now calls for, given what was narrated before. Updates `state`. */
export function handoffNarrations(nodes: ReadonlyArray<DocumentNode>, state: HandoffState, now: number): AmbientNarrationNode[] {
    const out: AmbientNarrationNode[] = [];
    for (const n of nodes.slice(-LAUNCH_WINDOW)) {
        if (n.type !== "tool" || n.tool !== "Bash" || n.timestamp == null) continue;
        const tool = n as ToolNode;
        if (state.started.has(tool.id) || !isAcceptedBackgroundLaunch(tool) || tool.timestamp < state.since || now - tool.timestamp > RECENT_MS) continue;
        const phrase = bashDescription(tool);
        if (!phrase) continue;
        state.started.set(tool.id, { phrase, at: tool.timestamp });
        out.push(line(`handoff-start-${tool.id}`, `Moved to the background: ${phrase}. The agent is free; it picks this up when it finishes.`, now));
    }
    // A finish is looked for only back to the oldest launch still waiting for one.
    const waiting = new Set([...state.started.keys()].filter((id) => !state.finished.has(id)));
    if (waiting.size === 0) return out;
    for (const [id, completion] of backgroundCompletions(tailFrom(nodes, waiting))) {
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
    const state = newHandoffState(Date.now());
    createEffect(
        on(documentNodes, (nodes) => {
            const lines = handoffNarrations(nodes, state, Date.now());
            if (lines.length > 0) onNarration(lines);
        })
    );
}
