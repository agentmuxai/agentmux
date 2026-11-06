// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The attachment tray's markup in parts (AttachmentTray.tsx): the header (the summary, or the progress bar with
 * Cancel), the row of tiles with its "+N" tile, the Undo line and the lightbox. Each reads the tray's state
 * (attachment-tray-state.ts).
 */

import { Button } from "@/app/element/ui";
import { For, Show, onCleanup } from "solid-js";
import clsx from "clsx";
import { formatBytes } from "@/util/format-bytes";
import { AttachmentLightbox } from "./AttachmentLightbox";
import { AttachmentTile } from "./AttachmentTile";
import { useAttachmentUrl } from "./attachment-url";
import type { AttachmentTrayState } from "./attachment-tray-state";
import { isPicture, kindIcon, type FileKind } from "./file-kind";

type Part = { t: AttachmentTrayState };

/** The header: the summary, or the progress bar with Cancel while anything is processing, and Show less / Clear. */
export function TrayHeader(props: Part) {
    const { d, noun, level, summary, expanded, setExpanded, clearAll } = props.t;
    return (
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
                    <Button variant="link" onClick={() => d.cancel()}>
                        Cancel
                    </Button>
                </span>
            </Show>
            <span class="agent-attachment-tray__header-actions">
                <Show when={expanded()}>
                    <Button variant="link" onClick={() => setExpanded(false)}>
                        Show less
                    </Button>
                </Show>
                <Show when={d.count() > 1}>
                    <Button variant="link" onClick={clearAll}>
                        Clear
                    </Button>
                </Show>
            </span>
        </div>
    );
}

/** One cell of the "+N" tile's mosaic: the thumbnail of a picture, else an icon for its kind. */
function MosaicCell(props: { id?: string; kind: FileKind; name: string }) {
    const url = useAttachmentUrl(() => (isPicture(props.kind) ? props.id : undefined), "thumb");
    return (
        <span class={`cell is-${props.kind}`} style={url() ? { "background-image": `url(${url()})` } : undefined}>
            {isPicture(props.kind) ? null : <i class={`fa-solid ${kindIcon(props.kind, props.name)}`} />}
        </span>
    );
}

/** The "+N" tile: the next few attachments as a mosaic, and a button that shows them all. */
function MoreTile(props: Part) {
    const { d, noun, layout, overflowTiles, setExpanded } = props.t;
    return (
        <li class="agent-attachment-tile agent-attachment-tile--more">
            <Button
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
            </Button>
        </li>
    );
}

/** The tiles in one row (or all of them, expanded), then the "+N" tile. */
export function TileList(props: Part) {
    const t = props.t;
    const { d, noun, expanded, layout, shownKeys, tileByKey, tileEls, setLightboxIndex } = t;
    return (
        <ul
            ref={t.observeList}
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
                                onRemove={() => t.removeTile(key, false)}
                                onKeyDown={t.onTileKeyDown(key)}
                            />
                        </Show>
                    );
                }}
            </For>
            <Show when={!expanded() && layout().overflow > 0}>
                <MoreTile t={t} />
            </Show>
        </ul>
    );
}

/** "Removed …" with Undo, for a few seconds after a removal. */
export function UndoLine(props: Part) {
    const { d, undo, undoState } = props.t;
    return (
        <Show when={undo()}>
            {(u) => (
                <div class="agent-attachment-tray__undo">
                    <span>{u().label}</span>
                    <Button
                        variant="link"
                        onClick={() => {
                            d.restore(u().removed);
                            undoState.dismiss();
                        }}
                    >
                        Undo
                    </Button>
                </div>
            )}
        </Show>
    );
}

/** The lightbox over the tray's attachments, while one is open. */
export function TrayLightbox(props: Part) {
    const { d, lightboxIndex, setLightboxIndex, lightboxItems, removeFromLightbox } = props.t;
    return (
        <Show when={lightboxIndex() != null && d.count() > 0}>
            <AttachmentLightbox
                items={lightboxItems()}
                index={lightboxIndex()!}
                onIndex={setLightboxIndex}
                onClose={() => setLightboxIndex(null)}
                onRemove={removeFromLightbox}
            />
        </Show>
    );
}
