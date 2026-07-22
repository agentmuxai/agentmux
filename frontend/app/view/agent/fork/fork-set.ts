// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * fork-set — pure derivation of the **fork set** for an agent pane: the active
 * conversation's lineage root plus every fork descended from it, in the order
 * the fork bar renders them (root first, then by creation time).
 *
 * A fork is an `AgentDefinition` linked by `parent_id` to the one it branched
 * from (created via `ForkAgentDefinitionCommand`). The fork set is therefore a
 * pure function of the definition list + which definitions are currently open
 * as panes — no parallel state, matching the "derive from a source of truth"
 * rule (SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15 §5.3). The fork BAR (a
 * later phase) renders these via <PaneRow>; the in-pane block-stack swap wires
 * the `blockId` to the active view.
 *
 * Spec: docs/specs/SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md §6.
 */

/** Minimal definition shape this module needs (subset of AgentDefinition). */
export interface ForkDefinition {
    id: string;
    name: string;
    /**
     * Forked-from definition id; empty/undefined for a root.
     *
     * NOTE: the backend also writes this field when a definition is
     * instantiated from a catalog template (`agentdefcreatefromtemplate`
     * sets it to the template's id — `db_agents.parent_template_id`,
     * wire-mapped to `parent_id` for back-compat). That's template
     * *provenance*, not a conversation fork — two unrelated agents cloned
     * from the same template must NOT be treated as forks of each other.
     * `hasParent` below excludes template parents for exactly this reason.
     */
    parent_id?: string;
    /** Free-form branch label (e.g. "pr-422-review"); empty for a root. */
    branch_label?: string;
    /** Epoch ms — orders siblings oldest-first under the root. */
    created_at: number;
    /** 1 for a seeded catalog template, 0 for a user-owned agent. */
    is_seeded?: number;
}

/** One row in the fork bar. */
export interface ForkSetEntry {
    definitionId: string;
    /** Display title: branch label when set, else the definition name. */
    title: string;
    /** True for the lineage root (rendered as the base entry). */
    isRoot: boolean;
    /** True for the pane's currently-active conversation. */
    isActive: boolean;
    /** The open block running this fork, if any (undefined = not open). */
    blockId?: string;
    /** Distance from the root (0 = root) — for optional indentation/ordering. */
    depth: number;
}

/** Guard against a malformed `parent_id` cycle. Lineages are tiny in practice. */
const MAX_LINEAGE_WALK = 1000;

function titleOf(d: ForkDefinition): string {
    const label = d.branch_label?.trim();
    return label && label.length > 0 ? label : d.name;
}

/**
 * Is `parent_id` a usable link to a definition that exists in the set?
 *
 * Excludes template parents: `parent_id` doubles as "cloned from this
 * template" provenance (see the `ForkDefinition.parent_id` doc comment),
 * which is not a fork lineage. Without this check, every definition ever
 * created from the same template would walk up to that template as a
 * shared lineage root and appear as forks of each other — even when they
 * have no fork relationship at all.
 *
 * Requires `is_seeded === 0` (not just `!== 1`) so a parent with an
 * unpopulated `is_seeded` — e.g. a legacy row predating the column — is
 * excluded too, rather than defaulting to "trusted as a real fork parent."
 */
function hasParent(d: ForkDefinition, byId: Map<string, ForkDefinition>): boolean {
    const p = d.parent_id;
    if (!p || p.length === 0) return false;
    const parent = byId.get(p);
    return !!parent && parent.is_seeded === 0;
}

/**
 * Compute the fork set for the pane whose active conversation is
 * `activeDefinitionId`.
 *
 * - Walks `parent_id` up to the lineage **root** (a definition with no parent,
 *   or whose parent isn't in `definitions`).
 * - Collects the root + every descendant fork.
 * - Orders root-first, then breadth-first by `created_at` (stable, oldest
 *   sibling first) so the bar reads like a timeline of branches.
 *
 * Returns `[]` when `activeDefinitionId` isn't in `definitions` (e.g. the pane
 * isn't an agent, or the list hasn't loaded yet) — callers render no bar.
 *
 * @param definitions   all known agent definitions (lineage via `parent_id`).
 * @param openBlockByDef definitionId → open blockId for currently-open panes.
 * @param activeDefinitionId the pane's active conversation's definition id.
 */
export function computeForkSet(
    definitions: ReadonlyArray<ForkDefinition>,
    openBlockByDef: ReadonlyMap<string, string>,
    activeDefinitionId: string,
): ForkSetEntry[] {
    const byId = new Map<string, ForkDefinition>();
    for (const d of definitions) byId.set(d.id, d);

    const active = byId.get(activeDefinitionId);
    if (!active) return [];

    // 1) Walk to the lineage root.
    let root = active;
    for (let i = 0; i < MAX_LINEAGE_WALK && hasParent(root, byId); i++) {
        root = byId.get(root.parent_id!)!;
    }

    // 2) Children index for a stable breadth-first descent from the root.
    const childrenOf = new Map<string, ForkDefinition[]>();
    for (const d of definitions) {
        if (hasParent(d, byId)) {
            const arr = childrenOf.get(d.parent_id!) ?? [];
            arr.push(d);
            childrenOf.set(d.parent_id!, arr);
        }
    }
    for (const arr of childrenOf.values()) {
        // Oldest sibling first; tie-break on id for determinism.
        arr.sort((a, b) => a.created_at - b.created_at || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    }

    // 3) BFS from the root, depth-tracked, cycle-guarded by a visited set.
    const out: ForkSetEntry[] = [];
    const visited = new Set<string>();
    const queue: Array<{ def: ForkDefinition; depth: number }> = [{ def: root, depth: 0 }];
    while (queue.length > 0 && out.length < MAX_LINEAGE_WALK) {
        const { def, depth } = queue.shift()!;
        if (visited.has(def.id)) continue;
        visited.add(def.id);
        out.push({
            definitionId: def.id,
            title: titleOf(def),
            isRoot: def.id === root.id,
            isActive: def.id === activeDefinitionId,
            blockId: openBlockByDef.get(def.id),
            depth,
        });
        for (const child of childrenOf.get(def.id) ?? []) {
            if (!visited.has(child.id)) queue.push({ def: child, depth: depth + 1 });
        }
    }

    return out;
}
