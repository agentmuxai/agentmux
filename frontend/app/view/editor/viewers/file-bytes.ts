// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A file's bytes as a blob object URL, for an `<img>`, `<video>` or
 * `<audio>`: fetched with the auth header (an element's `src` can't send it),
 * fetched again on a path change or a revision bump (an in-place overwrite),
 * a superseded fetch dropped, and the URL revoked on swap and unmount.
 * Moved from the retired Media pane, behaviour unchanged.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.13.
 */

import { fetchMediaBlob } from "@/app/element/local-media";
import { createEffect, createSignal, onCleanup, type Accessor } from "solid-js";

export interface FileBytesSource {
    /** The file on disk ("" for none). */
    path: string;
    /** Dropped bytes with no host path, shown instead of `path`. */
    file?: File;
    /** Bumped when the file changed in place: fetch it again. */
    revision: number;
    /** Force the blob's MIME type (SVG must be image/svg+xml to render). */
    type?: string;
}

export interface FileBytes {
    url: Accessor<string | null>;
    error: Accessor<string>;
    setError: (message: string) => void;
}

export function createFileBytes(source: Accessor<FileBytesSource>): FileBytes {
    const [url, setUrl] = createSignal<string | null>(null);
    const [error, setError] = createSignal("");
    let current: string | null = null;
    let token = 0;

    const swap = (next: string | null) => {
        const prev = current;
        current = next;
        setUrl(next);
        if (prev) URL.revokeObjectURL(prev);
    };

    createEffect(() => {
        const { path, file, revision, type } = source();
        void revision;
        const mine = ++token;
        if (file) {
            setError("");
            swap(URL.createObjectURL(type && file.type !== type ? new Blob([file], { type }) : file));
            return;
        }
        if (!path) {
            swap(null);
            return;
        }
        void (async () => {
            try {
                const blob = await fetchMediaBlob(path, type ? { type } : {});
                if (mine !== token) return; // superseded by a newer request
                swap(URL.createObjectURL(blob));
                setError("");
            } catch (e) {
                if (mine !== token) return;
                swap(null);
                setError(`Couldn't load the file: ${(e as Error)?.message ?? e}`);
            }
        })();
    });

    onCleanup(() => {
        token++;
        if (current) URL.revokeObjectURL(current);
        current = null;
    });

    return { url, error, setError };
}
