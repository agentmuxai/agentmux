// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The attachment preview's state: which item is shown, the details fetched for an item that only has an id, the
 * send-copy URL or text preview it shows, and the paging and keys.
 */

import { createEffect, createResource } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AttachmentInfo } from "@/types/rpc/AttachmentInfo";
import { useAttachmentText, useAttachmentUrl } from "./attachment-url";
import { fileKind, isPicture } from "./file-kind";

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

export interface LightboxProps {
    items: LightboxItem[];
    index: number;
    onIndex: (i: number) => void;
    onClose: () => void;
    onRemove?: (key: string) => void;
}

export type LightboxState = ReturnType<typeof createLightbox>;

export function createLightbox(props: LightboxProps) {
    let dialogRef: HTMLDivElement | undefined;
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

    createEffect(() => {
        if (props.items.length === 0) props.onClose();
    });

    return {
        item,
        info,
        kind,
        url,
        preview,
        previewBytes,
        noun,
        missing,
        go,
        onKeyDown,
        setDialog: (el: HTMLDivElement) => {
            dialogRef = el;
        },
        dialog: () => dialogRef,
    };
}
