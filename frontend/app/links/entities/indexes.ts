// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The two lookups the recogniser reads (spec §4.3). Each is built once when its source changes and carries a version, so
 * a drawn text is linkified again only when its indexes moved, and a lookup is a map hit, never a scan of the roster.
 */

import type { AgentEntry, AgentIndex, TaskEntry, TaskIndex } from "./types";

/** Names that are also ordinary words. Written alone, in any case, they are not linked (D3): only `Name@computer`, or an id that is more than the word. */
const NEVER_ALONE_WORDS = ["will", "mark", "grace", "hope", "rose", "sage", "patch", "ace", "art", "wren", "may", "bill"];

/**
 * Whether an agent's slug or id, written `written`, is read as the agent when it stands alone in text. It is when it is
 * the display name's own word in another case (`opaz` for Opaz), or carries a digit (`naki-2`, `opaz-0909m`): more than
 * a word. An id that is another plain word (the slug `pulse` of the agent Pulp) is never the agent on its own: the
 * product "Pulse" is not a mention. It links only with its computer (`pulse@computer`).
 */
function aliasStandsAlone(agent: AgentEntry, written: string): boolean {
    return written.toLowerCase() === agent.name.trim().toLowerCase() || /\d/.test(written);
}

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
        find: (written, withComputer = false) => {
            const named = byName.get(written);
            if (named) return named;
            const keyed = byKey.get(written.toLowerCase());
            return keyed && (withComputer || aliasStandsAlone(keyed, written)) ? keyed : undefined;
        },
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
