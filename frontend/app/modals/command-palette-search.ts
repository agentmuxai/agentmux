// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The command palette's search, apart from the modal so it can be tested
// without rendering it. Literal first, like the My Agents filter: typing a
// command's name lists that command, not every command that looks like it.

import type { CommandEntry } from "@/app/store/command-registry";
import { literalFirstSearch } from "@/app/util/fuzzysearch";

// Fuzzy fallback, used only when no command contains the query anywhere.
const FUZZY_KEYS = [
    { name: "label", weight: 0.5 },
    { name: "keywords", weight: 0.2 },
    { name: "category", weight: 0.2 },
    { name: "id", weight: 0.1 },
];

/**
 * Commands matching `query`, best first. A command whose label contains the
 * query comes before one that only matches in its keywords, category or id
 * (so "zoom" lists Zoom In and Zoom Out, then Actual Size, whose id is
 * `view:zoom:reset`). Ties keep `commands` order. Only when no command
 * contains the query does fuzzy matching step in, for a typo. Returns
 * `commands` unchanged for a blank query.
 */
export function searchCommands(commands: CommandEntry[], query: string): CommandEntry[] {
    return literalFirstSearch(commands, query, (c) => [c.label, c.keywords, c.category, c.id], { keys: FUZZY_KEYS });
}
