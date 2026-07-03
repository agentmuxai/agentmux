// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSkillModel — view model for the agent pane's Skills tab (part of
 * AgentSetupModal). Drives the list + create/edit/delete/bind lifecycle
 * over the standalone Skill primitive (`skill.*` App API,
 * agentmux-srv/src/server/app_api/skill.rs). Same shape as AgentMcpModel —
 * see its doc comment for the is_global / bound-status caveat, which
 * applies identically here.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

export interface SkillDraft {
    id?: string;
    name: string;
    trigger: string;
    skill_type: string;
    description: string;
    content: string;
}

export function emptySkillDraft(): SkillDraft {
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

export class AgentSkillModel {
    readonly agentId: string;

    private _skills = createSignal<Skill[]>([]);
    skillsAtom: Accessor<Skill[]> = this._skills[0];
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

    selectedAtom: Accessor<Skill | null>;

    constructor(agentId: string) {
        this.agentId = agentId;
        this.selectedAtom = createMemo(() => {
            const id = this.selectedIdAtom();
            if (!id) return null;
            return this.skillsAtom().find((s) => s.id === id) ?? null;
        });
        void this.refresh();
    }

    async refresh(): Promise<void> {
        try {
            const list = await RpcApi.SkillListCommand(TabRpcClient, { agent_id: this.agentId });
            this.setSkills(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load skills: ${(e as Error).message ?? e}`);
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
        if (skill.is_global) {
            this.setError("Global skills are managed in the Armory, not here.");
            return;
        }
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
            const saved = await RpcApi.SkillUpsertCommand(TabRpcClient, {
                agent_id: this.agentId,
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
        const target = this.skillsAtom().find((s) => s.id === id);
        if (target?.is_global) {
            this.setError("Global skills are managed in the Armory, not here.");
            return;
        }
        this.setError(null);
        try {
            await RpcApi.SkillDeleteCommand(TabRpcClient, { agent_id: this.agentId, id });
            if (this.selectedIdAtom() === id) this.setSelectedId(null);
            this.setDraft(null);
            await this.refresh();
        } catch (e) {
            this.setError(`Delete failed: ${(e as Error).message ?? e}`);
        }
    }

    async bind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.SkillBindCommand(TabRpcClient, { agent_id: this.agentId, skill_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Bind failed: ${(e as Error).message ?? e}`);
        }
    }

    async unbind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.SkillUnbindCommand(TabRpcClient, { agent_id: this.agentId, skill_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Unbind failed: ${(e as Error).message ?? e}`);
        }
    }

    dispose(): void {
        // Solid signals are GC'd with the instance; nothing to unsubscribe.
    }
}
