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

/** The one agent in `list`; none when two agents answer to the same written name, since a wrong link is worse than none. */
function onlyOne(list: AgentEntry[] | undefined): AgentEntry | undefined {
    return list?.length === 1 ? list[0] : undefined;
}

function add(map: Map<string, AgentEntry[]>, key: string, agent: AgentEntry): void {
    const list = map.get(key);
    if (!list) map.set(key, [agent]);
    else if (!list.some((other) => other.id === agent.id)) list.push(agent);
}

/** A display name with a number, as in `Loap #3`: the name and the number apart. */
const NUMBERED = /^(.*\S) #\d+$/;

export function buildAgentIndex(agents: AgentEntry[], options: AgentIndexOptions = {}): AgentIndex {
    const byName = new Map<string, AgentEntry[]>();
    const byKey = new Map<string, AgentEntry[]>();
    const firsts = new Set<string>();
    const spaced = new Map<string, string[]>();
    const families = new Set<string>();
    for (const agent of agents) {
        const name = agent.name.trim();
        if (name) {
            add(byName, name, agent);
            firsts.add(name[0]);
            const first = /^[A-Za-z][A-Za-z0-9_-]*/.exec(name)?.[0];
            if (first && name.length > first.length && !spaced.get(first)?.includes(name)) spaced.set(first, [...(spaced.get(first) ?? []), name]);
            const base = NUMBERED.exec(name)?.[1];
            if (base) families.add(base);
        }
        for (const key of [agent.slug, agent.id]) {
            if (!key) continue;
            add(byKey, key.toLowerCase(), agent);
            firsts.add(key[0]);
            firsts.add(key[0].toUpperCase());
        }
    }
    // The longest name that starts with a word is tried first.
    for (const names of spaced.values()) names.sort((x, y) => y.length - x.length);
    const never = new Set((options.neverAlone ?? NEVER_ALONE_WORDS).map((w) => w.toLowerCase()));
    return {
        version: options.version ?? 0,
        localComputer: options.localComputer,
        find: (written, withComputer = false) => {
            const named = byName.get(written);
            if (named) return onlyOne(named);
            const keyed = onlyOne(byKey.get(written.toLowerCase()));
            return keyed && (withComputer || aliasStandsAlone(keyed, written)) ? keyed : undefined;
        },
        longestName: (text, start) => {
            const first = /^[A-Za-z][A-Za-z0-9_-]*/.exec(text.slice(start, start + 64))?.[0];
            const names = first ? spaced.get(first) : undefined;
            // The name must end where a word ends: `Loap #3` is not the start of `Loap #35` or `Loap #3x`.
            return names?.find((name) => text.startsWith(name, start) && !/^(?:\w|-\w)/.test(text.slice(start + name.length, start + name.length + 2)));
        },
        isNumberedFamily: (base) => families.has(base),
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
