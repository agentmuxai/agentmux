// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The icon of a Files pane entry: the system's own for its file type once srv has
 * made it (setting `files:icons`), AgentMux's built-in glyph until then and
 * whenever there is none. Both sit in the box their parent gives, so the swap
 * moves nothing. docs/specs/SPEC_FILES_SYSTEM_FILE_ICONS_2026_10_27.md §4.5.
 */
import type { FsEntry } from "@/types/rpc/FsEntry";
import { createEffect, createSignal, on, onCleanup, Show, type JSX } from "solid-js";
import { cachedIcon, iconTarget, snapPx, systemIcon } from "./files-icons";
import type { FilesModel } from "./files-model";

export function iconOf(e: FsEntry): string {
    if (e.is_dir) return "folder";
    const n = e.name.toLowerCase();
    if (/\.(ts|tsx|js|jsx|mjs|cjs|py|rs|go|java|kt|swift|c|cpp|h|hpp|cs|rb|php|lua|sh|bash|zsh|ps1)$/.test(n)) return "file-code";
    if (/\.(html|htm|css|scss|sass|less|svg|xml|json|toml|yaml|yml|ini|env|conf)$/.test(n)) return "file-code";
    if (/\.(md|markdown|txt|rst|adoc|log)$/.test(n)) return "file-lines";
    if (/\.(png|jpe?g|gif|webp|bmp|ico|heic|tiff?)$/.test(n)) return "file-image";
    if (/\.(mp4|webm|mov|mkv|avi)$/.test(n)) return "file-video";
    if (/\.(wav|mp3|flac|ogg|m4a)$/.test(n)) return "file-audio";
    if (/\.pdf$/.test(n)) return "file-pdf";
    if (/\.(zip|tar|gz|7z|rar|bz2|xz)$/.test(n)) return "file-zipper";
    return "file";
}

export function TypeIcon(props: { model: FilesModel; entry: FsEntry; cssPx: number; glyphClass: string }): JSX.Element {
    const [src, setSrc] = createSignal<string | undefined>();
    createEffect(
        on(
            () => {
                const t = iconTarget(props.entry);
                return [props.model.systemIcons(), t.kind, t.ext, snapPx(props.cssPx, props.model.iconScale())] as const;
            },
            ([enabled, kind, ext, px]) => {
                if (!enabled) {
                    setSrc(undefined);
                    return;
                }
                const target = { kind, ext };
                const hit = cachedIcon(target, px);
                setSrc(hit);
                if (hit) return;
                // Scrolled past before it was made: stop asking for it.
                const gone = new AbortController();
                onCleanup(() => gone.abort());
                void systemIcon(target, px, gone.signal).then((url) => {
                    if (!gone.signal.aborted && url) setSrc(url);
                });
            }
        )
    );
    return (
        <Show
            when={src()}
            fallback={
                <i
                    class={`fa fa-${iconOf(props.entry)} ${props.glyphClass}`}
                    classList={{ "files-icon-dir": props.entry.is_dir }}
                    aria-hidden="true"
                />
            }
        >
            {(url) => (
                <img class="files-sys-icon" src={url()} width={props.cssPx} height={props.cssPx} alt="" draggable={false} decoding="async" />
            )}
        </Show>
    );
}
