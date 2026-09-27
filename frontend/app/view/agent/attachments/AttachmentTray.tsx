// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The composer's attachment tray, above the text box. Spec §5.1–§5.4, §5.9.
 *
 * Collapsed: one row of 64 px tiles; when they don't fit, the last slot is a
 * "+N" tile. Expanded: a scrolling grid with names and sizes. A summary
 * ("30 images · 214 MB / 1 GB", or "attachments" once any isn't an image)
 * sits in the header, with a progress bar and
 * Cancel while anything is processing. Removing shows an Undo line for a few
 * seconds, paused while hovered or focused.
 */

import clsx from "clsx";
import { For, Show, createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { formatBytes } from "@/util/format-bytes";
import type { AttachmentDraft, DraftAttachment, RemovedAttachment } from "./attachment-draft";
import { AttachmentLightbox, type LightboxItem } from "./AttachmentLightbox";
import { AttachmentTile, type TileModel } from "./AttachmentTile";
import { useAttachmentUrl } from "./attachment-url";
import { attachmentNoun, fileKind, isPicture, kindIcon, type FileKind } from "./file-kind";
import { limitLevel, trayLayout } from "./tray-layout";

const UNDO_MS = 6000;

export function toTileModel(item: DraftAttachment, number: number): TileModel {
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

interface Props {
    draft: AttachmentDraft;
    /** Return focus to the text box (after the last tile is removed, or Esc). */
    focusComposer: () => void;
    /** Lets the composer move focus into the tray (Backspace at the start). */
    registerFocusLast?: (fn: () => void) => void;
}

export function AttachmentTray(props: Props) {
    const d = props.draft;
    const [width, setWidth] = createSignal(600);
    const [expanded, setExpanded] = createSignal(false);
    const [lightboxIndex, setLightboxIndex] = createSignal<number | null>(null);
    const [undo, setUndo] = createSignal<{ removed: RemovedAttachment[]; label: string } | null>(null);
    const [announcement, setAnnouncement] = createSignal("");
    const tileEls = new Map<string, HTMLLIElement>();
    let undoTimer: ReturnType<typeof setTimeout> | undefined;
    let undoPaused = false;
    // The list mounts and unmounts with the tray, so observe it from its ref.
    let ro: ResizeObserver | undefined;
    const observeList = (el: HTMLUListElement) => {
        if (typeof ResizeObserver === "undefined") return;
        ro?.disconnect();
        ro = new ResizeObserver((entries) => setWidth(entries[0]?.contentRect.width ?? 600));
        ro.observe(el);
    };
    onCleanup(() => {
        ro?.disconnect();
        clearTimeout(undoTimer);
    });

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

    // One polite announcement per batch, not per progress tick.
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

    const focusTile = (key: string | undefined) => {
        if (!key) {
            props.focusComposer();
            return;
        }
        tileEls.get(key)?.querySelector<HTMLButtonElement>(".agent-attachment-tile__main")?.focus();
    };

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
        d.items().map((i, n) => ({ key: i.key, number: n + 1, name: i.name, id: i.info?.id, info: i.info, error: i.error }));

    return (
        <>
            <Show when={d.count() > 0 || undo()}>
                <div
                    class={clsx("agent-attachment-tray", { "is-expanded": expanded() })}
                    onMouseEnter={() => (undoPaused = true)}
                    onMouseLeave={() => (undoPaused = false)}
                    onFocusIn={() => (undoPaused = true)}
                    onFocusOut={() => (undoPaused = false)}
                >
                    <Show when={d.count() > 0}>
                        <div class="agent-attachment-tray__header">
                            <Show
                                when={d.processingCount() > 0}
                                fallback={
                                    <span class={clsx("agent-attachment-tray__summary", `is-${level()}`)}>
                                        {summary()}
                                    </span>
                                }
                            >
                                <span class="agent-attachment-tray__processing">
                                    <span class="text">
                                        Processing {d.progress().count} of {d.count()} ·{" "}
                                        {formatBytes(d.progress().done)} of {formatBytes(d.progress().total)}
                                    </span>
                                    <span
                                        class="agent-attachment-tray__bar"
                                        role="progressbar"
                                        aria-label={`Processing ${noun(2)}`}
                                        aria-valuemin={0}
                                        aria-valuemax={100}
                                        aria-valuenow={Math.round(
                                            (100 * d.progress().done) / Math.max(1, d.progress().total),
                                        )}
                                    >
                                        <span
                                            class="fill"
                                            style={{
                                                width: `${(100 * d.progress().done) / Math.max(1, d.progress().total)}%`,
                                            }}
                                        />
                                    </span>
                                    <button type="button" class="link" onClick={() => d.cancel()}>
                                        Cancel
                                    </button>
                                </span>
                            </Show>
                            <span class="agent-attachment-tray__header-actions">
                                <Show when={expanded()}>
                                    <button type="button" class="link" onClick={() => setExpanded(false)}>
                                        Show less
                                    </button>
                                </Show>
                                <Show when={d.count() > 1}>
                                    <button type="button" class="link" onClick={clearAll}>
                                        Clear
                                    </button>
                                </Show>
                            </span>
                        </div>
                    </Show>
                    <ul
                        ref={observeList}
                        class="agent-attachment-tray__tiles"
                        aria-label={`Attached ${noun(2)}: ${d.count()}`}
                    >
                        <For each={shownKeys()}>
                            {(key) => {
                                onCleanup(() => tileEls.delete(key));
                                const tile = () => tileByKey().get(key);
                                return (
                                    <Show when={tile()}>
                                        <AttachmentTile
                                            ref={(el) => tileEls.set(key, el)}
                                            tile={tile()!}
                                            flash={d.flashKey() === key}
                                            caption={expanded()}
                                            onOpen={() => setLightboxIndex(tile()!.number - 1)}
                                            onRemove={() => removeTile(key, false)}
                                            onKeyDown={onTileKeyDown(key)}
                                        />
                                    </Show>
                                );
                            }}
                        </For>
                        <Show when={!expanded() && layout().overflow > 0}>
                            <li class="agent-attachment-tile agent-attachment-tile--more">
                                <button
                                    type="button"
                                    class="agent-attachment-tile__main"
                                    aria-label={`Show all ${d.count()} ${noun(d.count())}`}
                                    title={`Show all ${d.count()} ${noun(d.count())}`}
                                    onClick={() => setExpanded(true)}
                                >
                                    <span class="mosaic" aria-hidden="true">
                                        <For each={overflowTiles()}>
                                            {(t) => (
                                                <MosaicCell
                                                    id={t.status === "ready" ? t.id : undefined}
                                                    kind={t.kind}
                                                    name={t.name}
                                                />
                                            )}
                                        </For>
                                    </span>
                                    <span class="more">+{layout().overflow}</span>
                                </button>
                            </li>
                        </Show>
                    </ul>
                    <Show when={undo()}>
                        {(u) => (
                            <div class="agent-attachment-tray__undo">
                                <span>{u().label}</span>
                                <button
                                    type="button"
                                    class="link"
                                    onClick={() => {
                                        d.restore(u().removed);
                                        setUndo(null);
                                        clearTimeout(undoTimer);
                                    }}
                                >
                                    Undo
                                </button>
                            </div>
                        )}
                    </Show>
                </div>
            </Show>
            <span class="sr-only" aria-live="polite">
                {announcement()}
            </span>
            <Show when={lightboxIndex() != null && d.count() > 0}>
                <AttachmentLightbox
                    items={lightboxItems()}
                    index={lightboxIndex()!}
                    onIndex={setLightboxIndex}
                    onClose={() => setLightboxIndex(null)}
                    onRemove={(key) => {
                        removeTile(key, false);
                        const n = d.count();
                        if (n === 0) setLightboxIndex(null);
                        else setLightboxIndex(Math.min(lightboxIndex()!, n - 1));
                    }}
                />
            </Show>
        </>
    );
}

function MosaicCell(props: { id?: string; kind: FileKind; name: string }) {
    const url = useAttachmentUrl(() => (isPicture(props.kind) ? props.id : undefined), "thumb");
    return (
        <span class={`cell is-${props.kind}`} style={url() ? { "background-image": `url(${url()})` } : undefined}>
            {isPicture(props.kind) ? null : <i class={`fa-solid ${kindIcon(props.kind, props.name)}`} />}
        </span>
    );
}
