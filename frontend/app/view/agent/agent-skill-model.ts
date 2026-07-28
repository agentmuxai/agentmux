// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSkillModel — view model for the agent pane's Skills tab (part of
 * AgentStashModal). Same shape as AgentMcpModel — see its doc comment for
 * the is_global / bound_to_agent / no-check_s1 details, which apply
 * identically here. A reactive, read-only view of the standalone Skill
 * primitive (`skill.*` App API, agentmux-srv/src/server/app_api/skill.rs)
 * plus a Bind/Unbind toggle — global skills are authored in the Armory,
 * not here.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";

export class AgentSkillModel {
    readonly agentId: string;

    private _skills = createSignal<SkillListItem[]>([]);
    skillsAtom: Accessor<SkillListItem[]> = this._skills[0];
    private setSkills = this._skills[1];

    private _selectedId = createSignal<string | null>(null);
    selectedIdAtom: Accessor<string | null> = this._selectedId[0];
    setSelectedId = this._selectedId[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    selectedAtom: Accessor<SkillListItem | null>;

    private unsubChanged: () => void;

    constructor(agentId: string) {
        this.agentId = agentId;
        this.selectedAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.skillsAtom().find((s) => s.id === id) ?? null;
        });
        void this.refresh();
        this.unsubChanged = waveEventSubscribe({
            eventType: "skills:changed",
            handler: () => void this.refresh(),
        });
    }

    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.SkillCatalogListForAgentCommand(TabRpcClient, { agent_id: this.agentId });
            this.setSkills(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load skills: ${(e as Error).message ?? e}`);
        }
    }

    handleSelect(skill: SkillListItem): void {
        this.setError(null);
        this.setSelectedId(skill.id);
    }

    async bind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.SkillCatalogBindCommand(TabRpcClient, { agent_id: this.agentId, skill_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Bind failed: ${(e as Error).message ?? e}`);
        }
    }

    async unbind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.SkillCatalogUnbindCommand(TabRpcClient, { agent_id: this.agentId, skill_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Unbind failed: ${(e as Error).message ?? e}`);
        }
    }

    dispose(): void {
        this.unsubChanged();
    }
}
