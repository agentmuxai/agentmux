// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The attachment tray's state (AttachmentTray.tsx): the row of tiles and its "+N" overflow, expanding, the
 * Undo line and its timer, the announcements, the keys on a tile, and the lightbox's items. The pieces are made
 * in the order the tray always made them.
 */

import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { formatBytes } from "@/util/format-bytes";
import type { AttachmentDraft, DraftAttachment, RemovedAttachment } from "./attachment-draft";
import type { LightboxItem } from "./AttachmentLightbox";
import type { TileModel } from "./AttachmentTile";
import { attachmentNoun, fileKind, type FileKind } from "./file-kind";
import { limitLevel, trayLayout } from "./tray-layout";

const UNDO_MS = 6000;

export interface AttachmentTrayProps {
    draft: AttachmentDraft;
    /** Return focus to the text box (after the last tile is removed, or Esc). */
    focusComposer: () => void;
    /** Lets the composer move focus into the tray (Backspace at the start). */
    registerFocusLast?: (fn: () => void) => void;
}

function toTileModel(item: DraftAttachment, number: number): TileModel {
    const byteStage = item.stage !== "processing" && item.bytes > 0;
    return {
        key: item.key,
        number,
        name: item.name,
        bytes: item.bytes,
        id: item.info?.id,
        width: item.info?.width,
        height: item.info?.height,
        status: item.status,
        progress: byteStage ? Math.min(1, item.doneBytes / item.bytes) : undefined,
        error: item.error,
        firstFrameOnly: item.info?.first_frame_only,
        kind: fileKind(item.info?.kind, item.name),
        pageCount: item.info?.page_count,
        macros: item.info?.macros,
        textNote: item.info?.text_note,
    };
}

/** The tiles, how many fit in a row, and what the "+N" tile stands for; collapses again when the overflow goes away. */
function createTileRow(d: AttachmentDraft) {
    const [width, setWidth] = createSignal(600);
    const [expanded, setExpanded] = createSignal(false);
    // The list mounts and unmounts with the tray, so observe it from its ref.
    let ro: ResizeObserver | undefined;
    const observeList = (el: HTMLUListElement) => {
        if (typeof ResizeObserver === "undefined") return;
        ro?.disconnect();
        ro = new ResizeObserver((entries) => setWidth(entries[0]?.contentRect.width ?? 600));
        ro.observe(el);
    };
    onCleanup(() => ro?.disconnect());

    const tiles = createMemo(() => d.items().map((item, i) => toTileModel(item, i + 1)));
    const kinds = createMemo<FileKind[]>(() => tiles().map((t) => t.kind));
    const noun = (n: number) => attachmentNoun(n, kinds());
    const layout = createMemo(() => trayLayout(tiles().length, width()));
    const shown = createMemo(() => (expanded() ? tiles() : tiles().slice(0, layout().visible)));
    const overflowTiles = createMemo(() => tiles().slice(layout().visible, layout().visible + 4));
    // <For> keys on identity; tile models are rebuilt on every progress tick,
    // so iterate stable keys and look the model up, or every tile re-mounts.
    const tileByKey = createMemo(() => new Map(tiles().map((t) => [t.key, t])));
    const shownKeys = createMemo(() => shown().map((t) => t.key));

    // Collapse again when the overflow goes away.
    createEffect(() => {
        if (layout().overflow === 0 && expanded()) setExpanded(false);
    });
    return { expanded, setExpanded, observeList, tiles, noun, layout, shown, overflowTiles, tileByKey, shownKeys };
}

/** The Undo line after a removal: gone after a few seconds, which hovering or focusing the tray pauses. */
function createUndo() {
    const [undo, setUndo] = createSignal<{ removed: RemovedAttachment[]; label: string } | null>(null);
    let undoTimer: ReturnType<typeof setTimeout> | undefined;
    let undoPaused = false;
    onCleanup(() => clearTimeout(undoTimer));
    const startUndo = (removed: RemovedAttachment[], label: string) => {
        clearTimeout(undoTimer);
        setUndo({ removed, label });
        const arm = () => {
            undoTimer = setTimeout(() => {
                if (undoPaused) return arm();
                setUndo(null);
            }, UNDO_MS);
        };
        arm();
    };
    return {
        undo,
        startUndo,
        pause: () => (undoPaused = true),
        resume: () => (undoPaused = false),
        /** Undo was pressed: the line is done with. */
        dismiss: () => {
            setUndo(null);
            clearTimeout(undoTimer);
        },
    };
}

