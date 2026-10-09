// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The files attached to an agent pane's unsent message: what the composer's
 * tray shows, and what goes out with the next send.
 * docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §5, §6;
 * SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md (any file, not only images).
 *
 * Module-level and keyed by blockId, like the composer's text draft
 * (AgentFooter's composerDrafts): the footer unmounts on every pane-tab
 * switch, and the attachments must survive that. They don't survive an app
 * restart; the backend's retention sweep removes their files later.
 *
 * Two ways in:
 *  - paths (drop): `attachments.ingest` copies and processes them in the
 *    backend; progress arrives as `attachment:*` events for this block.
 *  - Files (Ctrl+V): each is streamed to `POST /api/v1/attachments/upload`
 *    with XMLHttpRequest, for upload progress. The bytes go from the Blob
 *    straight into the request, never into a JS ArrayBuffer.
 */

import { createSignal, type Accessor } from "solid-js";
import { authHeaders } from "@/app/store/auth-headers";
import { MOS, pushNotification } from "@/app/store/global";
import { WpsEvent } from "@/app/store/mps-events";
import { muxEventSubscribe } from "@/app/store/mps";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWebServerEndpoint } from "@/util/endpoints";
import { formatBytes } from "@/util/format-bytes";
import type { AttachmentBatchDoneEvent } from "@/types/rpc/AttachmentBatchDoneEvent";
import type { AttachmentFailedEvent } from "@/types/rpc/AttachmentFailedEvent";
import type { AttachmentInfo } from "@/types/rpc/AttachmentInfo";
import type { AttachmentProgressEvent } from "@/types/rpc/AttachmentProgressEvent";
import type { AttachmentReadyEvent } from "@/types/rpc/AttachmentReadyEvent";
import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import type { AttachmentRejected } from "@/types/rpc/AttachmentRejected";
import { attachmentNoun, fileKind } from "./file-kind";

/** Fallback limits until the backend reports its own (settings-driven) ones. */
export const DEFAULT_MAX_FILES = 128;
export const DEFAULT_MAX_TOTAL_BYTES = 1024 * 1024 * 1024;

export type DraftAttachmentStatus = "processing" | "ready" | "error";

export interface DraftAttachment {
    /** Stable UI key for this entry. */
    key: string;
    name: string;
    /** Size of the original, for the limits and the size summary. */
    bytes: number;
    status: DraftAttachmentStatus;
    /** `copying` / `uploading` report bytes; `processing` is a decode with no byte progress. */
    stage: "copying" | "uploading" | "processing";
    doneBytes: number;
    info?: AttachmentInfo;
    error?: string;
    /** Set for entries from `attachments.ingest`, to match its events. */
    batchId?: string;
    index?: number;
}

export interface RemovedAttachment {
    item: DraftAttachment;
    position: number;
}

let keySeq = 0;
const nextKey = () => `att-${Date.now().toString(36)}-${(keySeq++).toString(36)}`;

/**
 * Clipboard bitmaps arrive as "image.png"; give them a readable, sortable
 * name. Copied files keep theirs.
 */
