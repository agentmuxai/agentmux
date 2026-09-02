// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SkillCatalogModel — view model for the Armory's Skills tab. Drives list +
 * create/edit/delete, plus binding a global skill to an agent, over the
 * window-scoped `skill.catalog.*` App API
 * (agentmux-srv/src/server/app_api/skill.rs) — no `agent_id`/`check_s1`
 * context, since the Armory's connection is never agent-authenticated.
 * Every row here is global by construction — the catalog only ever
 * lists/creates/edits is_global rows. Per-agent private skills, and
 * unbinding, live in the Agent-setup modal (AgentSkillModel), not here.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";

export interface SkillDraft {
    id?: string;
    name: string;
    trigger: string;
    skill_type: string;
    description: string;
    content: string;
}

function emptySkillDraft(): SkillDraft {
    return { id: undefined, name: "", trigger: "", skill_type: "prompt", description: "", content: "" };
}

function draftFromSkill(s: Skill): SkillDraft {
    return {
        id: s.id,
        name: s.name,
        trigger: s.trigger,
        skill_type: s.skill_type,
        description: s.description,
        content: s.content,
    };
}

export class SkillCatalogModel {
    // Cross-window reactivity (SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md) —
    // same rationale as McpCatalogModel's own unsubChanged.
    private unsubChanged: () => void;

    private _skills = createSignal<SkillCatalogItem[]>([]);
    skillsAtom: Accessor<SkillCatalogItem[]> = this._skills[0];
    private setSkills = this._skills[1];

    private _selectedId = createSignal<string | null>(null);
    selectedIdAtom: Accessor<string | null> = this._selectedId[0];
    setSelectedId = this._selectedId[1];

    private _draft = createSignal<SkillDraft | null>(null);
    draftAtom: Accessor<SkillDraft | null> = this._draft[0];
    setDraft = this._draft[1];

    private _saving = createSignal<boolean>(false);
    savingAtom: Accessor<boolean> = this._saving[0];
    private setSaving = this._saving[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    // Catalog-side bind action (#1960 gap #3) — the agent picker's options,
    // and the id the user currently has selected in it.
    private _agents = createSignal<AgentDefinition[]>([]);
    agentsAtom: Accessor<AgentDefinition[]> = this._agents[0];
    private setAgents = this._agents[1];

    private _bindAgentId = createSignal<string>("");
    bindAgentIdAtom: Accessor<string> = this._bindAgentId[0];
    setBindAgentId = this._bindAgentId[1];

    selectedAtom: Accessor<SkillCatalogItem | null>;

    constructor() {
        this.selectedAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.skillsAtom().find((s) => s.id === id) ?? null;
        });
        void this.refresh();
        void this.loadAgents();
        this.unsubChanged = waveEventSubscribe({
            eventType: "skills:changed",
            handler: () => void this.refresh(),
        });
    }

    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.SkillCatalogListCommand(TabRpcClient, {});
            this.setSkills(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load skills: ${(e as Error).message ?? e}`);
        }
    }

    // Populates the catalog-side bind picker. Best-effort: a failure here
    // shouldn't block viewing/editing the catalog, only disable binding.
    async loadAgents(): Promise<void> {
        try {
            const agents = await RpcApi.ListAgentDefinitionsCommand(TabRpcClient);
            this.setAgents(agents);
        } catch {
            this.setAgents([]);
        }
    }

    async bindToAgent(skillId: string, agentId: string): Promise<void> {
        if (!agentId) {
            this.setError("Pick an agent to bind to.");
            return;
        }
        this.setError(null);
        try {
            await RpcApi.SkillCatalogBindCommand(TabRpcClient, { agent_id: agentId, skill_id: skillId });
            await this.refresh();
        } catch (e) {
            this.setError(`Bind failed: ${(e as Error).message ?? e}`);
        }
    }

    handleSelect(skill: Skill): void {
        this.setError(null);
        this.setDraft(null);
        this.setSelectedId(skill.id);
    }

    startNew(): void {
        this.setError(null);
        this.setDraft(emptySkillDraft());
        this.setSelectedId(null);
    }

    startEdit(skill: Skill): void {
        this.setError(null);
        this.setDraft(draftFromSkill(skill));
        this.setSelectedId(skill.id);
    }

    cancelDraft(): void {
        this.setDraft(null);
        this.setError(null);
    }

    async saveDraft(): Promise<void> {
        const draft = this.draftAtom();
        if (!draft) return;
        if (!draft.name.trim()) {
            this.setError("Name is required.");
            return;
        }
        this.setSaving(true);
        this.setError(null);
        try {
            const saved = await RpcApi.SkillCatalogUpsertCommand(TabRpcClient, {
                id: draft.id,
                name: draft.name.trim(),
                trigger: draft.trigger,
                skill_type: draft.skill_type || "prompt",
                description: draft.description,
                content: draft.content,
            });
            await this.refresh();
            this.setDraft(null);
            this.setSelectedId(saved.id);
        } catch (e) {
            this.setError(`Save failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setSaving(false);
        }
    }

    async deleteSkill(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.SkillCatalogDeleteCommand(TabRpcClient, { id });
            if (this.selectedIdAtom() === id) this.setSelectedId(null);
            this.setDraft(null);
            await this.refresh();
        } catch (e) {
            this.setError(`Delete failed: ${(e as Error).message ?? e}`);
        }
    }

    dispose(): void {
        this.unsubChanged();
    }
}
