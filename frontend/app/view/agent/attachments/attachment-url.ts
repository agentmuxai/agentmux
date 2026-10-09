// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Object URLs for stored attachment files. The route needs `X-AuthKey`, which
 * an <img src> can't send, so each file is fetched once into a Blob and
 * shared by every tile showing it (the Media pane's pattern). URLs are
 * reference-counted and revoked a little after the last user lets go, so a
 * virtualized transcript scrolling a row out and back doesn't refetch.
 */

import { createEffect, createResource, createSignal, onCleanup, type Accessor } from "solid-js";
import { authHeaders } from "@/app/store/auth-headers";
import { getWebServerEndpoint } from "@/util/endpoints";

/** `text` is the extracted text version of a document. */
export type AttachmentFileKind = "thumb" | "send" | "original" | "text";

interface Entry {
    refs: number;
    /** undefined while loading, null when the file is gone. */
    url: Accessor<string | null | undefined>;
    setUrl: (u: string | null | undefined) => void;
    revokeTimer?: ReturnType<typeof setTimeout>;
}

const REVOKE_AFTER_MS = 30_000;
const entries = new Map<string, Entry>();

function load(id: string, kind: AttachmentFileKind, entry: Entry): void {
    fetch(`${getWebServerEndpoint()}/api/v1/attachments/${id}/${kind}`, {
        headers: authHeaders(),
    })
        .then((r) => (r.ok ? r.blob() : null))
        .then((blob) => entry.setUrl(blob ? URL.createObjectURL(blob) : null))
        .catch(() => entry.setUrl(null));
}

function acquire(id: string, kind: AttachmentFileKind): Entry {
    const key = `${id}:${kind}`;
    let entry = entries.get(key);
    if (!entry) {
        const [url, setUrl] = createSignal<string | null | undefined>(undefined);
        entry = { refs: 0, url, setUrl };
        entries.set(key, entry);
        load(id, kind, entry);
    }
    if (entry.revokeTimer) {
        clearTimeout(entry.revokeTimer);
        entry.revokeTimer = undefined;
    }
    entry.refs += 1;
    return entry;
}

function release(id: string, kind: AttachmentFileKind): void {
    const key = `${id}:${kind}`;
    const entry = entries.get(key);
    if (!entry) return;
    entry.refs -= 1;
    if (entry.refs > 0) return;
    entry.revokeTimer = setTimeout(() => {
        if (entry.refs > 0) return;
        const u = entry.url();
        if (u) URL.revokeObjectURL(u);
        entries.delete(key);
    }, REVOKE_AFTER_MS);
}

/**
 * An object URL for attachment `id` (undefined while loading, null when it
 * no longer exists). Follows `id` if it changes; releases on cleanup.
 */
export function useAttachmentUrl(
    id: Accessor<string | undefined>,
    kind: AttachmentFileKind,
): Accessor<string | null | undefined> {
    const [current, setCurrent] = createSignal<Entry | null>(null);
    createEffect(() => {
        const value = id();
        if (!value) {
            setCurrent(null);
            return;
        }
        const entry = acquire(value, kind);
        setCurrent(entry);
        onCleanup(() => release(value, kind));
    });
    return () => current()?.url();
}

/**
 * The text of a stored text file (a text preview, or a document's text
 * version): undefined while loading, null when it's gone. Read from the
 * shared object URL, so the tile and the preview fetch it once.
 */
export function useAttachmentText(
    id: Accessor<string | undefined>,
    kind: AttachmentFileKind,
): Accessor<string | null | undefined> {
    const url = useAttachmentUrl(id, kind);
    const [text] = createResource(
        () => url() ?? undefined,
        async (u) => {
            try {
                return await (await fetch(u)).text();
            } catch {
                return null;
            }
        },
    );
    return () => (url() === null ? null : text());
}
