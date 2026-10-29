// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SkillMatrixModel — the agents × skills grid in Memory → Skills. One cell per agent and catalog
 * skill says how the agent gets the skill (an explicit bind, or the skill's "every agent" / "new
 * agents" reach), or that it does not, or that it opted out of the reach. A click changes it:
 * bind or unbind, or opt out of / back into a reach. The agent's files change at its next launch.
 */

import { createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentDefinition } from "@/app/store/rpc-api";
import type { SkillCatalogItem } from "@/types/rpc/SkillCatalogItem";

/** How one agent stands with one skill. */
export type CellState = "bound" | "reach-all" | "reach-new" | "opted-out" | "none";

export interface CellFacts {
    bound: boolean;
    reached: boolean;
    optedOut: boolean;
}

export const CELL_TEXT: Record<CellState, string> = {
    bound: "Bound",
    "reach-all": "Every agent",
    "reach-new": "New agents",
    "opted-out": "Opted out",
    none: "",
};

/** What the cell shows. An explicit bind wins over a reach. */
export function cellState(skill: SkillCatalogItem, facts: CellFacts | undefined): CellState {
    if (!facts) return "none";
    if (facts.bound) return "bound";
    if (facts.optedOut) return "opted-out";
    if (facts.reached) return skill.all_agents ? "reach-all" : "reach-new";
    return "none";
}

/** What clicking the cell does, in words (also its tooltip). */
export function cellAction(state: CellState): string {
    switch (state) {
        case "bound":
            return "Unbind";
        case "reach-all":
        case "reach-new":
            return "Opt out";
        case "opted-out":
            return "Opt back in";
        default:
            return "Bind";
    }
}

export class SkillMatrixModel {
    // agent id -> skill id -> facts
    private _cells = createSignal<ReadonlyMap<string, ReadonlyMap<string, CellFacts>>>(new Map());
    cellsAtom: Accessor<ReadonlyMap<string, ReadonlyMap<string, CellFacts>>> = this._cells[0];

    private _loading = createSignal(false);
    loadingAtom: Accessor<boolean> = this._loading[0];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];

    private seq = 0;

    /** Ask each agent what it gets. One that cannot be asked is left out of the grid. */
    async load(agents: AgentDefinition[]): Promise<void> {
        const mine = ++this.seq;
        this._loading[1](true);
        const rows = await Promise.all(
            agents.map(async (agent) => {
                try {
                    const list = await RpcApi.SkillCatalogListForAgentCommand(TabRpcClient, { agent_id: agent.id });
                    const facts = new Map<string, CellFacts>();
                    for (const s of list) facts.set(s.id, { bound: s.bound_to_agent, reached: s.reached, optedOut: s.opted_out });
                    return [agent.id, facts] as const;
                } catch {
                    return null;
                }
            }),
        );
        if (mine !== this.seq) return; // a newer load is under way
        this._cells[1](new Map(rows.filter((r) => r !== null)));
        this._loading[1](false);
    }

    /** Do what the cell's action says, then ask again. */
    async toggle(agent: AgentDefinition, skill: SkillCatalogItem, state: CellState, agents: AgentDefinition[]): Promise<void> {
        this._error[1](null);
        try {
            const ids = { agent_id: agent.id, skill_id: skill.id };
            if (state === "bound") await RpcApi.SkillCatalogUnbindCommand(TabRpcClient, ids);
            else if (state === "reach-all" || state === "reach-new") {
                await RpcApi.SkillCatalogSetOptOutCommand(TabRpcClient, { ...ids, opted_out: true });
            } else if (state === "opted-out") {
                await RpcApi.SkillCatalogSetOptOutCommand(TabRpcClient, { ...ids, opted_out: false });
            } else await RpcApi.SkillCatalogBindCommand(TabRpcClient, ids);
            await this.load(agents);
        } catch (e) {
            this._error[1](`Couldn't change ${agent.name}: ${(e as Error).message ?? e}`);
        }
    }
}
