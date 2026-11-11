// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's preview panel's state (files-preview.tsx): what is read for the selected entry and when. A
 * preview starts after a short pause in the selection and is abandoned the moment the selection moves on; text reads
 * only its first 256 KB. docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.6.
 */

import {
    AUDIO_EXTENSIONS,
    fetchMediaBlob,
    fetchMediaRange,
    IMAGE_EXTENSIONS,
    INLINE_IMAGE_MAX_BYTES,
    VIDEO_EXTENSIONS,
} from "@/app/element/local-media";
import { formatBytes } from "@/util/format-bytes";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { createEffect, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { extensionOf } from "./files-sort";

/** How much of a text file the preview reads. */
export const PREVIEW_TEXT_BYTES = 256 * 1024;
/** Wait this long after the selection settles before reading anything. */
const PREVIEW_DELAY_MS = 150;

export type Preview =
    | { kind: "none" }
    | { kind: "loading" }
    | { kind: "folder"; entry: FsEntry }
    | { kind: "image"; entry: FsEntry; url: string }
    | { kind: "text"; entry: FsEntry; text: string; truncated: boolean; markdown: boolean }
    | { kind: "binary"; entry: FsEntry }
    /** Video or audio: played in a Media pane, not read here. */
    | { kind: "av"; entry: FsEntry }
    | { kind: "other"; entry: FsEntry; reason?: string };

const MARKDOWN = new Set(["md", "markdown", "mdx"]);

/** A NUL in the first 8 KB: not text worth showing. */
function looksBinary(bytes: Uint8Array): boolean {
    const n = Math.min(bytes.length, 8192);
    for (let i = 0; i < n; i++) if (bytes[i] === 0) return true;
    return false;
}

/** Reads what the preview shows for `entry` at `path`. */
async function loadPreview(entry: FsEntry, path: string, signal: AbortSignal, blockId: string): Promise<Preview> {
    if (entry.is_dir) return { kind: "folder", entry };
    if (entry.error) return { kind: "other", entry, reason: entry.error };
    const ext = extensionOf(entry.name);
    if (IMAGE_EXTENSIONS.includes(ext)) {
        if ((entry.size ?? 0) > INLINE_IMAGE_MAX_BYTES) return { kind: "other", entry, reason: "Too large to preview." };
        const blob = await fetchMediaBlob(path, { maxBytes: INLINE_IMAGE_MAX_BYTES, signal, blockId, type: ext === "svg" ? "image/svg+xml" : undefined });
        return { kind: "image", entry, url: URL.createObjectURL(blob) };
    }
    if (VIDEO_EXTENSIONS.includes(ext) || AUDIO_EXTENSIONS.includes(ext)) return { kind: "av", entry };
    if (entry.size === 0) return { kind: "text", entry, text: "", truncated: false, markdown: MARKDOWN.has(ext) };
    const { blob, total } = await fetchMediaRange(path, 0, PREVIEW_TEXT_BYTES - 1, { signal, blockId });
    if (!blob) return { kind: "other", entry };
    const bytes = new Uint8Array(await blob.arrayBuffer());
    if (looksBinary(bytes)) return { kind: "binary", entry };
    // `fatal: false`: a multi-byte character cut at the 256 KB edge becomes
    // one replacement character instead of failing the whole preview.
    const text = new TextDecoder("utf-8", { fatal: false }).decode(bytes);
    return { kind: "text", entry, text, truncated: total > bytes.length, markdown: MARKDOWN.has(ext) };
}

/** What the preview panel is given. */
export interface FilesPreviewProps {
    /** The one selected entry, or null (nothing, or several, selected). */
    entry: FsEntry | null;
    pathOf: (name: string) => string;
    /** The Files pane the preview is in, named on every read. */
    blockId: string;
    onOpen: (entry: FsEntry) => void;
    onOpenWithOs: (entry: FsEntry) => void;
}

/** An entry's size and modified time, as one line. */
export const previewFacts = (e: FsEntry): string =>
    [e.size != null ? formatBytes(e.size) : null, e.mtime != null ? new Date(e.mtime).toLocaleString() : null]
        .filter(Boolean)
        .join(" · ");

/** The preview of the selected entry: it follows the selection, reads after a pause and drops what was left behind. */
export function createFilesPreview(props: FilesPreviewProps): Accessor<Preview> {
    const [preview, setPreview] = createSignal<Preview>({ kind: "none" });
    let objectUrl: string | null = null;
    const releaseUrl = () => {
        if (objectUrl) URL.revokeObjectURL(objectUrl);
        objectUrl = null;
    };
    onCleanup(releaseUrl);

    createEffect(
        on(
            () => {
                const e = props.entry;
                // Re-read when the file changes on disk, not only when the
                // selection does.
                return e ? `${props.pathOf(e.name)}\u0000${e.mtime ?? ""}\u0000${e.size ?? ""}` : null;
            },
            (key) => {
                const entry = props.entry;
                if (!key || !entry) {
                    releaseUrl();
                    setPreview({ kind: "none" });
                    return;
                }
                const abort = new AbortController();
                const timer = setTimeout(() => {
                    setPreview({ kind: "loading" });
                    loadPreview(entry, props.pathOf(entry.name), abort.signal, props.blockId).then(
                        (p) => {
                            if (abort.signal.aborted) {
                                if (p.kind === "image") URL.revokeObjectURL(p.url);
                                return;
                            }
                            releaseUrl();
                            if (p.kind === "image") objectUrl = p.url;
                            setPreview(p);
                        },
                        (err) => {
                            if (abort.signal.aborted) return;
                            setPreview({ kind: "other", entry, reason: err instanceof Error ? err.message : String(err) });
                        }
                    );
                }, PREVIEW_DELAY_MS);
                onCleanup(() => {
                    clearTimeout(timer);
                    abort.abort();
                });
            }
        )
    );
    return preview;
}
