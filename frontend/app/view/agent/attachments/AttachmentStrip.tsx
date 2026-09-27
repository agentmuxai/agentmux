// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * A sent message's attachments, above its text in the transcript: the same
 * tiles as the composer, read-only, collapsed to one row with "+N". Spec §5.8.
 * Entries without an id are attachments the agent was told are no longer
 * available (swept by retention before the send). The transcript only keeps
 * names, so a tile's kind is guessed from the extension.
 */

import { For, Show, createMemo, createSignal, onCleanup } from "solid-js";
import { AttachmentLightbox, type LightboxItem } from "./AttachmentLightbox";
import { AttachmentTile, type TileModel } from "./AttachmentTile";
import { attachmentNoun, fileKind } from "./file-kind";
import { trayLayout } from "./tray-layout";

export interface StripAttachment {
    /** Absent when the image was already gone at send time. */
    id?: string;
    name: string;
}

export function AttachmentStrip(props: { attachments: StripAttachment[] }) {
    const [width, setWidth] = createSignal(600);
    const [expanded, setExpanded] = createSignal(false);
    const [open, setOpen] = createSignal<number | null>(null);
    let ro: ResizeObserver | undefined;
    onCleanup(() => ro?.disconnect());

    const tiles = createMemo<TileModel[]>(() =>
        props.attachments.map((a, i) => ({
            key: `${i}:${a.id ?? a.name}`,
            number: i + 1,
            name: a.name,
            id: a.id,
            status: a.id ? "ready" : "error",
            error: a.id ? undefined : "No longer available",
            kind: fileKind(undefined, a.name),
        })),
    );
    const noun = (n: number) =>
        attachmentNoun(
            n,
            tiles().map((t) => t.kind),
        );
    const layout = createMemo(() => trayLayout(tiles().length, width()));
    const shown = createMemo(() => (expanded() ? tiles() : tiles().slice(0, layout().visible)));
    const items = (): LightboxItem[] =>
        tiles().map((t) => ({ key: t.key, number: t.number, name: t.name, id: t.id, error: t.error }));

    return (
        <div class="agent-attachment-strip">
            <ul
                class="agent-attachment-tray__tiles"
                aria-label={`${tiles().length} attached ${noun(tiles().length)}`}
                ref={(el) => {
                    if (typeof ResizeObserver === "undefined") return;
                    ro?.disconnect();
                    ro = new ResizeObserver((e) => setWidth(e[0]?.contentRect.width ?? 600));
                    ro.observe(el);
                }}
            >
                <For each={shown()}>
                    {(tile) => <AttachmentTile tile={tile} onOpen={() => setOpen(tile.number - 1)} />}
                </For>
                <Show when={!expanded() && layout().overflow > 0}>
                    <li class="agent-attachment-tile agent-attachment-tile--more">
                        <button
                            type="button"
                            class="agent-attachment-tile__main"
                            aria-label={`Show all ${tiles().length} ${noun(tiles().length)}`}
                            onClick={() => setExpanded(true)}
                        >
                            <span class="more">+{layout().overflow}</span>
                        </button>
                    </li>
                </Show>
            </ul>
            <Show when={open() != null}>
                <AttachmentLightbox items={items()} index={open()!} onIndex={setOpen} onClose={() => setOpen(null)} />
            </Show>
        </div>
    );
}
