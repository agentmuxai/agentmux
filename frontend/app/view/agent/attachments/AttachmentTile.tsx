// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * One image tile: a 64 px thumbnail with its number badge, a remove button
 * (composer only), a progress ring while processing, and an error state.
 * Spec §5.1, §5.4, §5.7.
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import { formatBytes } from "@/util/format-bytes";
import { useAttachmentUrl } from "./attachment-url";

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

function initials(name: string): string {
    const base = name.replace(/\.[^.]+$/, "").trim();
    return (base.slice(0, 2) || "?").toUpperCase();
}

export function tileTooltip(t: TileModel): string {
    const parts = [t.name];
    if (t.width && t.height) parts.push(`${t.width}×${t.height}`);
    if (t.bytes != null) parts.push(formatBytes(t.bytes));
    if (t.firstFrameOnly) parts.push("GIF, first frame only");
    if (t.status === "error" && t.error) parts.push(t.error);
    return parts.join(" · ");
}

const RING_R = 14;
const RING_C = 2 * Math.PI * RING_R;

export function AttachmentTile(props: Props) {
    const url = useAttachmentUrl(() => (props.tile.status === "ready" ? props.tile.id : undefined), "thumb");
    const label = () => {
        const t = props.tile;
        const state =
            t.status === "processing" ? ", processing" : t.status === "error" ? `, failed: ${t.error ?? "error"}` : "";
        return `Image ${t.number}: ${t.name}${t.bytes != null ? `, ${formatBytes(t.bytes)}` : ""}${state}`;
    };

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
                    when={url()}
                    fallback={
                        <span class="agent-attachment-tile__placeholder" aria-hidden="true">
                            <Show when={props.tile.status === "error"} fallback={initials(props.tile.name)}>
                                <i class="fa-solid fa-triangle-exclamation" />
                            </Show>
                        </span>
                    }
                >
                    <img src={url()!} alt="" draggable={false} />
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
