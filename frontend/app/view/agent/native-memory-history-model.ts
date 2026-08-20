// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NativeMemoryHistoryModel — view model for one memory file's version
 * history: list, diff any two versions, revert to a prior version. Backs
 * `NativeMemoryHistoryPanel`, which is mounted from two places — the
 * agent's own Stash "Memory" tab (`AgentNativeMemoryModal`) and Armory's
 * "Native Memory" tab (agent-picker + this same panel) — both reading the
 * identical `agent:memory:history/diff/revert` RPCs, so there is one
 * source of truth and two entry points, per
 * docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md §4.3.
 */

import { createSignal, type Accessor } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/** Order two version ids oldest-first, given `versions` in the model's own
 *  newest-first order (matches `agent:memory:history`'s response order —
 *  see `agent_native_memory_version_list`'s `ORDER BY created_at DESC` on
 *  the backend). Extracted as a standalone, unit-testable function after
 *  reagent P1 caught this exact ordering inverted in an earlier revision —
 *  a larger index in a newest-first list means an OLDER version, easy to
 *  get backwards inline. Returns `[idA, idB]` unchanged if either id isn't
 *  found in `versions` (defensive default; callers only invoke this with
 *  ids known to be present). */
export function orderVersionsOldestFirst(
    idA: string,
    idB: string,
    versions: NativeMemoryVersionMeta[],
): [string, string] {
    const indexOf = (id: string) => versions.findIndex((v) => v.id === id);
    const idxA = indexOf(idA);
    const idxB = indexOf(idB);
    if (idxA < 0 || idxB < 0) return [idA, idB];
    return idxA > idxB ? [idA, idB] : [idB, idA];
}

/** Human label for a version's `source` field. */
export function sourceLabel(source: string): string {
    switch (source) {
        case "human": return "Human";
        case "agent_inferred": return "Agent";
        case "jekt": return "Jekt";
        case "external_fs_write": return "Detected outside AgentMux";
        case "revert": return "Revert";
        default: return source;
    }
}

/** Whether a version's source warrants a visible warning tag — a claim
 *  about *why* the write happened that the operator should specifically
 *  notice, per the spec's §4.4 (distinguishing "unverified sender" from
 *  "untracked write path" — two materially different weaker claims). */
export function sourceWarning(v: NativeMemoryVersionMeta): string | null {
    if (v.source === "jekt") {
        let tier = "";
        let trust = "";
        try {
            const detail = JSON.parse(v.source_detail || "{}") as Record<string, unknown>;
            tier = typeof detail.TIER === "string" ? detail.TIER : "";
            trust = typeof detail.TRUST === "string" ? detail.TRUST : "";
        } catch {
            // source_detail wasn't valid JSON — fall through with an empty tier/trust.
        }
        const parts = [tier && `TIER=${tier}`, trust && `TRUST=${trust}`].filter(Boolean);
        return `written in response to a jekt${parts.length ? ` — ${parts.join(", ")}` : ""}`;
    }
    if (v.source === "external_fs_write") {
        let detectedVia = "";
        try {
            const detail = JSON.parse(v.source_detail || "{}") as Record<string, unknown>;
            detectedVia = typeof detail.detected_via === "string" ? detail.detected_via : "";
        } catch {
            // ignore
        }
        return `detected outside AgentMux's write path — provenance unknown${detectedVia ? ` (${detectedVia})` : ""}`;
    }
    return null;
}

export class NativeMemoryHistoryModel {
    readonly agentId: string;
    readonly filename: string;

    private _versions = createSignal<NativeMemoryVersionMeta[]>([]);
    versionsAtom: Accessor<NativeMemoryVersionMeta[]> = this._versions[0];
    private setVersions = this._versions[1];

