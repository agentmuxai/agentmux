// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which viewer shows a file in the Editor, from srv's probe of it (content
 * first, the extension only where content leaves it open), and which other
 * viewers "Reopen with…" offers. Pure: the model and the tests call it.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.2, §4.3.
 */

import type { FileProbeResult } from "@/types/rpc/FileProbeResult";
import { AUDIO_EXTENSIONS, extOf, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS } from "@/app/element/local-media";

export type ViewerId = "text" | "image" | "media" | "pdf" | "info";

/** readeditorfile's cap: a larger text file isn't loaded into CodeMirror. */
export const TEXT_MAX_BYTES = 10_000_000;

export const VIEWER_IDS: readonly ViewerId[] = ["text", "image", "media", "pdf", "info"];

export const VIEWER_LABELS: Record<ViewerId, { label: string; icon: string }> = {
    text: { label: "Text", icon: "file-lines" },
    image: { label: "Image", icon: "image" },
    media: { label: "Audio and video", icon: "film" },
    pdf: { label: "PDF", icon: "file-pdf" },
    info: { label: "File info", icon: "circle-info" },
};

export function isViewerId(v: unknown): v is ViewerId {
    return typeof v === "string" && (VIEWER_IDS as readonly string[]).includes(v);
}

/** The probe of a file, as far as the viewers need it. */
export type FileProbe = FileProbeResult;

/** The viewers that can show the file, the default first. */
export function viewerOptions(probe: FileProbe): ViewerId[] {
    const textOk = probe.size <= TEXT_MAX_BYTES;
    switch (probe.kind) {
        case "text":
            return textOk ? ["text", "info"] : ["info"];
        case "svg":
            return textOk ? ["image", "text", "info"] : ["image", "info"];
        case "image":
            // Text last: a way out when the probe was wrong about a file.
            return textOk ? ["image", "info", "text"] : ["image", "info"];
        case "audio":
        case "video":
            return ["media", "info"];
        case "pdf":
            return textOk ? ["pdf", "info", "text"] : ["pdf", "info"];
        default:
            // Binary nothing renders: the info card, or the bytes as text
            // ("Open as text anyway") while it fits the editor.
            return textOk ? ["info", "text"] : ["info"];
    }
}

/** The viewer to open the file in: the user's choice when the file can be
 *  shown that way, else the default for its kind. */
export function chooseViewer(probe: FileProbe, override?: string): ViewerId {
    const options = viewerOptions(probe);
    if (isViewerId(override) && options.includes(override)) return override;
    return options[0];
}

/** A dropped file with no host path, by its name and MIME type: shown from
 *  its bytes in the image or media viewer, else the info card. */
export function viewerForDropped(name: string, type: string): ViewerId {
    const ext = extOf(name);
    if (type.startsWith("image/") || IMAGE_EXTENSIONS.includes(ext)) return "image";
    if (type.startsWith("video/") || type.startsWith("audio/")) return "media";
    if (type === "application/pdf" || ext === "pdf") return "pdf";
    if (VIDEO_EXTENSIONS.includes(ext) || AUDIO_EXTENSIONS.includes(ext)) return "media";
    return "info";
}

/** Opening these with the system runs them: the editor asks first. */
const PROGRAM = /\.(exe|msi|com|scr|bat|cmd|ps1|lnk|app|pkg|appx|msix|url|reg|jar|sh)$/i;

/** Whether opening the file with the system's app would run it. */
export function isProgramPath(path: string): boolean {
    return PROGRAM.test(path);
}

/** Whether the probe says the file is binary (not text, and not SVG). */
export function isBinaryProbe(probe: FileProbe): boolean {
    return probe.text == null;
}
