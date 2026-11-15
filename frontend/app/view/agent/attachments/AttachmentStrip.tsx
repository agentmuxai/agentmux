// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * A sent message's attachments, above its text in the transcript: the same
 * tiles as the composer, read-only, collapsed to one row with "+N". Spec §5.8.
 * Entries without an id are attachments the agent was told are no longer
 * available (swept by retention before the send). The transcript only keeps
 * names, so a tile's kind is guessed from the extension.
 */

import { For, Show, createEffect, createMemo, createSignal, onCleanup, untrack } from "solid-js";
import { Button } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AttachmentInfo } from "@/types/rpc/AttachmentInfo";
import { AttachmentLightbox, type LightboxItem } from "./AttachmentLightbox";
import { AttachmentTile, type TileModel } from "./AttachmentTile";
import { attachmentNoun, fileKind } from "./file-kind";
import { trayLayout } from "./tray-layout";

export interface StripAttachment {
    /** Absent when the image was already gone at send time. */
    id?: string;
    name: string;
    /** An image, which a text-only model cannot read; any other file reaches it as text or a path. */
    image?: boolean;
}

export function AttachmentStrip(props: { attachments: StripAttachment[] }) {
    const [width, setWidth] = createSignal(600);
    const [expanded, setExpanded] = createSignal(false);
    const [open, setOpen] = createSignal<number | null>(null);
    let ro: ResizeObserver | undefined;
    onCleanup(() => ro?.disconnect());

    // The transcript only keeps ids and names; the details (the real kind, a PDF's page count) are fetched once per id,
    // as the preview does. A signal, not a resource, so a load never suspends the pane's view (#47406).
    const [infos, setInfos] = createSignal<Record<string, AttachmentInfo>>({});
    const asked = new Set<string>();
    createEffect(() => {
        const ids = props.attachments.map((a) => a.id).filter((id): id is string => !!id && !untrack(() => asked.has(id)));
        if (!ids.length) return;
        for (const id of ids) asked.add(id);
        RpcApi.AttachmentsInfoCommand(TabRpcClient, { ids }).then(
            (r) => setInfos((prev) => ({ ...prev, ...Object.fromEntries(r.items.map((i) => [i.id, i])) })),
            () => ids.forEach((id) => asked.delete(id)),
        );
    });
    const tiles = createMemo<TileModel[]>(() =>
        props.attachments.map((a, i) => {
            const info = a.id ? infos()[a.id] : undefined;
            return {
                key: `${i}:${a.id ?? a.name}`,
                number: i + 1,
                name: a.name,
                id: a.id,
                status: a.id ? "ready" : "error",
                error: a.id ? undefined : "No longer available",
                kind: fileKind(info?.kind, a.name),
                pageCount: info?.page_count,
            };
        }),
    );
    const noun = (n: number) =>
        attachmentNoun(
            n,
            tiles().map((t) => t.kind),
        );
    // By value: a width change that fits the same number of tiles leaves `shown` and the "+N" tile alone.
    const layout = createMemo(() => trayLayout(tiles().length, width()), undefined, {
        equals: (a, b) => a.visible === b.visible && a.overflow === b.overflow,
    });
    const shown = createMemo(() => (expanded() ? tiles() : tiles().slice(0, layout().visible)));
    // <For> keys on identity and the models are rebuilt when the details arrive, so iterate the keys and look each
    // model up, or every tile would mount again (the tray does the same).
    const shownKeys = createMemo(() => shown().map((t) => t.key));
    const tileByKey = createMemo(() => new Map(tiles().map((t) => [t.key, t])));
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
                    // A strip in a hidden pane reports no width; laying it out for 0 px would drop its tiles and mount them
                    // again when the pane is shown, which a pane that flips between the two did every frame.
                    ro = new ResizeObserver((e) => {
                        const w = e[0]?.contentRect.width;
                        if (w) setWidth(w);
                    });
                    ro.observe(el);
                }}
            >
                <For each={shownKeys()}>
                    {(key) => (
                        <AttachmentTile
                            tile={tileByKey().get(key)!}
                            onOpen={() => setOpen(tileByKey().get(key)!.number - 1)}
                        />
                    )}
                </For>
                <Show when={!expanded() && layout().overflow > 0}>
                    <li class="agent-attachment-tile agent-attachment-tile--more">
                        <Button
                            class="agent-attachment-tile__main"
                            aria-label={`Show all ${tiles().length} ${noun(tiles().length)}`}
                            onClick={() => setExpanded(true)}
                        >
                            <span class="more">+{layout().overflow}</span>
                        </Button>
                    </li>
                </Show>
            </ul>
            <Show when={open() != null}>
                <AttachmentLightbox items={items()} index={open()!} onIndex={setOpen} onClose={() => setOpen(null)} />
            </Show>
        </div>
    );
}
