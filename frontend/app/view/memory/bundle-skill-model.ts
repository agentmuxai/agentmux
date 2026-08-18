// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleSkillModel — view model for a bundle's Skills section (part of the
 * Bundle editor, memory-manager.tsx). Same shape as AgentSkillModel
 * (agent-skill-model.ts) and BundleMcpModel (bundle-mcp-model.ts) — see
 * BundleMcpModel's doc comment for why `addPrivate` is the actually-
 * functional path, not bind/unbind of existing globals.
 */

import { createMemo, createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";

export class BundleSkillModel {
    readonly bundleId: string;

    private _skills = createSignal<SkillBundleListItem[]>([]);
    skillsAtom: Accessor<SkillBundleListItem[]> = this._skills[0];
    private setSkills = this._skills[1];

    private _selectedId = createSignal<string | null>(null);
    selectedIdAtom: Accessor<string | null> = this._selectedId[0];
    setSelectedId = this._selectedId[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    private _adding = createSignal<boolean>(false);
    addingAtom: Accessor<boolean> = this._adding[0];
    setAdding = this._adding[1];

    selectedAtom: Accessor<SkillBundleListItem | null>;

    private unsubChanged: () => void;

    constructor(bundleId: string) {
        this.bundleId = bundleId;
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
            const list = await RpcApi.SkillCatalogListForBundleCommand(TabRpcClient, { bundle_id: this.bundleId });
            this.setSkills(list);
            this.setError(null);
        } catch (e) {
            this.setError(`Failed to load skills: ${(e as Error).message ?? e}`);
        }
    }

    handleSelect(skill: SkillBundleListItem): void {
        this.setError(null);
        this.setSelectedId(skill.id);
    }

    async bind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.SkillCatalogBindToBundleCommand(TabRpcClient, { bundle_id: this.bundleId, skill_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Bind failed: ${(e as Error).message ?? e}`);
        }
    }

    async unbind(id: string): Promise<void> {
        this.setError(null);
        try {
            await RpcApi.SkillCatalogUnbindFromBundleCommand(TabRpcClient, { bundle_id: this.bundleId, skill_id: id });
            await this.refresh();
        } catch (e) {
            this.setError(`Unbind failed: ${(e as Error).message ?? e}`);
        }
    }

    /** Creates a NEW, PRIVATE skill scoped directly to this bundle — see
     *  BundleMcpModel.addPrivate's identical doc comment, including why
     *  this returns a success boolean instead of Promise<void> (reagentx
     *  P1 on PR #2647). */
    async addPrivate(name: string, content: string): Promise<boolean> {
        this.setError(null);
        this.setAdding(true);
        try {
            await RpcApi.SkillCatalogUpsertForBundleCommand(TabRpcClient, {
                bundle_id: this.bundleId,
                name,
                content,
            });
            await this.refresh();
            return true;
        } catch (e) {
            this.setError(`Add failed: ${(e as Error).message ?? e}`);
            return false;
        } finally {
            this.setAdding(false);
        }
    }

    dispose(): void {
        this.unsubChanged();
    }
}