    private _loading = createSignal<boolean>(false);
    loadingAtom: Accessor<boolean> = this._loading[0];
    private setLoading = this._loading[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    private setError = this._error[1];

    /** Up to two version ids selected for comparison, oldest-first once both are set. */
    private _diffSelection = createSignal<string[]>([]);
    diffSelectionAtom: Accessor<string[]> = this._diffSelection[0];
    private setDiffSelection = this._diffSelection[1];

    private _diffText = createSignal<string | null>(null);
    diffTextAtom: Accessor<string | null> = this._diffText[0];
    private setDiffText = this._diffText[1];

    private _diffLoading = createSignal<boolean>(false);
    diffLoadingAtom: Accessor<boolean> = this._diffLoading[0];
    private setDiffLoading = this._diffLoading[1];

    private _reverting = createSignal<boolean>(false);
    revertingAtom: Accessor<boolean> = this._reverting[0];
    private setReverting = this._reverting[1];

    /** reagent P2 on PR #2678: guards computeDiff against a stale response —
     *  selecting one version pair, then a different pair before the first
     *  NativeMemoryDiffCommand resolves, could otherwise let the stale
     *  response overwrite diffTextAtom after the newer selection's request
     *  already completed. */
    private latestDiffRequestId = 0;

    /** Called after a successful revert so the caller (e.g.
     *  AgentNativeMemoryModel) can refresh the live file content it shows
     *  elsewhere — this model only owns history/diff/revert state, not the
     *  "current content" view that already exists on the sibling model. */
    onReverted?: (newContent: string) => void;

    constructor(agentId: string, filename: string) {
        this.agentId = agentId;
        this.filename = filename;
        void this.loadHistory();
    }

    async loadHistory(): Promise<void> {
        this.setLoading(true);
        this.setError(null);
        try {
            const res = await RpcApi.NativeMemoryHistoryCommand(TabRpcClient, {
                agent_id: this.agentId,
                filename: this.filename,
            });
            this.setVersions(res.versions);
        } catch (e) {
            this.setError(`Failed to load history: ${(e as Error).message ?? e}`);
        } finally {
            this.setLoading(false);
        }
    }

    /** Toggle a version in/out of the (at most 2) diff selection. Selecting
     *  a third clears down to just the newly-clicked one — simpler than a
     *  queue, and matches "pick two things to compare" as the only real
     *  use case. */
    toggleDiffSelection(versionId: string): void {
        // Any change to the selection invalidates whatever diff request (if
        // any) was previously in flight — otherwise a stale response could
        // still land afterward and repopulate diffTextAtom for a selection
        // the user has since abandoned, even on a path (like selecting a
        // third id, below) that doesn't itself start a new diff request.
        this.latestDiffRequestId++;
        const current = this.diffSelectionAtom();
        if (current.includes(versionId)) {
            this.setDiffSelection(current.filter((id) => id !== versionId));
            this.setDiffText(null);
            return;
        }
        const next = current.length >= 2 ? [versionId] : [...current, versionId];
        this.setDiffSelection(next);
        this.setDiffText(null);
        if (next.length === 2) void this.computeDiff(next[0], next[1]);
    }

    private async computeDiff(idA: string, idB: string): Promise<void> {
        // Order oldest -> newest so the diff reads as "what changed since
        // the earlier version", regardless of click order.
        const [from, to] = orderVersionsOldestFirst(idA, idB, this.versionsAtom());

        // toggleDiffSelection already bumped latestDiffRequestId for this
        // call — capture it as-is rather than bumping again here.
        const requestId = this.latestDiffRequestId;
        this.setDiffLoading(true);
        this.setError(null);
        try {
            const res = await RpcApi.NativeMemoryDiffCommand(TabRpcClient, {
                agent_id: this.agentId,
                from_version_id: from,
                to_version_id: to,
            });
            if (requestId !== this.latestDiffRequestId) return;
            this.setDiffText(res.diff);
        } catch (e) {
            if (requestId !== this.latestDiffRequestId) return;
            this.setError(`Failed to load diff: ${(e as Error).message ?? e}`);
        } finally {
            if (requestId === this.latestDiffRequestId) this.setDiffLoading(false);
        }
    }

    clearDiffSelection(): void {
        this.latestDiffRequestId++;
        this.setDiffSelection([]);
        this.setDiffText(null);
    }

    /** Revert to `versionId` — recorded as a NEW version (source "revert"),
     *  never a rewrite of history. Reloads history afterward so the new
     *  version shows up immediately. */
    async revertTo(versionId: string): Promise<void> {
        this.setReverting(true);
        this.setError(null);
        try {
            await RpcApi.NativeMemoryRevertCommand(TabRpcClient, {
                agent_id: this.agentId,
                filename: this.filename,
                target_version_id: versionId,
            });
            this.clearDiffSelection();
            await this.loadHistory();
            const reverted = this.versionsAtom().find((v) => v.id === versionId);
            if (reverted && this.onReverted) {
                // Fetch the reverted content directly rather than trusting
                // any cached copy — read_file is cheap and this only fires
                // on an explicit user action, not a hot path.
                const read = await RpcApi.NativeMemoryReadFileCommand(TabRpcClient, {
                    agent_id: this.agentId,
                    filename: this.filename,
                });
                this.onReverted(read.content);
            }
        } catch (e) {
            this.setError(`Revert failed: ${(e as Error).message ?? e}`);
        } finally {
            this.setReverting(false);
        }
    }

    dispose(): void {
        // Solid signals are GC'd with the instance; nothing to unsubscribe.
    }
}
