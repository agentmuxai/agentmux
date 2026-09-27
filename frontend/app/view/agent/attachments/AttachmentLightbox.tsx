// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Full-window preview of one attachment, with ←/→ to page, Esc to close, and
 * Remove in the composer. Images show the send-copy (≤ 2000 px), which is
 * both what the agent sees and small enough to load instantly, and state the
 * original's and the send-copy's size. Spec §5.1. SVGs show the file itself,
 * text files a scrolling view of their first lines, and everything else the
 * type icon with the details the agent is given
 * (SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §5).
 */

import { Portal } from "solid-js/web";
import { Match, Show, Switch, createEffect, createResource, onCleanup, onMount } from "solid-js";
import { usePaneOverlay } from "@/app/platform/pane-overlay";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { formatBytes } from "@/util/format-bytes";
import type { AttachmentInfo } from "@/types/rpc/AttachmentInfo";
import { useAttachmentText, useAttachmentUrl } from "./attachment-url";
import { extLabel, fileKind, isPicture, kindIcon, kindLabel } from "./file-kind";

export interface LightboxItem {
    key: string;
    number: number;
    name: string;
    id?: string;
    info?: AttachmentInfo;
    error?: string;
    /** Still copying or processing (composer only): no id yet, but not gone. */
    pending?: boolean;
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
    // The transcript only knows ids and names; fetch the details on demand.
    const [fetched] = createResource(
        () => (item()?.info ? undefined : item()?.id),
        async (id) => (await RpcApi.AttachmentsInfoCommand(TabRpcClient, { ids: [id] })).items[0],
    );
    const info = () => item()?.info ?? fetched();
    const kind = () => fileKind(info()?.kind, item()?.name ?? "");
    const url = useAttachmentUrl(() => (isPicture(kind()) ? item()?.id : undefined), "send");
    const preview = useAttachmentText(() => (kind() === "text" ? item()?.id : undefined), "thumb");
    // The preview drops a final newline; anything more means it was cut.
    const previewBytes = () => new TextEncoder().encode(preview() ?? "").length;
    const noun = () => (kind() === "image" ? "image" : "file");
    const missing = () => item()?.error ?? `This ${noun()} is no longer available.`;

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
                    aria-label={`${kind() === "image" ? "Image" : "File"} ${item()?.number}: ${item()?.name}`}
                    tabIndex={-1}
                    onKeyDown={onKeyDown}
                    onClick={(e) => e.stopPropagation()}
                >
                    <div class="agent-attachment-lightbox__image">
                        <Switch
                            fallback={
                                <FileDetails
                                    name={item()?.name ?? ""}
                                    kind={kind()}
                                    info={info()}
                                />
                            }
                        >
                            <Match when={item()?.error}>
                                <span class="agent-attachment-lightbox__missing">{missing()}</span>
                            </Match>
                            <Match when={item()?.pending}>
                                <span class="agent-attachment-lightbox__missing">Processing…</span>
                            </Match>
                            <Match when={!item()?.id}>
                                <span class="agent-attachment-lightbox__missing">{missing()}</span>
                            </Match>
                            <Match when={isPicture(kind())}>
                                <Show
                                    when={url()}
                                    fallback={
                                        <span class="agent-attachment-lightbox__missing">
                                            {url() === null ? missing() : "Loading…"}
                                        </span>
                                    }
                                >
                                    <img src={url()!} alt={item()?.name ?? ""} />
                                </Show>
                            </Match>
                            <Match when={kind() === "text"}>
                                <Show
                                    when={preview() != null}
                                    fallback={
                                        <span class="agent-attachment-lightbox__missing">
                                            {preview() === null ? missing() : "Loading…"}
                                        </span>
                                    }
                                >
                                    <div class="agent-attachment-lightbox__text">
                                        <pre>{preview()}</pre>
                                        <Show when={info() && info()!.bytes > previewBytes() + 1}>
                                            <span class="note">
                                                The first lines. The agent gets the whole file.
                                            </span>
                                        </Show>
                                    </div>
                                </Show>
                            </Match>
                        </Switch>
                    </div>
                    <div class="agent-attachment-lightbox__bar">
                        <span class="agent-attachment-lightbox__title" title={item()?.name}>
                            <span class="num">{item()?.number}</span>
                            {item()?.name}
                        </span>
                        <Show when={info()}>
                            {(i) => (
                                <span class="agent-attachment-lightbox__meta">
                                    <Show
                                        when={kind() === "image"}
                                        fallback={
                                            <>
                                                {kindLabel(kind())} · {formatBytes(i().bytes)}
                                            </>
                                        }
                                    >
                                        {i().width}×{i().height} · {formatBytes(i().bytes)}
                                        <span class="sent">
                                            {" "}
                                            · sent as {i().send_width}×{i().send_height}, {formatBytes(i().send_bytes)}
                                            {i().first_frame_only ? " (first frame)" : ""}
                                        </span>
                                    </Show>
                                </span>
                            )}
                        </Show>
                        <span class="agent-attachment-lightbox__actions">
                            <Show when={props.items.length > 1}>
                                <button type="button" aria-label={`Previous ${noun()}`} title="Previous (←)" onClick={() => go(-1)}>
                                    <i class="fa-solid fa-chevron-left" />
                                </button>
                                <span class="pos">
                                    {Math.min(props.index, props.items.length - 1) + 1} / {props.items.length}
                                </span>
                                <button type="button" aria-label={`Next ${noun()}`} title="Next (→)" onClick={() => go(1)}>
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

/** A file with no preview: its type icon and what the agent is told about it. */
function FileDetails(props: { name: string; kind: ReturnType<typeof fileKind>; info?: AttachmentInfo }) {
    const pages = () => props.info?.page_count;
    return (
        <div class={`agent-attachment-lightbox__details is-${props.kind}`}>
            <span class="icon" aria-hidden="true">
                <i class={`fa-solid ${kindIcon(props.kind, props.name)}`} />
                <Show when={extLabel(props.name)}>
                    <span class="ext">{extLabel(props.name)}</span>
                </Show>
            </span>
            <span class="name">{props.name}</span>
            <span class="facts">
                {kindLabel(props.kind)}
                <Show when={props.info}>{(i) => <> · {formatBytes(i().bytes)}</>}</Show>
                <Show when={pages()}>
                    {(n) => (
                        <>
                            {" "}
                            · {n()} {n() === 1 ? "page" : "pages"}
                        </>
                    )}
                </Show>
            </span>
            <Show when={props.info?.text_bytes}>
                {(n) => <span class="facts">Text version extracted ({formatBytes(n())}); the agent gets both.</span>}
            </Show>
            <Show when={props.info?.text_note}>{(note) => <span class="facts">{note()}</span>}</Show>
            <Show when={props.info?.macros}>
                <span class="warn">
                    <i class="fa-solid fa-triangle-exclamation" /> Contains macros. AgentMux never opens or runs them.
                </span>
            </Show>
        </div>
    );
}
