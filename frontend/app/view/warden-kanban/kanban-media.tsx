// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The files a task carries: a strip of thumbnails on the card, and the Media section of the drawer
// (SPEC_KANBAN_TASK_MEDIA_2026_10_25.md §4).

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";

import { Button, Pill } from "@/app/element/ui";
import { fetchMediaBlob, INLINE_IMAGE_MAX_BYTES } from "@/app/element/local-media";
import { MarkdownImg, openMediaInEditor } from "@/app/element/markdown-media";
import { formatAge } from "@/app/view/warden-shared/warden-shared";

import type { KanbanActRequest, KanbanMediaItem, KanbanMediaPreview, KanbanTask } from "./kanban-types";

/** A task that has begun, so a missing picture on a UI task is worth a pill. */
const STARTED = new Set(["claimed", "blocked", "review", "done", "failed"]);

/** Newest attempt first, then newest file first (the drawer's order). */
export function mediaNewestFirst(items: KanbanMediaItem[]): KanbanMediaItem[] {
    return [...items].sort((a, b) => b.attempt - a.attempt || b.at - a.at || b.id - a.id);
}

/** The "+N" after the thumbnails the card shows: files it carries that it does not show. */
export function mediaOverflow(count: number, shown: number): number {
    return Math.max(0, count - shown);
}

/** What a UI task without media says on its card, or `null` when it needs no words. */
export function noMediaWords(task: KanbanTask): string | null {
    const m = task.card.media;
    if (!m || !m.ui_change || !STARTED.has(task.state)) return null;
    // srv sends the reason only while it is the current attempt's, so it shows even beside an earlier
    // attempt's pictures (a send-back handed in again without new ones).
    if (m.no_media_reason) return `No media: ${m.no_media_reason}`;
    return m.count > 0 ? null : "no media";
}

/** One thumbnail: an image's small copy (or the file itself for an SVG), a recording's tile with a play mark. */
function Thumb(props: { p: KanbanMediaPreview }): JSX.Element {
    const [url, setUrl] = createSignal<string>();
    const abort = new AbortController();
    let made: string | undefined;
    onMount(() => {
        if (props.p.kind !== "image") return;
        const path = props.p.thumb || props.p.path;
        if (!path) return;
        const type = props.p.thumb ? "image/png" : props.p.mime;
        fetchMediaBlob(path, { maxBytes: INLINE_IMAGE_MAX_BYTES, type, signal: abort.signal })
            .then((blob) => {
                if (abort.signal.aborted) return;
                made = URL.createObjectURL(blob);
                setUrl(made);
            })
            .catch(() => {});
    });
    onCleanup(() => {
        abort.abort();
        if (made) URL.revokeObjectURL(made);
    });
    return (
        <Button
            density="compact"
            tone="quiet"
            class={props.p.kind === "recording" ? "kanban-media__thumb kanban-media__thumb--recording" : "kanban-media__thumb"}
            title={props.p.kind === "recording" ? "Recording" : "Screenshot"}
            onClick={(e) => {
                e.stopPropagation();
                if (props.p.path) openMediaInEditor(props.p.path);
            }}
        >
            <Show when={url()} fallback={<i class={`fa-solid fa-${props.p.kind === "recording" ? "film" : "image"}`} aria-hidden="true" />}>
                {(u) => <img src={u()} alt="" draggable={false} />}
            </Show>
            <Show when={props.p.kind === "recording"}>
                <i class="fa-solid fa-play kanban-media__play" aria-hidden="true" />
            </Show>
        </Button>
    );
}

/** Under the card's title: up to three thumbnails and "+N", or a quiet pill on a UI task with none. */
export function CardMedia(props: { task: KanbanTask }): JSX.Element {
    const m = () => props.task.card.media;
    return (
        <>
            <Show when={(m()?.count ?? 0) > 0}>
                <div class="kanban-media__strip">
                    <For each={m()?.first ?? []}>{(p) => <Thumb p={p} />}</For>
                    <Show when={mediaOverflow(m()?.count ?? 0, m()?.first.length ?? 0) > 0}>
                        <span class="kanban-media__more">+{mediaOverflow(m()?.count ?? 0, m()?.first.length ?? 0)}</span>
                    </Show>
                </div>
            </Show>
            <Show when={noMediaWords(props.task)}>{(words) => <Pill icon="image">{words()}</Pill>}</Show>
        </>
    );
}

/** The drawer's Media section: every file, newest attempt first, and the operator's two moves. */
export function DrawerMedia(props: {
    task: KanbanTask;
    items: KanbanMediaItem[];
    now: number;
    busy: boolean;
    onAct: (req: KanbanActRequest) => Promise<unknown>;
}): JSX.Element {
    const ui = () => props.task.card.media?.ui_change ?? false;
    const shown = () => props.items.length > 0 || ui() || STARTED.has(props.task.state);
    return (
        <Show when={shown()}>
            <section class="kanban-drawer__section">
                <h4>Media</h4>
                <label class="kanban-media__flag">
                    <input
                        type="checkbox"
                        checked={ui()}
                        disabled={props.busy}
                        onChange={(e) => void props.onAct({ action: "ui_change", ui_change: e.currentTarget.checked })}
                    />
                    Changes the UI (handing it in for review needs screenshots or a recording)
                </label>
                <Show when={noMediaWords(props.task)}>{(words) => <p class="warden-manager-dim">{words()}</p>}</Show>
                <div class="kanban-media__grid">
                    <For each={mediaNewestFirst(props.items)}>
                        {(item) => (
                            <figure class="kanban-media__item">
                                <Show
                                    when={item.removed_at == null && item.path}
                                    fallback={
                                        <div class="kanban-media__removed">removed{item.removed_by ? ` by ${item.removed_by}` : ""}</div>
                                    }
                                >
                                    {/* No alt: the shared component would print it as a second caption; the name and
                                        caption are the figcaption's. */}
                                    <MarkdownImg props={{ src: item.path, alt: "" }} media={{ baseDir: "" }} />
                                </Show>
                                <figcaption>
                                    <div class="kanban-media__name">{item.name}</div>
                                    <Show when={item.caption}>
                                        <div>{item.caption}</div>
                                    </Show>
                                    <div class="warden-manager-dim">
                                        {item.attached_by || "an agent"}, attempt {item.attempt},{" "}
                                        {formatAge(Math.max(0, props.now - item.at))} ago
                                    </div>
                                    <Show when={item.removed_at == null}>
                                        <div class="kanban-drawer__row">
                                            <Button density="compact" tone="quiet" icon="up-right-from-square" onClick={() => openMediaInEditor(item.path)}>
                                                Open
                                            </Button>
                                            <Button
                                                density="compact"
                                                tone="quiet"
                                                icon="trash"
                                                disabled={props.busy}
                                                onClick={() => void props.onAct({ action: "media_remove", media_id: item.id })}
                                            >
                                                Remove
                                            </Button>
                                        </div>
                                    </Show>
                                </figcaption>
                            </figure>
                        )}
                    </For>
                </div>
            </section>
        </Show>
    );
}
