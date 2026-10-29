// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SkillImportModel — the "Import a skill" panel in Memory → Skills. A skill comes from a folder, a
 * git repository (https) or one SKILL.md at an https URL. Reading a source writes nothing: the
 * person sees every skill's whole text, and for one that already exists a line diff against the
 * catalog, then ticks what to bring in. An imported skill lands bound to nobody and outside "new
 * agents get it". The same panel checks an imported skill against its source and, when the source
 * moved on, shows the diff and waits for the approval; nothing is applied silently.
 */

import { createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { SkillImportItem } from "@/types/rpc/SkillImportItem";
import type { SkillImportPreview } from "@/types/rpc/SkillImportPreview";
import type { SkillSource } from "@/types/rpc/SkillSource";

export type SkillSourceKind = "folder" | "git" | "url";

export const SOURCE_KIND_LABELS: Record<SkillSourceKind, string> = {
    folder: "A folder on this computer",
    git: "A git repository (https)",
    url: "A SKILL.md at an https address",
};

export class SkillImportModel {
    private _kind = createSignal<SkillSourceKind>("folder");
    kindAtom: Accessor<SkillSourceKind> = this._kind[0];
    setKind = this._kind[1];

    private _location = createSignal("");
    locationAtom: Accessor<string> = this._location[0];
    setLocation = this._location[1];

    private _gitRef = createSignal("");
    gitRefAtom: Accessor<string> = this._gitRef[0];
    setGitRef = this._gitRef[1];

    private _preview = createSignal<SkillImportPreview | null>(null);
    previewAtom: Accessor<SkillImportPreview | null> = this._preview[0];

    /** Keys of the previewed skills the person has ticked. */
    private _picked = createSignal<ReadonlySet<string>>(new Set());
    pickedAtom: Accessor<ReadonlySet<string>> = this._picked[0];

    private _busy = createSignal(false);
    busyAtom: Accessor<boolean> = this._busy[0];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];

    constructor(private readonly onDone: (savedIds: string[]) => void) {}

    /** Read the source the person typed. */
    async read(): Promise<void> {
        const location = this.locationAtom().trim();
        if (!location) {
            this._error[1]("Say where the skill is.");
            return;
        }
        const source: SkillSource = { kind: this.kindAtom(), location, git_ref: this.gitRefAtom().trim(), item_key: "" };
        await this.runPreview({ source, skill_id: "" });
    }

    /** Re-read an imported skill's own source, to see whether it changed. */
    async check(skillId: string): Promise<void> {
        await this.runPreview({ source: null, skill_id: skillId });
    }

    private async runPreview(req: { source: SkillSource | null; skill_id: string }): Promise<void> {
        this._busy[1](true);
        this._error[1](null);
        this._preview[1](null);
        try {
            const preview = await RpcApi.SkillCatalogImportPreviewCommand(TabRpcClient, req);
            this._preview[1](preview);
            // New skills start ticked. A skill that already exists and differs replaces the one in
            // the catalog, so on an import it starts unticked (the person decides after reading the
            // diff); on "check for changes" that replacement is the question being asked.
            const start = req.skill_id ? "changed" : "new";
            this._picked[1](new Set(preview.items.filter((i) => i.status === start).map((i) => i.key)));
        } catch (e) {
            this._error[1](cleanError(e));
        } finally {
            this._busy[1](false);
        }
    }

    toggle(item: SkillImportItem): void {
        const next = new Set(this.pickedAtom());
        if (!next.delete(item.key)) next.add(item.key);
        this._picked[1](next);
    }

    /** Write the ticked skills. */
    async commit(): Promise<void> {
        const preview = this.previewAtom();
        if (!preview) return;
        const items = preview.items.filter((i) => this.pickedAtom().has(i.key) && i.status !== "same");
        if (items.length === 0) return;
        this._busy[1](true);
        this._error[1](null);
        try {
            const res = await RpcApi.SkillCatalogImportCommitCommand(TabRpcClient, {
                source: preview.source,
                items: items.map((i) => ({
                    key: i.key,
                    name: i.name,
                    description: i.description,
                    content: i.content,
                    existing_id: i.existing_id,
                })),
            });
            this.onDone(res.saved);
        } catch (e) {
            this._error[1](cleanError(e));
        } finally {
            this._busy[1](false);
        }
    }
}

function cleanError(e: unknown): string {
    const msg = (e as Error)?.message ?? String(e);
    return msg.replace(/^skill\.catalog\.import_(preview|commit): /, "");
}
