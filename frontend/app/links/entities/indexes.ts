// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The two lookups the recogniser reads (spec §4.3). Each is built once when its source changes and carries a version, so
 * a drawn text is linkified again only when its indexes moved, and a lookup is a map hit, never a scan of the roster.
 */

import type { AgentEntry, AgentIndex, TaskEntry, TaskIndex } from "./types";

/** Names that are also ordinary words. Written alone, in any case, they are not linked (D3): only `Name@computer`, or an id that is more than the word. */
const NEVER_ALONE_WORDS = ["will", "mark", "grace", "hope", "rose", "sage", "patch", "pulp", "ace", "art", "wren", "may", "bill"];

export type AgentIndexOptions = { localComputer?: string; neverAlone?: string[]; version?: number };

export function buildAgentIndex(agents: AgentEntry[], options: AgentIndexOptions = {}): AgentIndex {
    const byName = new Map<string, AgentEntry>();
    const byKey = new Map<string, AgentEntry>();
    const firsts = new Set<string>();
    for (const agent of agents) {
        const name = agent.name.trim();
        if (name) {
            byName.set(name, agent);
            firsts.add(name[0]);
        }
        for (const key of [agent.slug, agent.id]) {
            if (!key) continue;
            byKey.set(key.toLowerCase(), agent);
            firsts.add(key[0]);
            firsts.add(key[0].toUpperCase());
        }
    }
    const never = new Set((options.neverAlone ?? NEVER_ALONE_WORDS).map((w) => w.toLowerCase()));
    return {
        version: options.version ?? 0,
        localComputer: options.localComputer,
        find: (written) => byName.get(written) ?? byKey.get(written.toLowerCase()),
        neverAlone: (written) => never.has(written.toLowerCase()),
        startsWith: (char) => firsts.has(char),
    };
}

export function buildTaskIndex(tasks: TaskEntry[], defaultBoard: string, version = 0): TaskIndex {
    const byId = new Map<string, TaskEntry>();
    const boards = new Set<string>([defaultBoard]);
    for (const task of tasks) {
        byId.set(`${task.board}#${task.number}`, task);
        boards.add(task.board);
    }
    return {
        version,
        defaultBoard,
        hasBoard: (slug) => boards.has(slug),
        get: (board, number) => byId.get(`${board}#${number}`),
    };
}