/** What a screen reader is told: one polite line per batch, not per progress tick, and each removal. */
function useBatchAnnouncement(d: AttachmentDraft, noun: (n: number) => string) {
    const [announcement, setAnnouncement] = createSignal("");
    createEffect(
        on(
            () => d.processingCount(),
            (now, before) => {
                if (before != null && before > 0 && now === 0) {
                    const failed = d.items().filter((i) => i.status === "error").length;
                    setAnnouncement(
                        `${d.count()} ${noun(d.count())} ready${failed ? `, ${failed} failed` : ""}.`,
                    );
                } else if ((before ?? 0) < now) {
                    setAnnouncement(`Adding ${now} ${noun(now)}.`);
                }
            },
        ),
    );
    return [announcement, setAnnouncement] as const;
}

/** The header's summary ("30 images · 214 MB / 1 GB") and how near the limits it is. */
function createTraySummary(d: AttachmentDraft, noun: (n: number) => string) {
    const countLevel = () => limitLevel(d.count(), d.maxFiles());
    const bytesLevel = () => limitLevel(d.totalBytes(), d.maxTotalBytes());
    const level = () =>
        countLevel() === "over" || bytesLevel() === "over"
            ? "over"
            : countLevel() === "warn" || bytesLevel() === "warn"
              ? "warn"
              : "ok";

    const summary = () => {
        const n = d.count();
        const countText = countLevel() === "ok" ? `${n} ${noun(n)}` : `${n} / ${d.maxFiles()} ${noun(2)}`;
        return `${countText} · ${formatBytes(d.totalBytes())} / ${formatBytes(d.maxTotalBytes())}`;
    };
    return { level, summary };
}

/** The attachment tray's state: everything its markup reads, and what its controls do. */
export function createAttachmentTray(props: AttachmentTrayProps) {
    const d = props.draft;
    const [lightboxIndex, setLightboxIndex] = createSignal<number | null>(null);
    const tileEls = new Map<string, HTMLLIElement>();
    const row = createTileRow(d);
    const undoState = createUndo();
    const { undo, startUndo } = undoState;
    const { tiles, noun, shown } = row;

    const [announcement, setAnnouncement] = useBatchAnnouncement(d, noun);
    const { level, summary } = createTraySummary(d, noun);

    const focusTile = (key: string | undefined) => {
        if (!key) {
            props.focusComposer();
            return;
        }
        tileEls.get(key)?.querySelector<HTMLButtonElement>(".agent-attachment-tile__main")?.focus();
    };

    const removeTile = (key: string, moveFocus: boolean) => {
        const list = tiles();
        const idx = list.findIndex((t) => t.key === key);
        const neighbour = list[idx + 1]?.key ?? list[idx - 1]?.key;
        const removed = d.remove(key);
        if (!removed) return;
        startUndo([removed], `Removed ${removed.item.name}`);
        setAnnouncement(`Removed ${removed.item.name}.`);
        if (moveFocus) queueMicrotask(() => focusTile(neighbour));
    };

    const clearAll = () => {
        const what = noun(d.count());
        const removed = d.removeAll();
        if (removed.length === 0) return;
        startUndo(removed, `Removed ${removed.length} ${what}`);
        setAnnouncement(`Removed ${removed.length} ${what}.`);
        props.focusComposer();
    };

    const onTileKeyDown = (key: string) => (e: KeyboardEvent) => {
        const list = shown();
        const idx = list.findIndex((t) => t.key === key);
        if (e.key === "Delete" || e.key === "Backspace") {
            e.preventDefault();
            removeTile(key, true);
        } else if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
            e.preventDefault();
            const delta = e.key === "ArrowLeft" ? -1 : 1;
            if (e.altKey) {
                d.move(key, delta);
                queueMicrotask(() => focusTile(key));
            } else {
                focusTile(list[idx + delta]?.key ?? (delta > 0 ? undefined : list[0]?.key));
            }
        } else if (e.key === "Escape") {
            e.preventDefault();
            props.focusComposer();
        }
    };

    props.registerFocusLast?.(() => {
        const last = shown()[shown().length - 1];
        if (last) focusTile(last.key);
    });

    const lightboxItems = (): LightboxItem[] =>
        d.items().map((i, n) => ({
            key: i.key,
            number: n + 1,
            name: i.name,
            id: i.info?.id,
            info: i.info,
            error: i.error,
            pending: i.status === "processing",
        }));

    /** Removed from inside the lightbox: it stays on the neighbour, and closes when nothing is left. */
    const removeFromLightbox = (key: string) => {
        removeTile(key, false);
        const n = d.count();
        if (n === 0) setLightboxIndex(null);
        else setLightboxIndex(Math.min(lightboxIndex()!, n - 1));
    };

    return {
        d, ...row, undo, undoState, tileEls,
        lightboxIndex, setLightboxIndex, announcement,
        level, summary, removeTile, clearAll, onTileKeyDown, lightboxItems, removeFromLightbox,
    };
}

/** What the attachment tray's markup reads. */
export type AttachmentTrayState = ReturnType<typeof createAttachmentTray>;
