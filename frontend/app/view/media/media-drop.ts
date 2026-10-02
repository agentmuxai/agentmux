// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a Media pane accepts from a drag, and what it does with a drop.
// SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.3.

import { AUDIO_EXTENSIONS, basenameOf, extOf, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS } from "@/app/element/local-media";
import type { DragFiles, DropVerdict, FileDropHook } from "@/app/drag/file-drop";

// Fixed default filter for directory-mode watching — not user-configurable
// in v1 (SPEC_MEDIA_PANE_2026_07_26.md open question #3 leans toward this).
export const ALL_MEDIA_EXTENSIONS = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS, ...AUDIO_EXTENSIONS];

const MEDIA_MIME_TYPES = new Set([
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/svg+xml",
    "video/webm",
    "video/mp4",
    "video/quicktime",
    "audio/wav",
    "audio/x-wav",
    "audio/wave",
    "audio/vnd.wave",
]);

export const isMediaName = (name: string) => ALL_MEDIA_EXTENSIONS.includes(extOf(name));

/**
 * Whether a file drag can open in a media pane: exactly one file of a type it
 * plays. Names decide when known, else the MIME type; an unknown type is let
 * through and the drop reports it if it can't be shown.
 */
export function mediaDropVerdict(drag: DragFiles): DropVerdict {
    const refuse: DropVerdict = { ok: false, reason: "Media panes open one image, video or audio file" };
    const open: DropVerdict = { ok: true, message: "Open here", icon: "fa-photo-film" };
    if (drag.count !== 1) return refuse;
    const name = drag.names?.[0];
    if (name !== undefined) return isMediaName(name) ? open : refuse;
    const type = drag.types[0] ?? "";
    if (type === "") return open;
    return MEDIA_MIME_TYPES.has(type) ? open : refuse;
}

export interface MediaDropActions {
    showPath(path: string): void;
    /** A file with no host path: shown from its bytes, without live updates. */
    showFile(file: File): void;
    cantOpen(name: string): void;
}

export function createMediaDropHook(actions: MediaDropActions): FileDropHook {
    return {
        accept: mediaDropVerdict,
        drop({ paths, files }) {
            const path = paths[0];
            if (path !== undefined) {
                if (isMediaName(path)) actions.showPath(path);
                else actions.cantOpen(basenameOf(path));
                return;
            }
            const file = files[0];
            if (!file) return;
            if (isMediaName(file.name)) actions.showFile(file);
            else actions.cantOpen(file.name || "file");
        },
    };
}
