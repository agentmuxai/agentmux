// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Full-window preview of one attachment, with ←/→ to page, Esc to close, and
 * Remove in the composer. Shows the send-copy (≤ 2000 px), which is both what
 * the agent sees and small enough to load instantly, and states the
 * original's and the send-copy's size. Spec §5.1.
 */

import { Portal } from "solid-js/web";
import { Show, createEffect, createResource, onCleanup, onMount } from "solid-js";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { formatBytes } from "@/util/format-bytes";
import type { AttachmentInfo } from "@/types/rpc/AttachmentInfo";
import { useAttachmentUrl } from "./attachment-url";

export interface LightboxItem {
    key: string;
    number: number;
    name: string;
    id?: string;
    info?: AttachmentInfo;
    error?: string;
}

interface Props {
    items: LightboxItem[];
    index: number;
    onIndex: (i: number) => void;
    onClose: () => void;
    onRemove?: (key: string) => void;
}

export function AttachmentLightbox(props: Props) {
    let dialogRef: HTMLDivElement | undefined;
    let backdropRef: HTMLDivElement | undefined;
    // Native browser panes paint above the HTML renderer; register the
    // backdrop so they clip around it (as modal.tsx does).
    usePaneOverlay(() => backdropRef);
    const item = () => props.items[Math.min(props.index, props.items.length - 1)];
    const url = useAttachmentUrl(() => item()?.id, "send");
    // The transcript only knows ids and names; fetch the details on demand.
    const [fetched] = createResource(
        () => (item()?.info ? undefined : item()?.id),
        async (id) => (await RpcApi.AttachmentsInfoCommand(TabRpcClient, { ids: [id] })).items[0],
    );
    const info = () => item()?.info ?? fetched();

    const go = (delta: number) => {
        const n = props.items.length;
        if (n === 0) return;
        props.onIndex((props.index + delta + n) % n);
    };

    const onKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
            e.preventDefault();
            e.stopPropagation();
            props.onClose();
        } else if (e.key === "ArrowLeft") {
            e.preventDefault();
            go(-1);
        } else if (e.key === "ArrowRight") {
            e.preventDefault();
            go(1);
        }
    };

    onMount(() => dialogRef?.focus());
    createEffect(() => {
        if (props.items.length === 0) props.onClose();
    });
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    onCleanup(() => (document.body.style.overflow = prevOverflow));

    return (
        <Portal>
            <div ref={backdropRef} class="agent-attachment-lightbox" onClick={() => props.onClose()}>
                <div
                    ref={dialogRef}
                    class="agent-attachment-lightbox__dialog"
                    role="dialog"
                    aria-modal="true"
                    aria-label={`Image ${item()?.number}: ${item()?.name}`}
                    tabIndex={-1}
                    onKeyDown={onKeyDown}
                    onClick={(e) => e.stopPropagation()}
                >
                    <div class="agent-attachment-lightbox__image">
                        <Show
                            when={url()}
                            fallback={
                                <span class="agent-attachment-lightbox__missing">
                                    {item()?.error ??
                                        (url() === null ? "This image is no longer available." : "Loading…")}
                                </span>
                            }
                        >
                            <img src={url()!} alt={item()?.name ?? ""} />
                        </Show>
                    </div>
                    <div class="agent-attachment-lightbox__bar">
                        <span class="agent-attachment-lightbox__title" title={item()?.name}>
                            <span class="num">{item()?.number}</span>
                            {item()?.name}
                        </span>
                        <Show when={info()}>
                            {(i) => (
                                <span class="agent-attachment-lightbox__meta">
                                    {i().width}×{i().height} · {formatBytes(i().bytes)}
                                    <span class="sent">
                                        {" "}
                                        · sent as {i().send_width}×{i().send_height}, {formatBytes(i().send_bytes)}
                                        {i().first_frame_only ? " (first frame)" : ""}
                                    </span>
                                </span>
                            )}
                        </Show>
                        <span class="agent-attachment-lightbox__actions">
                            <Show when={props.items.length > 1}>
                                <button type="button" aria-label="Previous image" title="Previous (←)" onClick={() => go(-1)}>
                                    <i class="fa-solid fa-chevron-left" />
                                </button>
                                <span class="pos">
                                    {Math.min(props.index, props.items.length - 1) + 1} / {props.items.length}
                                </span>
                                <button type="button" aria-label="Next image" title="Next (→)" onClick={() => go(1)}>
                                    <i class="fa-solid fa-chevron-right" />
                                </button>
                            </Show>
                            <Show when={props.onRemove}>
                                <button type="button" class="remove" onClick={() => props.onRemove!(item()!.key)}>
                                    Remove
                                </button>
                            </Show>
                            <button type="button" aria-label="Close preview" title="Close (Esc)" onClick={() => props.onClose()}>
                                <i class="fa-solid fa-xmark" />
                            </button>
                        </span>
                    </div>
                </div>
            </div>
        </Portal>
    );
}
