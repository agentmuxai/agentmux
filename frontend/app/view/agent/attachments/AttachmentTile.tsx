// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * One attachment tile: a 64 px thumbnail (images, SVG, text) or a colored
 * type icon (everything else) with its number badge, a remove button
 * (composer only), a progress ring while processing, and an error state.
 * Spec §5.1, §5.4, §5.7; SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §5.
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import { formatBytes } from "@/util/format-bytes";
import { useAttachmentText, useAttachmentUrl } from "./attachment-url";
import { extLabel, hasThumbnail, kindIcon, type FileKind } from "./file-kind";

export interface TileModel {
    key: string;
    /** 1-based; matches the number the agent sees. */
    number: number;
    name: string;
    bytes?: number;
    /** Set once processed. */
    id?: string;
    width?: number;
    height?: number;
    status: "processing" | "ready" | "error";
    /** 0..1 for copies and uploads; undefined while decoding. */
    progress?: number;
    error?: string;
    firstFrameOnly?: boolean;
    kind: FileKind;
    pageCount?: number;
    macros?: boolean;
    /** Why a document has no text version. */
    textNote?: string;
}

interface Props {
    tile: TileModel;
    /** Absent in the transcript, where tiles are read-only. */
    onRemove?: () => void;
    onOpen: () => void;
    onKeyDown?: JSX.EventHandler<HTMLLIElement, KeyboardEvent>;
    flash?: boolean;
    caption?: boolean;
    ref?: (el: HTMLLIElement) => void;
}

export function tileTooltip(t: TileModel): string {
    const parts = [t.name];
    if (t.width && t.height) parts.push(`${t.width}×${t.height}`);
    if (t.bytes != null) parts.push(formatBytes(t.bytes));
    if (t.pageCount) parts.push(`${t.pageCount} ${t.pageCount === 1 ? "page" : "pages"}`);
    if (t.firstFrameOnly) parts.push("GIF, first frame only");
    if (t.macros) parts.push("Contains macros");
    if (t.textNote) parts.push(t.textNote);
    if (t.status === "error" && t.error) parts.push(t.error);
    return parts.join(" · ");
}

const RING_R = 14;
const RING_C = 2 * Math.PI * RING_R;

/** The type icon with the extension under it. */
function FileIcon(props: { tile: TileModel }) {
    return (
        <span
            class={clsx(`agent-attachment-tile__icon is-${props.tile.kind}`, { "has-pages": !!props.tile.pageCount })}
            aria-hidden="true"
        >
            <i class={`fa-solid ${kindIcon(props.tile.kind, props.tile.name)}`} />
            <Show when={extLabel(props.tile.name)}>
                <span class="ext">{extLabel(props.tile.name)}</span>
            </Show>
        </span>
    );
}

/** A mini page with a text file's first lines. Rendered as text, never HTML. */
function TextThumb(props: { id: string; fallback: JSX.Element }) {
    const text = useAttachmentText(() => props.id, "thumb");
    return (
        <Show when={text()} fallback={props.fallback}>
            <span class="agent-attachment-tile__text" aria-hidden="true">
                {text()}
            </span>
        </Show>
    );
}

export function AttachmentTile(props: Props) {
    const ready = () => props.tile.status === "ready" && !!props.tile.id;
    const picture = () => ready() && (props.tile.kind === "image" || props.tile.kind === "svg");
    const url = useAttachmentUrl(() => (picture() ? props.tile.id : undefined), "thumb");
    const label = () => {
        const t = props.tile;
        const state =
            t.status === "processing" ? ", processing" : t.status === "error" ? `, failed: ${t.error ?? "error"}` : "";
        const noun = t.kind === "image" ? "Image" : "File";
        return `${noun} ${t.number}: ${t.name}${t.bytes != null ? `, ${formatBytes(t.bytes)}` : ""}${state}`;
    };
    const icon = () => (
        <Show
            when={props.tile.status !== "error"}
            fallback={
                <span class="agent-attachment-tile__placeholder" aria-hidden="true">
                    <i class="fa-solid fa-triangle-exclamation" />
                </span>
            }
        >
            <FileIcon tile={props.tile} />
        </Show>
    );

    return (
        <li
            ref={props.ref}
            class={clsx("agent-attachment-tile", {
                "is-processing": props.tile.status === "processing",
                "is-error": props.tile.status === "error",
                "is-flash": props.flash,
            })}
            onKeyDown={props.onKeyDown}
        >
            <button
                type="button"
                class="agent-attachment-tile__main"
                title={tileTooltip(props.tile)}
                aria-label={`${label()}. Open preview`}
                onClick={() => props.onOpen()}
            >
                <Show
                    when={ready() && props.tile.kind === "text"}
                    fallback={
                        <Show when={url()} fallback={icon()}>
                            <img src={url()!} alt="" draggable={false} />
                        </Show>
                    }
                >
                    <TextThumb id={props.tile.id!} fallback={icon()} />
                </Show>
                <Show when={props.tile.status === "processing"}>
                    <span class="agent-attachment-tile__progress" aria-hidden="true">
                        <svg viewBox="0 0 32 32" class={clsx({ "is-indeterminate": props.tile.progress == null })}>
                            <circle class="track" cx="16" cy="16" r={RING_R} />
                            <circle
                                class="bar"
                                cx="16"
                                cy="16"
                                r={RING_R}
                                stroke-dasharray={`${RING_C}`}
                                stroke-dashoffset={`${RING_C * (1 - (props.tile.progress ?? 0.25))}`}
                            />
                        </svg>
                    </span>
                </Show>
                <span class="agent-attachment-tile__badge" aria-hidden="true">
                    {props.tile.number}
                </span>
                <Show when={props.tile.pageCount && !hasThumbnail(props.tile.kind)}>
                    <span class="agent-attachment-tile__pages" aria-hidden="true">
                        {props.tile.pageCount} p
                    </span>
                </Show>
                <Show when={props.tile.macros}>
                    <span class="agent-attachment-tile__warn" aria-hidden="true">
                        <i class="fa-solid fa-triangle-exclamation" />
                    </span>
                </Show>
            </button>
            <Show when={props.onRemove}>
                <button
                    type="button"
                    class="agent-attachment-tile__remove"
                    aria-label={`Remove ${props.tile.name}`}
                    title="Remove"
                    tabIndex={-1}
                    onClick={(e) => {
                        e.stopPropagation();
                        props.onRemove!();
                    }}
                >
                    <i class="fa-solid fa-xmark" />
                </button>
            </Show>
            <Show when={props.caption}>
                <span class="agent-attachment-tile__caption" title={props.tile.name}>
                    <span class="name">{props.tile.name}</span>
                    <Show when={props.tile.bytes != null}>
                        <span class="size">{formatBytes(props.tile.bytes!)}</span>
                    </Show>
                </span>
            </Show>
        </li>
    );
}
