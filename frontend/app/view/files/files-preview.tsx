// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's preview panel (spec §6.6): the selected file, read-only.
 * Code and text are highlighted, Markdown rendered, images shown; anything
 * else gets its facts and an Open button. One file at a time: a preview
 * starts after a short pause in the selection and is abandoned the moment
 * the selection moves on, and text reads only its first 256 KB.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.6.
 */

import { Markdown } from "@/app/element/markdown";
import {
    AUDIO_EXTENSIONS,
    fetchMediaBlob,
    fetchMediaRange,
    formatBytes,
    IMAGE_EXTENSIONS,
    INLINE_IMAGE_MAX_BYTES,
    VIDEO_EXTENSIONS,
} from "@/app/element/local-media";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { createEffect, createSignal, Match, on, onCleanup, Show, Switch, type JSX } from "solid-js";
import { detectLanguage } from "../agent/components/detectLanguage";
import { HighlightedCode } from "../agent/components/HighlightedCode";
import { extensionOf } from "./files-sort";
import { Button } from "@/app/element/ui";

/** How much of a text file the preview reads. */
export const PREVIEW_TEXT_BYTES = 256 * 1024;
/** Wait this long after the selection settles before reading anything. */
export const PREVIEW_DELAY_MS = 150;

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
export function looksBinary(bytes: Uint8Array): boolean {
    const n = Math.min(bytes.length, 8192);
    for (let i = 0; i < n; i++) if (bytes[i] === 0) return true;
    return false;
}

/** Reads what the preview shows for `entry` at `path`. */
export async function loadPreview(entry: FsEntry, path: string, signal: AbortSignal): Promise<Preview> {
    if (entry.is_dir) return { kind: "folder", entry };
    if (entry.error) return { kind: "other", entry, reason: entry.error };
    const ext = extensionOf(entry.name);
    if (IMAGE_EXTENSIONS.includes(ext)) {
        if ((entry.size ?? 0) > INLINE_IMAGE_MAX_BYTES) return { kind: "other", entry, reason: "Too large to preview." };
        const blob = await fetchMediaBlob(path, { maxBytes: INLINE_IMAGE_MAX_BYTES, signal, type: ext === "svg" ? "image/svg+xml" : undefined });
        return { kind: "image", entry, url: URL.createObjectURL(blob) };
    }
    if (VIDEO_EXTENSIONS.includes(ext) || AUDIO_EXTENSIONS.includes(ext)) return { kind: "av", entry };
    if (entry.size === 0) return { kind: "text", entry, text: "", truncated: false, markdown: MARKDOWN.has(ext) };
    const { blob, total } = await fetchMediaRange(path, 0, PREVIEW_TEXT_BYTES - 1, { signal });
    if (!blob) return { kind: "other", entry };
    const bytes = new Uint8Array(await blob.arrayBuffer());
    if (looksBinary(bytes)) return { kind: "binary", entry };
    // `fatal: false`: a multi-byte character cut at the 256 KB edge becomes
    // one replacement character instead of failing the whole preview.
    const text = new TextDecoder("utf-8", { fatal: false }).decode(bytes);
    return { kind: "text", entry, text, truncated: total > bytes.length, markdown: MARKDOWN.has(ext) };
}

export function FilesPreview(props: {
    /** The one selected entry, or null (nothing, or several, selected). */
    entry: FsEntry | null;
    pathOf: (name: string) => string;
    onOpen: (entry: FsEntry) => void;
    onOpenWithOs: (entry: FsEntry) => void;
}): JSX.Element {
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
                    loadPreview(entry, props.pathOf(entry.name), abort.signal).then(
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

    const facts = (e: FsEntry): string =>
        [e.size != null ? formatBytes(e.size) : null, e.mtime != null ? new Date(e.mtime).toLocaleString() : null]
            .filter(Boolean)
            .join(" · ");

    return (
        <aside class="files-preview" aria-label="Preview">
            <Switch>
                <Match when={preview().kind === "none"}>
                    <div class="files-preview-empty">Select a file to preview it.</div>
                </Match>
                <Match when={preview().kind === "loading"}>
                    <div class="files-preview-empty">Loading…</div>
                </Match>
                <Match when={preview().kind === "folder" && (preview() as Extract<Preview, { kind: "folder" }>)}>
                    {(p) => (
                        <div class="files-preview-card">
                            <i class="fa fa-folder files-preview-icon" aria-hidden="true" />
                            <div class="files-preview-name">{p().entry.name}</div>
                            <div class="files-preview-facts">{facts(p().entry)}</div>
                            <Button class="files-button" onClick={() => props.onOpen(p().entry)}>
                                Open folder
                            </Button>
                        </div>
                    )}
                </Match>
                <Match when={preview().kind === "image" && (preview() as Extract<Preview, { kind: "image" }>)}>
                    {(p) => (
                        <div class="files-preview-media">
                            <img src={p().url} alt={p().entry.name} onDblClick={() => props.onOpen(p().entry)} />
                            <div class="files-preview-facts">{facts(p().entry)}</div>
                        </div>
                    )}
                </Match>
                <Match when={preview().kind === "text" && (preview() as Extract<Preview, { kind: "text" }>)}>
                    {(p) => (
                        <div class="files-preview-text">
                            <Show
                                when={p().markdown}
                                fallback={
                                    <HighlightedCode
                                        code={p().text}
                                        lang={detectLanguage(p().entry.name, p().text.split("\n", 1)[0])}
                                        class="files-preview-code"
                                    />
                                }
                            >
                                <div class="files-preview-md">
                                    <Markdown text={p().text} scrollable={false} />
                                </div>
                            </Show>
                            <Show when={p().truncated}>
                                <div class="files-preview-more">
                                    Showing the first {formatBytes(PREVIEW_TEXT_BYTES)}.{" "}
                                    <button type="button" class="files-link" onClick={() => props.onOpen(p().entry)}>
                                        Open the whole file
                                    </button>
                                </div>
                            </Show>
                        </div>
                    )}
                </Match>
                <Match when={preview().kind === "av" && (preview() as Extract<Preview, { kind: "av" }>)}>
                    {(p) => (
                        <div class="files-preview-card">
                            <i class="fa fa-film files-preview-icon" aria-hidden="true" />
                            <div class="files-preview-name">{p().entry.name}</div>
                            <div class="files-preview-facts">{facts(p().entry)}</div>
                            <Button class="files-button" onClick={() => props.onOpen(p().entry)}>
                                Play in a Media pane
                            </Button>
                        </div>
                    )}
                </Match>
                <Match when={(preview().kind === "binary" || preview().kind === "other") && (preview() as Extract<Preview, { kind: "binary" | "other" }>)}>
                    {(p) => (
                        <div class="files-preview-card">
                            <i class="fa fa-file files-preview-icon" aria-hidden="true" />
                            <div class="files-preview-name">{p().entry.name}</div>
                            <div class="files-preview-facts">{facts(p().entry)}</div>
                            <div class="files-preview-facts">
                                {p().kind === "binary" ? "No preview: not a text file." : ((p() as { reason?: string }).reason ?? "No preview.")}
                            </div>
                            <Button class="files-button" onClick={() => props.onOpenWithOs(p().entry)}>
                                Open with default app
                            </Button>
                        </div>
                    )}
                </Match>
            </Switch>
        </aside>
    );
}
