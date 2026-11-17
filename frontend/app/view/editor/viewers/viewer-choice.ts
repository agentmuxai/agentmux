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

export type ViewerId = "text" | "large" | "image" | "media" | "pdf" | "hex" | "info";

/** readeditorfile's cap: a larger text file isn't loaded into CodeMirror. */
export const TEXT_MAX_BYTES = 10_000_000;

export const VIEWER_IDS: readonly ViewerId[] = ["text", "large", "image", "media", "pdf", "hex", "info"];

export const VIEWER_LABELS: Record<ViewerId, { label: string; icon: string }> = {
    text: { label: "Text", icon: "file-lines" },
    large: { label: "Large text", icon: "file-lines" },
    image: { label: "Image", icon: "image" },
    media: { label: "Audio and video", icon: "film" },
    pdf: { label: "PDF", icon: "file-pdf" },
    hex: { label: "Hex", icon: "table-cells" },
    info: { label: "File info", icon: "circle-info" },
};

export function isViewerId(v: unknown): v is ViewerId {
    return typeof v === "string" && (VIEWER_IDS as readonly string[]).includes(v);
}

/** The probe of a file, as far as the viewers need it. */
export type FileProbe = FileProbeResult;

/** The viewers that can show the file, the default first. Every file can
 *  be shown as hex: its bytes are read in parts, whatever its size. A file
 *  over the editor's cap is shown as text by the Large text viewer, which
 *  reads it in parts too. */
export function viewerOptions(probe: FileProbe): ViewerId[] {
    const text: ViewerId[] = [probe.size <= TEXT_MAX_BYTES ? "text" : "large"];
    switch (probe.kind) {
        case "text":
            return [...text, "info", "hex"];
        case "svg":
            return ["image", ...text, "info", "hex"];
        case "image":
            // Text after the card: a way out when the probe was wrong about a file.
            return ["image", "info", ...text, "hex"];
        case "audio":
        case "video":
            return ["media", "info", "hex"];
        case "pdf":
            return ["pdf", "info", ...text, "hex"];
        case "other":
            // Binary of no kind the probe knows: its bytes.
            return ["hex", "info", ...text];
        default:
            // A kind with a name and no viewer yet (an archive, a database, a
            // font, an Office document): the info card, which says what it is
            // and opens it with the system's app.
            return ["info", ...text, "hex"];
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
 *  its bytes in the image, media or PDF viewer, else the info card. The
 *  user's choice of the card or of hex wins: both show any bytes. */
export function viewerForDropped(name: string, type: string, override?: string): ViewerId {
    if (override === "info" || override === "hex") return override;
    const ext = extOf(name);
    if (type.startsWith("image/") || IMAGE_EXTENSIONS.includes(ext)) return "image";
    if (type.startsWith("video/") || type.startsWith("audio/")) return "media";
    if (type === "application/pdf" || ext === "pdf") return "pdf";
    if (VIDEO_EXTENSIONS.includes(ext) || AUDIO_EXTENSIONS.includes(ext)) return "media";
    return "info";
}

/** The viewers a dropped file can be shown in, its own first. */
export function droppedViewerOptions(name: string, type: string): ViewerId[] {
    const own = viewerForDropped(name, type);
    return own === "info" ? ["info", "hex"] : [own, "info", "hex"];
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
