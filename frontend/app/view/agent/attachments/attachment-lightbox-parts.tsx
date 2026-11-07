// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/* The attachment preview's views, over a state from attachment-lightbox-state.ts: the picture, text or details, and the bar. */

import { Match, Show, Switch, type JSX } from "solid-js";
import { Button, IconButton } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { formatBytes } from "@/util/format-bytes";
import type { AttachmentInfo } from "@/types/rpc/AttachmentInfo";
import { extLabel, fileKind, isPicture, kindIcon, kindLabel } from "./file-kind";
import type { LightboxProps, LightboxState } from "./attachment-lightbox-state";

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

function PictureView(props: { s: LightboxState }): JSX.Element {
    const s = props.s;
    return (
        <Show
            when={s.url()}
            fallback={
                <span class="agent-attachment-lightbox__missing">{s.url() === null ? s.missing() : "Loading…"}</span>
            }
        >
            <img src={s.url()!} alt={s.item()?.name ?? ""} />
        </Show>
    );
}

function TextView(props: { s: LightboxState }): JSX.Element {
    const s = props.s;
    return (
        <Show
            when={s.preview() != null}
            fallback={
                <span class="agent-attachment-lightbox__missing">
                    {s.preview() === null ? s.missing() : "Loading…"}
                </span>
            }
        >
            <div class="agent-attachment-lightbox__text">
                <pre>{s.preview()}</pre>
                <Show when={s.info() && s.info()!.bytes > s.previewBytes() + 1}>
                    <span class="note">The first lines. The agent gets the whole file.</span>
                </Show>
            </div>
        </Show>
    );
}

/** What the preview area shows: a note, the picture, the text, or the file's details. */
export function LightboxBody(props: { s: LightboxState }): JSX.Element {
    const s = props.s;
    return (
        <Switch fallback={<FileDetails name={s.item()?.name ?? ""} kind={s.kind()} info={s.info()} />}>
            <Match when={s.item()?.error}>
                <span class="agent-attachment-lightbox__missing">{s.missing()}</span>
            </Match>
            <Match when={s.item()?.pending}>
                <span class="agent-attachment-lightbox__missing">Processing…</span>
            </Match>
            <Match when={!s.item()?.id}>
                <span class="agent-attachment-lightbox__missing">{s.missing()}</span>
            </Match>
            <Match when={isPicture(s.kind())}>
                <PictureView s={s} />
            </Match>
            <Match when={s.kind() === "text"}>
                <TextView s={s} />
            </Match>
        </Switch>
    );
}

function Meta(props: { s: LightboxState }): JSX.Element {
    const s = props.s;
    return (
        <Show when={s.info()}>
            {(i) => (
                <span class="agent-attachment-lightbox__meta">
                    <Show
                        when={s.kind() === "image"}
                        fallback={
                            <>
                                {kindLabel(s.kind())} · {formatBytes(i().bytes)}
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
    );
}

function Actions(props: { s: LightboxState; p: LightboxProps }): JSX.Element {
    const { s, p } = props;
    return (
        <span class="agent-attachment-lightbox__actions">
            <Show when={p.items.length > 1}>
                <IconButton
                    icon="chevron-left"
                    label={`Previous ${s.noun()}`}
                    tooltip={false}
                    title="Previous (←)"
                    onClick={() => s.go(-1)}
                />
                <span class="pos">
                    {Math.min(p.index, p.items.length - 1) + 1} / {p.items.length}
                </span>
                <IconButton
                    icon="chevron-right"
                    label={`Next ${s.noun()}`}
                    tooltip={false}
                    title="Next (→)"
                    onClick={() => s.go(1)}
                />
            </Show>
            <Show when={p.onRemove}>
                <Button tone="danger" onClick={() => p.onRemove!(s.item()!.key)}>
                    Remove
                </Button>
            </Show>
            <IconButton icon="xmark" label="Close preview" tooltip={false} title="Close (Esc)" onClick={() => p.onClose()} />
        </span>
    );
}

/** The bar under the preview: the number and name, the size line, and the paging and close buttons. */
export function LightboxBar(props: { s: LightboxState; p: LightboxProps }): JSX.Element {
    const s = props.s;
    return (
        <div class="agent-attachment-lightbox__bar">
            <span class="agent-attachment-lightbox__title" {...tip(s.item()?.name)}>
                <span class="num">{s.item()?.number}</span>
                {s.item()?.name}
            </span>
            <Meta s={s} />
            <Actions s={s} p={props.p} />
        </div>
    );
}