export function pastedFileName(file: File, now = new Date()): string {
    if (file.name && file.name !== "image.png") return file.name;
    const pad = (n: number) => String(n).padStart(2, "0");
    const stamp = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}.${pad(now.getMinutes())}.${pad(now.getSeconds())}`;
    if (!file.name && !file.type.startsWith("image/")) return `Pasted file ${stamp}`;
    const ext = file.type === "image/jpeg" ? "jpg" : (file.type.split("/")[1] ?? "png");
    return `Pasted image ${stamp}.${ext}`;
}

/** POST one file to the upload route; resolves with its stored info. */
export async function uploadFile(file: File, name: string): Promise<AttachmentInfo> {
    const res = await fetch(`${getWebServerEndpoint()}/api/v1/attachments/upload?name=${encodeURIComponent(name)}`, {
        method: "POST",
        headers: authHeaders(),
        body: file,
    });
    const body = await res.json().catch(() => null);
    if (!res.ok || !body?.id) throw new Error(body?.error ?? `Upload failed (${res.status}).`);
    return body as AttachmentInfo;
}

/**
 * Put a file's bytes into a pane's working folder (`cmd:cwd`): stored like an
 * attachment, then copied there by srv. For files that arrive as bytes with
 * no host path (Ctrl+V, virtual or pathless drops). Resolves with the path.
 */
export async function uploadFileToWorkdir(blockId: string, file: File): Promise<string> {
    const name = pastedFileName(file);
    const info = await uploadFile(file, name);
    const res = await RpcApi.AttachmentsCopyToWorkdirCommand(TabRpcClient, { block_id: blockId, id: info.id, name });
    return res.path;
}

export class AttachmentDraft {
    readonly blockId: string;
    readonly items: Accessor<DraftAttachment[]>;
    private readonly setItems: (fn: (prev: DraftAttachment[]) => DraftAttachment[]) => void;
    /** Key of a tile to flash (a duplicate was added). */
    readonly flashKey: Accessor<string | null>;
    private readonly setFlashKey: (k: string | null) => void;
    readonly maxFiles: Accessor<number>;
    private readonly setMaxFiles: (n: number) => void;
    readonly maxTotalBytes: Accessor<number>;
    private readonly setMaxTotalBytes: (n: number) => void;

    private unsubscribe: (() => void) | null = null;
    private readonly openBatches = new Set<string>();
    /**
     * Events for a batch whose ingest reply hasn't arrived yet. The backend
     * starts processing before the RPC returns, so a small image can report
     * `ready` (even `batch-done`) before the tray has an entry for it.
     */
    private readonly early = new Map<string, Array<() => void>>();
    private readonly uploads = new Map<string, XMLHttpRequest>();
    private pendingUploads: { key: string; file: File; name: string; limit: number }[] = [];
    private readonly cappedInFlight = new Set<string>();

    constructor(blockId: string) {
        this.blockId = blockId;
        const [items, setItems] = createSignal<DraftAttachment[]>([]);
        this.items = items;
        this.setItems = (fn) => setItems((prev) => fn(prev));
        const [flashKey, setFlashKey] = createSignal<string | null>(null);
        this.flashKey = flashKey;
        this.setFlashKey = setFlashKey;
        const [maxFiles, setMaxFiles] = createSignal(DEFAULT_MAX_FILES);
        this.maxFiles = maxFiles;
        this.setMaxFiles = setMaxFiles;
        const [maxTotalBytes, setMaxTotalBytes] = createSignal(DEFAULT_MAX_TOTAL_BYTES);
        this.maxTotalBytes = maxTotalBytes;
        this.setMaxTotalBytes = setMaxTotalBytes;
    }

    // ── Derived state ─────────────────────────────────────────────────────

    count = () => this.items().length;
    totalBytes = () => this.items().reduce((sum, i) => sum + i.bytes, 0);
    processingCount = () => this.items().filter((i) => i.status === "processing").length;
    readyItems = () => this.items().filter((i) => i.status === "ready" && i.info);
    /**
     * The aggregate bar, over items still in flight, by bytes: a copy or
     * upload counts its bytes so far; one that reached the decode stage
     * counts in full (decoding has no byte progress).
     */
    progress = (): { done: number; total: number; count: number } => {
        const inFlight = this.items().filter((i) => i.status === "processing");
        const total = inFlight.reduce((s, i) => s + i.bytes, 0);
        const done = inFlight.reduce((s, i) => s + (i.stage === "processing" ? i.bytes : Math.min(i.doneBytes, i.bytes)), 0);
        return { done, total, count: inFlight.length };
    };

    /** What a send carries: ready attachments in tray order. */
    refs(): AttachmentRef[] {
        return this.readyItems().map((i) => ({ id: i.info!.id, name: i.name }));
    }

    // ── Adding ────────────────────────────────────────────────────────────

    /**
     * Hand dropped paths to the backend. Resolves with the paths it didn't
     * take (`non_images`): always none since any file can be attached, but a
     * backend from the images-only release still reports them.
     */
    async ingestPaths(paths: string[]): Promise<string[]> {
        if (paths.length === 0) return [];
        this.ensureSubscribed();
        const batchId = crypto.randomUUID();
        this.early.set(batchId, []);
        let res: Awaited<ReturnType<typeof RpcApi.AttachmentsIngestCommand>>;
        try {
            res = await RpcApi.AttachmentsIngestCommand(TabRpcClient, {
                block_id: this.blockId,
                batch_id: batchId,
                paths,
                dry_run: false,
                existing_count: this.count(),
                existing_bytes: this.totalBytes(),
            });
        } catch (err) {
            this.early.delete(batchId);
            throw err;
        }
        this.setMaxFiles(res.max_files);
        this.setMaxTotalBytes(res.max_total_bytes);
        if (res.accepted.length > 0) {
            this.openBatches.add(batchId);
            this.setItems((prev) => [
                ...prev,
                ...res.accepted.map((a) => ({
                    key: nextKey(),
                    name: a.name,
                    bytes: a.bytes,
                    status: "processing" as const,
                    stage: "copying" as const,
                    doneBytes: 0,
                    batchId,
                    index: a.index,
                })),
            ]);
        }
        const buffered = this.early.get(batchId) ?? [];
        this.early.delete(batchId);
        for (const replay of buffered) replay();
        this.reportRejected(res.rejected);
        return res.non_images;
    }

    /** Hold an event for a batch whose ingest reply is still in flight. */
    private deferIfEarly(batchId: string | undefined, replay: () => void): boolean {
        const queue = batchId ? this.early.get(batchId) : undefined;
        if (!queue) return false;
        queue.push(replay);
        return true;
    }

    /**
     * Stream Files to the upload route, one request each. With `concurrency`,
     * they queue behind every other capped upload and share one cap; without
     * it (paste) they start at once and don't count against that cap.
     */
    uploadFiles(files: File[], concurrency?: number): void {
        const limit = concurrency && concurrency > 0 ? concurrency : undefined;
        const rejected: AttachmentRejected[] = [];
        let count = this.count();
        let bytes = this.totalBytes();
        for (const file of files) {
            const name = pastedFileName(file);
            if (count >= this.maxFiles()) {
                rejected.push({ path: name, name, code: "too_many", reason: `A message can carry at most ${this.maxFiles()} attachments.` });
                continue;
            }
            if (bytes + file.size > this.maxTotalBytes()) {
                rejected.push({ path: name, name, code: "too_large", reason: `Adding it would pass the ${formatBytes(this.maxTotalBytes())} limit for one prompt.` });
                continue;
            }
            count += 1;
            bytes += file.size;
            const key = this.addUploadTile(file, name);
            if (limit === undefined) this.startUpload(key, file, name);
            else this.pendingUploads.push({ key, file, name, limit });
        }
        this.reportRejected(rejected);
        this.pumpUploads();
    }

    private addUploadTile(file: File, name: string): string {
        const key = nextKey();
        this.setItems((prev) => [
            ...prev,
            { key, name, bytes: file.size, status: "processing", stage: "uploading", doneBytes: 0 },
        ]);
        return key;
    }

    private pumpUploads(): void {
        while (this.pendingUploads.length > 0 && this.cappedInFlight.size < this.pendingUploads[0].limit) {
            const next = this.pendingUploads.shift()!;
            this.cappedInFlight.add(next.key);
            this.startUpload(next.key, next.file, next.name);
        }
    }

    private startUpload(key: string, file: File, name: string): void {
        const xhr = new XMLHttpRequest();
        const settle = () => {
            this.uploads.delete(key);
            this.cappedInFlight.delete(key);
        };
        this.uploads.set(key, xhr);
        xhr.open("POST", `${getWebServerEndpoint()}/api/v1/attachments/upload?name=${encodeURIComponent(name)}`);
        for (const [k, v] of Object.entries(authHeaders())) xhr.setRequestHeader(k, v);
        xhr.upload.onprogress = (e) => {
            if (!e.lengthComputable) return;
            this.patch(key, { doneBytes: e.loaded, stage: e.loaded >= e.total ? "processing" : "uploading" });
        };
        xhr.onload = () => {
            settle();
            let body: any = null;
            try {
                body = JSON.parse(xhr.responseText);
            } catch {
                /* handled below */
            }
            if (xhr.status === 200 && body?.id) {
                this.markReady(key, { ...(body as AttachmentInfo), name });
            } else {
                this.patch(key, { status: "error", error: body?.error ?? `Upload failed (${xhr.status}).` });
            }
            this.pumpUploads();
        };
        xhr.onerror = () => {
            settle();
            this.patch(key, { status: "error", error: "Upload failed: the connection to AgentMux dropped." });
            this.pumpUploads();
        };
        xhr.onabort = () => {
            settle();
            this.pumpUploads();
        };
        xhr.send(file);
    }

    // ── Changing ──────────────────────────────────────────────────────────

    remove(key: string): RemovedAttachment | null {
        const list = this.items();
        const position = list.findIndex((i) => i.key === key);
        if (position < 0) return null;
        const item = list[position];
        this.pendingUploads = this.pendingUploads.filter((p) => p.key !== key);
        this.uploads.get(key)?.abort();
        this.setItems((prev) => prev.filter((i) => i.key !== key));
        return { item, position };
    }

    /** Undo a removal. Only a finished (ready or failed) item comes back as it was. */
    restore(removed: RemovedAttachment[]): void {
        const back = removed.filter((r) => r.item.status !== "processing").sort((a, b) => a.position - b.position);
        if (back.length === 0) return;
        this.setItems((prev) => {
            const next = [...prev];
            for (const r of back) next.splice(Math.min(r.position, next.length), 0, r.item);
            return next;
        });
    }

    removeAll(): RemovedAttachment[] {
        const list = this.items();
        this.cancel();
        this.setItems(() => []);
        return list.map((item, position) => ({ item, position }));
    }

    /** Move an item by `delta` places (keyboard reorder). */
    move(key: string, delta: number): void {
        this.setItems((prev) => {
            const from = prev.findIndex((i) => i.key === key);
            const to = from + delta;
            if (from < 0 || to < 0 || to >= prev.length) return prev;
            const next = [...prev];
            const [it] = next.splice(from, 1);
            next.splice(to, 0, it);
            return next;
        });
    }

    /** Stop everything still copying, uploading or processing. */
    cancel(): void {
        for (const batchId of this.openBatches) {
            void RpcApi.AttachmentsCancelCommand(TabRpcClient, { batch_id: batchId }).catch(() => {});
        }
        this.pendingUploads = [];
        for (const xhr of this.uploads.values()) xhr.abort();
        this.uploads.clear();
        this.cappedInFlight.clear();
        this.setItems((prev) => prev.filter((i) => i.status !== "processing"));
    }

    /** Put a recalled (un-queued) message's attachments back in the tray. */
    async restoreRefs(refs: AttachmentRef[]): Promise<void> {
        if (refs.length === 0) return;
        const res = await RpcApi.AttachmentsInfoCommand(TabRpcClient, { ids: refs.map((r) => r.id) }).catch(() => null);
        const byId = new Map<string, AttachmentInfo>((res?.items ?? []).map((i) => [i.id, i] as const));
        this.setItems((prev) => [
            ...prev,
            ...refs.map((r): DraftAttachment => {
                const info = byId.get(r.id);
                return info
                    ? { key: nextKey(), name: r.name, bytes: info.bytes, status: "ready", stage: "processing", doneBytes: info.bytes, info: { ...info, name: r.name } }
                    : { key: nextKey(), name: r.name, bytes: 0, status: "error", stage: "processing", doneBytes: 0, error: "No longer available." };
            }),
        ]);
    }

    /** After a send: the tray empties; the files stay in the store for the transcript. */
    clearAfterSend(): void {
        this.setItems(() => []);
    }

    // ── Events ────────────────────────────────────────────────────────────

    private ensureSubscribed(): void {
        if (this.unsubscribe) return;
        const scope = MOS.makeORef("block", this.blockId);
        this.unsubscribe = muxEventSubscribe(
            {
                eventType: WpsEvent.AttachmentProgress,
                scope,
                handler: (ev) => this.onProgress((ev as any)?.data as AttachmentProgressEvent),
            },
            {
                eventType: WpsEvent.AttachmentReady,
                scope,
                handler: (ev) => this.onReady((ev as any)?.data as AttachmentReadyEvent),
            },
            {
                eventType: WpsEvent.AttachmentFailed,
                scope,
                handler: (ev) => this.onFailed((ev as any)?.data as AttachmentFailedEvent),
            },
            {
                eventType: WpsEvent.AttachmentBatchDone,
                scope,
                handler: (ev) => this.onBatchDone((ev as any)?.data as AttachmentBatchDoneEvent),
            },
        );
    }

    private findByEvent(batchId: string | undefined, index: number | undefined): DraftAttachment | undefined {
        if (!batchId || index == null) return undefined;
        return this.items().find((i) => i.batchId === batchId && i.index === index);
    }

    onProgress(e: AttachmentProgressEvent | undefined): void {
        if (this.deferIfEarly(e?.batch_id, () => this.onProgress(e))) return;
        const item = this.findByEvent(e?.batch_id, e?.index);
        if (!item || item.status !== "processing") return;
        this.patch(item.key, { doneBytes: e!.done_bytes, stage: e!.stage === "processing" ? "processing" : "copying" });
    }

    onReady(e: AttachmentReadyEvent | undefined): void {
        if (this.deferIfEarly(e?.batch_id, () => this.onReady(e))) return;
        const item = this.findByEvent(e?.batch_id, e?.index);
        if (!item) return;
        this.markReady(item.key, e!.info);
    }

    onFailed(e: AttachmentFailedEvent | undefined): void {
        if (this.deferIfEarly(e?.batch_id, () => this.onFailed(e))) return;
        const item = this.findByEvent(e?.batch_id, e?.index);
        if (!item) return;
        if (e!.code === "cancelled") {
            this.setItems((prev) => prev.filter((i) => i.key !== item.key));
            return;
        }
        this.patch(item.key, { status: "error", error: e!.error });
    }

    onBatchDone(e: AttachmentBatchDoneEvent | undefined): void {
        if (this.deferIfEarly(e?.batch_id, () => this.onBatchDone(e))) return;
        if (!e?.batch_id) return;
        this.openBatches.delete(e.batch_id);
        // Anything the batch never reported on did not finish.
        for (const i of this.items()) {
            if (i.batchId === e.batch_id && i.status === "processing") {
                this.patch(i.key, { status: "error", error: "Processing didn't finish." });
            }
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────

    private markReady(key: string, info: AttachmentInfo): void {
        // The same bytes twice: keep the first tile and flash it.
        const dup = this.items().find((i) => i.key !== key && i.status === "ready" && i.info?.id === info.id);
        if (dup) {
            this.setItems((prev) => prev.filter((i) => i.key !== key));
            this.setFlashKey(dup.key);
            setTimeout(() => {
                if (this.flashKey() === dup.key) this.setFlashKey(null);
            }, 900);
            return;
        }
        this.patch(key, { status: "ready", info, doneBytes: info.bytes, error: undefined });
    }

    private patch(key: string, fields: Partial<DraftAttachment>): void {
        this.setItems((prev) => prev.map((i) => (i.key === key ? { ...i, ...fields } : i)));
    }

    private reportRejected(rejected: AttachmentRejected[]): void {
        if (rejected.length === 0) return;
        const first = rejected[0];
        const title =
            rejected.length === 1
                ? `${first.name} wasn't attached`
                : `${rejected.length} ${attachmentNoun(rejected.length, rejected.map((r) => fileKind(undefined, r.name)))} weren't attached`;
        const lines = rejected.slice(0, 5).map((r) => (rejected.length === 1 ? r.reason : `${r.name}: ${r.reason}`));
        if (rejected.length > 5) lines.push(`…and ${rejected.length - 5} more.`);
        this.toast("warning", title, lines.join("\n"));
    }

    private toast(type: "info" | "warning", title: string, message: string): void {
        pushNotification({
            icon: type === "warning" ? "fa-triangle-exclamation" : "fa-info-circle",
            title,
            message,
            timestamp: new Date().toISOString(),
            type,
            expiration: Date.now() + 8000,
        });
    }
}

const drafts = new Map<string, AttachmentDraft>();

/** The attachment draft for an agent pane, created on first use. */
export function getAttachmentDraft(blockId: string): AttachmentDraft {
    let d = drafts.get(blockId);
    if (!d) {
        d = new AttachmentDraft(blockId);
        drafts.set(blockId, d);
    }
    return d;
}
