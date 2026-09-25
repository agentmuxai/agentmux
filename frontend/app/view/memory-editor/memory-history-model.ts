// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryHistoryModel — view model for one memory's version history (list,
 * diff any two versions, revert to one) plus its current content, over a
 * PLUGGABLE data source. Was `NativeMemoryHistoryModel` with the
 * `agent:memory:*` RPCs hard-coded; split out so Global Memory
 * (`globalmemory:*`, db_bundle_versions) drives the same `MemoryHistory`
 * component — docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3.
 * `NativeMemoryHistoryModel` (../agent/native-memory-history-model.ts) is
 * now this class plus the native source.
 */

import { createSignal, type Accessor } from "solid-js";

/** The fields of a version the history UI renders — common to
 *  `NativeMemoryVersionMeta` and `GlobalMemoryVersionMeta`. */
export interface MemoryVersionMeta {
    id: string;
    content_hash: string;
    source: string;
    source_detail: string;
    created_at: number;
    /** Global Memory only: the trusted writer (an agent id, "armory-ui"). */
    written_by?: string;
}

export interface MemoryHistorySource<V extends MemoryVersionMeta = MemoryVersionMeta> {
    /** Newest first. */
    listVersions(): Promise<V[]>;
    /** Diff text in `line_diff` format (`"  "` / `"- "` / `"+ "` prefixes). */
    diff(fromVersionId: string, toVersionId: string): Promise<string>;
    /** Restore `versionId` as a NEW version. */
    revert(versionId: string): Promise<void>;
    /** The live content; omitted when the caller owns content itself. */
    readContent?(): Promise<string>;
}

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
    versions: Array<{ id: string }>,
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
export function sourceWarning(v: Pick<MemoryVersionMeta, "source" | "source_detail">): string | null {
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

export class MemoryHistoryModel<V extends MemoryVersionMeta = MemoryVersionMeta> {
    protected readonly source: MemoryHistorySource<V>;

    private _versions = createSignal<V[]>([]);
    versionsAtom: Accessor<V[]> = this._versions[0];
    private setVersions = this._versions[1];

    private _loading = createSignal<boolean>(false);
    loadingAtom: Accessor<boolean> = this._loading[0];
    private setLoading = this._loading[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    private setError = this._error[1];

    /** The current (live) content — `null` means still loading (or no
     *  content source), an empty string means genuinely no content. Mirrors
     *  `AgentNativeMemoryModel.contentAtom`'s own convention so both
     *  surfaces agree on what "no value yet" looks like. */
    private _content = createSignal<string | null>(null);
    contentAtom: Accessor<string | null> = this._content[0];
    private setContent = this._content[1];

    /** Separate from `errorAtom` so a caller can tell a content-fetch
     *  failure apart from a history-fetch failure. */
    private _contentError = createSignal<string | null>(null);
    contentErrorAtom: Accessor<string | null> = this._contentError[0];
    private setContentError = this._contentError[1];

    /** True only while a content fetch is actually in flight — distinct
     *  from `contentAtom() === null`, which also describes "fetch failed,
     *  nothing to show" and must not keep rendering a loading state. */
    private _contentLoading = createSignal<boolean>(true);
    contentLoadingAtom: Accessor<boolean> = this._contentLoading[0];
    private setContentLoading = this._contentLoading[1];

    /* codex P2 on PR #3218: the constructor's initial loadContent() and a
     * revert-triggered loadContent() can both be in flight at once; without
     * a request-id guard (mirroring latestDiffRequestId below) the older
     * response can resolve second and clobber the newer one. */
    private latestContentRequestId = 0;

    /** Guards loadHistory against a stale response — a change-event refresh
     *  can now reload history in place (no remount) while an earlier load
     *  is still in flight. */
    private latestHistoryRequestId = 0;

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
     *  diff request resolves, could otherwise let the stale response
     *  overwrite diffTextAtom after the newer selection's request already
     *  completed. */
    private latestDiffRequestId = 0;

    /** Called after a successful revert with the restored content, so a
     *  caller showing "current content" elsewhere can refresh. */
    onReverted?: (newContent: string) => void;

    constructor(source: MemoryHistorySource<V>) {
        this.source = source;
        void this.loadHistory();
        if (source.readContent) {
            void this.loadContent();
        } else {
            this.setContentLoading(false);
        }
    }

    /** Fetch the current content. Returns whether THIS call's result was the
     *  one actually applied — `false` on a stale (superseded) or failed
     *  response, so a caller like `revertTo` can tell "the content shown now
     *  truly reflects this call" apart from "some load happened but this one
     *  didn't win". */
    async loadContent(): Promise<boolean> {
        if (!this.source.readContent) return false;
        const requestId = ++this.latestContentRequestId;
        this.setContentLoading(true);
        this.setContentError(null);
        try {
            const content = await this.source.readContent();
            if (requestId !== this.latestContentRequestId) return false;
            this.setContent(content);
            this.setContentLoading(false);
            return true;
        } catch (e) {
            if (requestId !== this.latestContentRequestId) return false;
            this.setContentError(`Failed to load content: ${(e as Error).message ?? e}`);
            this.setContentLoading(false);
            return false;
        }
    }

    async loadHistory(): Promise<void> {
        const requestId = ++this.latestHistoryRequestId;
        this.setLoading(true);
        this.setError(null);
        try {
            const versions = await this.source.listVersions();
            if (requestId !== this.latestHistoryRequestId) return;
            this.setVersions(versions);
        } catch (e) {
            if (requestId !== this.latestHistoryRequestId) return;
            this.setError(`Failed to load history: ${(e as Error).message ?? e}`);
        } finally {
            if (requestId === this.latestHistoryRequestId) this.setLoading(false);
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
            const diff = await this.source.diff(from, to);
            if (requestId !== this.latestDiffRequestId) return;
            this.setDiffText(diff);
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
            await this.source.revert(versionId);
            this.clearDiffSelection();
            await this.loadHistory();
            // Only forward on a genuine, non-stale success — reagent P1 +
            // codex P2 on PR #3218: forwarding contentAtom() unconditionally
            // could hand the caller a stale pre-revert value (or null) if
            // this refresh failed, silently presenting old content as if it
            // were the freshly reverted content. The failure is still
            // visible via contentErrorAtom.
            const refreshed = await this.loadContent();
            if (refreshed && this.onReverted) {
                this.onReverted(this.contentAtom() ?? "");
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
