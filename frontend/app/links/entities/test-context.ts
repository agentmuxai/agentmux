// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A roster, a board and a context shared by the recogniser's tests. */

import { buildAgentIndex, buildTaskIndex } from "./indexes";
import type { AgentEntry, LinkContext, Segment, TaskEntry } from "./types";

export const ROSTER: AgentEntry[] = [
    { id: "opaz-0909m", slug: "opaz", name: "Opaz" },
    { id: "camper-0622h", slug: "camper", name: "Camper" },
    { id: "posa-1", slug: "posa", name: "Posa" },
    { id: "agenty", slug: "agenty", name: "AgentY" },
    { id: "naki-2-09169", slug: "naki-2", name: "Naki-2" },
    { id: "will-7", slug: "will-7", name: "Will" },
    { id: "pulp", slug: "pulp", name: "Pulp" },
    { id: "clare-3", slug: "clare", name: "Clare", computer: "starpower" },
];

const task = (board: string, number: number, title = "t"): TaskEntry => ({ board, number, title, state: "open" });

export const TASKS: TaskEntry[] = [task("main", 48247), task("main", 47448), task("main", 48175), task("main", 58373), task("main", 7940), task("work", 12)];

export function testContext(over: Partial<LinkContext> = {}): LinkContext {
    return {
        author: { kind: "agent", id: "opaz-0909m", confined: false },
        agents: buildAgentIndex(ROSTER, { localComputer: "narko", version: 1 }),
        tasks: buildTaskIndex(TASKS, "main", 1),
        repo: "agentmuxai/agentmux",
        final: true,
        ...over,
    };
}

/** The links of `segments`, in order, as `kind:label`. */
export function linksOf(segments: Segment[]): string[] {
    return segments.flatMap((s) => (s.type === "link" ? [`${s.kind}:${s.label}`] : []));
}

export function joined(segments: Segment[]): string {
    return segments.map((s) => (s.type === "text" ? s.text : s.label)).join("");
}
