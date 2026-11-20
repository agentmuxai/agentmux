// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A file with unsaved edits that changed on disk meanwhile: the Editor asks Reload or Keep mine (a panel inside the
 * pane, EditorDiskConflictPanel) and a save never overwrites the other change silently.
 *
 * "Changed on disk" means the file's current content hashes differently from what the tab last loaded or saved
 * (`tab.contentHash`), so the watcher's echo of the Editor's own save, or a touch that changes nothing, is not a
 * change. A file that cannot be read now (deleted, moved) is not a conflict either: a save then writes it afresh.
 * Scratch buffers have no other writer and are never checked.
 */

import { createSignal } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { sha256Hex } from "./editor-hash";

/** The part of a tab the check reads. */
export interface DiskConflictTab {
    id: string;
    filePath: string;
    contentHash?: string;
    isScratch?: boolean;
}

/** What the Editor's model supplies. */
export interface DiskConflictHost {
    /** The tab as it is now (a read takes time, and the tab may have been saved, changed or closed meanwhile). */
    current(tabId: string): (DiskConflictTab & { dirty?: boolean }) | undefined;
    /** The connection and block of a read for this tab (a host's file is read there). */
    on(tabId: string): { connection?: string; block_id?: string };
    /** Discard the tab's edits and show the file's current content. */
    reload(tab: DiskConflictTab): Promise<void>;
    /** Save the tab's buffer over the file, without asking again. */
    overwrite(tab: DiskConflictTab): Promise<void>;
}

export class DiskConflicts {
    private readonly flagged = createSignal<ReadonlySet<string>>(new Set());
    /** The disk version a person chose to overwrite ("Keep mine"): not asked about again until the file changes again. */
    private readonly kept = new Map<string, string>();
    /** Tabs whose Save is waiting on the answer. */
    private readonly saving = new Set<string>();

    constructor(private readonly host: DiskConflictHost) {}

    /** True while the tab shows the question. Reactive. */
    has(tabId: string | undefined): boolean {
        return !!tabId && this.flagged[0]().has(tabId);
    }

    /** A tab closed, or a save of it went through (the disk holds its own content): nothing more to ask about it. */
    forget(tabId: string): void {
        this.kept.delete(tabId);
        this.saving.delete(tabId);
        this.setFlag(tabId, false);
    }

    /** The watcher saw the file of a tab with unsaved edits change: ask if it really differs from what the tab started from. */
    async noteChange(tab: DiskConflictTab): Promise<void> {
        if ((await this.diverged(tab)) && this.host.current(tab.id)?.dirty) this.setFlag(tab.id, true);
    }

    /** Before a save writes: true to go ahead, false when the file changed under the edits (the question is shown). */
    async guardSave(tab: DiskConflictTab): Promise<boolean> {
        if (this.has(tab.id) || (await this.diverged(tab))) {
            this.saving.add(tab.id);
            this.setFlag(tab.id, true);
            return false;
        }
        return true;
    }

    /** "Keep mine": the edits stay, the disk version is not asked about again, and a save that was waiting goes ahead. */
    async keep(tab: DiskConflictTab): Promise<void> {
        this.setFlag(tab.id, false);
        // A save is waiting: it goes now, with no read first (a read can take long, and the tab in front may change).
        if (this.saving.delete(tab.id)) return this.host.overwrite(tab);
        const disk = await this.diskHash(tab);
        if (disk !== null) this.kept.set(tab.id, disk);
    }

    /** "Reload": the edits go and the file's current content shows. */
    async reload(tab: DiskConflictTab): Promise<void> {
        this.kept.delete(tab.id);
        this.saving.delete(tab.id);
        this.setFlag(tab.id, false);
        await this.host.reload(tab);
    }

    private async diverged(tab: DiskConflictTab): Promise<boolean> {
        if (tab.isScratch || !tab.contentHash) return false;
        const disk = await this.diskHash(tab);
        // Against the tab as it is after the read: a save that finished meanwhile moved its hash to what is on disk.
        const now = this.host.current(tab.id);
        return disk !== null && !!now && disk !== now.contentHash && disk !== this.kept.get(tab.id);
    }

    /** The hash of the file as it is now, or null when it cannot be read. */
    private async diskHash(tab: DiskConflictTab): Promise<string | null> {
        try {
            const result = await RpcApi.ReadEditorFileCommand(
                TabRpcClient,
                { path: tab.filePath, ...this.host.on(tab.id) },
                this.host.on(tab.id).connection ? { timeout: 180_000 } : undefined,
            );
            return await sha256Hex(result?.content ?? "");
        } catch {
            return null;
        }
    }

    private setFlag(tabId: string, on: boolean): void {
        const [flagged, setFlagged] = this.flagged;
        if (flagged().has(tabId) === on) return;
        const next = new Set(flagged());
        if (on) next.add(tabId);
        else next.delete(tabId);
        setFlagged(next);
    }
}
