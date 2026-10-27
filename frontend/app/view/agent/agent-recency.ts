// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When an agent was last used, and the order the lists you pick an agent from
 * put it in: most recently used first. One source for every such list (My
 * Agents, the Seat lead picker, the job owner, the Skill and MCP bind pickers,
 * the Hidden agents picker, the account bind menu, the Hangar's Agents places),
 * so they agree.
 *
 * The time is the first of these an item has, all epoch ms:
 *  - `last_active_at`: the last output of its conversation (a session row);
 *  - `started_at`: its last launch (a session row, a named-agent row);
 *  - `updated_at`: the last launch, edit or rename of its definition
 *    (`db_agents.updated_at`, which the srv also orders `listagents` by).
 * Never used sorts last; ties keep name order.
 */

export interface LastUsedFields {
    last_active_at?: number | null;
    started_at?: number | null;
    updated_at?: number | null;
}

export function lastUsedMs(a: LastUsedFields): number {
    return a.last_active_at || a.started_at || a.updated_at || 0;
}

/** Newest use first. */
export function compareLastUsed(a: LastUsedFields, b: LastUsedFields): number {
    return lastUsedMs(b) - lastUsedMs(a);
}

/** A copy of `items`, most recently used first, name order within a tie. */
export function sortByLastUsed<T extends LastUsedFields>(items: readonly T[], nameOf: (item: T) => string): T[] {
    return items
        .slice()
        .sort(
            (a, b) =>
                compareLastUsed(a, b) || nameOf(a).localeCompare(nameOf(b), undefined, { sensitivity: "base" })
        );
}
