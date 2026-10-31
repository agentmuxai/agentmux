// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A container agent's start-up rows (`cli-notice.ts`): "Container ready (12 s)" when its container
 * came up, and "Ready: send a message to start" when nothing is on its way to it. A pane that is
 * reopened, or whose AgentMux restarts, before its first message starts the container again and
 * says so again, so the transcript piled up a pair per open. A start-up row says where the agent is
 * now, so only the newest of each kind stays between two things the agent or the user said.
 */
import type { CliNoticeNode, DocumentNode } from "./types";

function isReadyRow(n: DocumentNode): n is CliNoticeNode {
    return n.type === "cli_notice" && (n.kind === "agent_ready" || (n.kind === "container_start" && n.startState === "ready"));
}

/**
 * `nodes` without the start-up rows a later one of the same kind replaces. Anything that is not a
 * notice (a message, a tool call, a card) ends the stretch, so a restart in the middle of a
 * conversation keeps its own row. Returns `nodes` itself when nothing goes.
 */
export function dropSupersededStartRows(nodes: readonly DocumentNode[]): readonly DocumentNode[] {
    let superseded: Set<string> | null = null;
    const newest = new Map<string, string>();
    for (const n of nodes) {
        if (n.type !== "cli_notice") {
            newest.clear();
            continue;
        }
        if (!isReadyRow(n)) continue;
        const earlier = newest.get(n.kind);
        if (earlier) (superseded ??= new Set()).add(earlier);
        newest.set(n.kind, n.id);
    }
    return superseded ? nodes.filter((n) => !superseded.has(n.id)) : nodes;
}
