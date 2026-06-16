// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useForkSet — reactive composition of the fork set for an agent pane. Re-derives
 * `computeForkSet(...)` whenever the definition list, the open-block map, or the
 * pane's active definition changes, so the `<ForkBar>` stays in sync with no
 * parallel state (the "derive from a source of truth" rule).
 *
 * The caller injects the three reactive sources — the agent-definition list
 * (`useAgentDefinitions`), the open-definition→block map (`getOpenDefinitionMap`,
 * refreshed on `agents:changed`), and the pane's active definition id (`agentId`
 * meta) — keeping this hook pure and testable.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md §6.
 */

import { createMemo, type Accessor } from "solid-js";
import { computeForkSet, type ForkDefinition, type ForkSetEntry } from "./fork-set";

export interface UseForkSetOpts {
    /** All known agent definitions (lineage via `parent_id`). */
    definitions: Accessor<ReadonlyArray<ForkDefinition>>;
    /** definitionId → open blockId for currently-open panes. */
    openBlockByDef: Accessor<ReadonlyMap<string, string>>;
    /** The pane's active conversation's definition id. */
    activeDefinitionId: Accessor<string>;
}

/** The fork set for this pane, recomputed reactively. Empty when the active
 *  definition isn't in the list (e.g. not loaded yet). */
export function useForkSet(opts: UseForkSetOpts): Accessor<ForkSetEntry[]> {
    return createMemo(() =>
        computeForkSet(opts.definitions(), opts.openBlockByDef(), opts.activeDefinitionId()),
    );
}
