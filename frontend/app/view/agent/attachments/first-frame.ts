// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * A tile's picture for a PDF or a video: its first page or first frame, drawn small in this window from the stored
 * file. Nothing is sent anywhere: the PDF opens in pdf.js with scripting, XFA and eval off (the Editor's own
 * rules), and the video is decoded by the browser, so a file the browser can't play (a codec it lacks, a broken
 * file, a password-protected PDF) has no picture and the tile keeps its type icon.
 * docs/specs/SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §12 (questions 1 and 2).
 */

/** The longest side of the picture, in CSS pixels; drawn at 2x so it stays sharp on a dense screen. */
const EDGE = 96;
/** The largest file read for a picture. A bigger one keeps its icon: the tile never reads more than it shows. */
export const MAX_PDF_BYTES = 30 * 1024 * 1024;
export const MAX_VIDEO_BYTES = 60 * 1024 * 1024;
/** How long a file may take to give its picture. */
const GIVE_UP_MS = 8000;

export type FrameKind = "pdf" | "video";

export function frameLimit(kind: FrameKind): number {
    return kind === "pdf" ? MAX_PDF_BYTES : MAX_VIDEO_BYTES;
}

/** Whether a tile of this kind and size gets a picture drawn from its file. */
export function wantsFrame(kind: string, bytes: number | undefined): kind is FrameKind {
    return (kind === "pdf" || kind === "video") && bytes != null && bytes > 0 && bytes <= frameLimit(kind);
}

/** The scale that fits a `w` x `h` picture into EDGE x2 on its longer side (never above 1 for a small source). */
export function fitScale(w: number, h: number): number {
    const longer = Math.max(w, h);
    return longer > 0 ? Math.min(1, (EDGE * 2) / longer) : 1;
}

function canvasBlob(canvas: HTMLCanvasElement): Promise<Blob | null> {
    return new Promise((resolve) => canvas.toBlob(resolve, "image/jpeg", 0.8));
}

/** A promise that gives up (null) after GIVE_UP_MS. */
function within<T>(work: Promise<T | null>): Promise<T | null> {
    return new Promise((resolve) => {
        const timer = setTimeout(() => resolve(null), GIVE_UP_MS);
        work.then(
            (v) => {
                clearTimeout(timer);
                resolve(v);
            },
            () => {
                clearTimeout(timer);
                resolve(null);
            }
        );
    });
}

/** The first page of a PDF, or null when it can't be opened (a password, a damaged file). */
async function pdfFirstPage(blob: Blob): Promise<Blob | null> {
    const { loadPdfjs, SAFE_DOCUMENT_OPTIONS } = await import("@/app/view/editor/viewers/pdf/pdfjs-runtime");
    const { pdfjs } = await loadPdfjs();
    const task = pdfjs.getDocument({ ...SAFE_DOCUMENT_OPTIONS, data: new Uint8Array(await blob.arrayBuffer()) });
    try {
        const doc = await task.promise;
        const page = await doc.getPage(1);
        const natural = page.getViewport({ scale: 1 });
        const viewport = page.getViewport({ scale: fitScale(natural.width, natural.height) });
        const canvas = document.createElement("canvas");
        canvas.width = Math.max(1, Math.floor(viewport.width));
        canvas.height = Math.max(1, Math.floor(viewport.height));
        const context = canvas.getContext("2d");
        if (!context) return null;
        // A page with no background of its own is white, as a person would see it.
        context.fillStyle = "#fff";
        context.fillRect(0, 0, canvas.width, canvas.height);
        await page.render({ canvas, canvasContext: context, viewport }).promise;
        return await canvasBlob(canvas);
    } finally {
        void task.destroy().catch(() => {});
    }
}

/** The first frame of a video the browser can play, or null. */
function videoFirstFrame(blob: Blob): Promise<Blob | null> {
    return new Promise((resolve) => {
        const source = URL.createObjectURL(blob);
        const video = document.createElement("video");
        const done = (result: Blob | null) => {
            video.removeAttribute("src");
            video.load();
            URL.revokeObjectURL(source);
            resolve(result);
        };
        video.muted = true;
        video.preload = "auto";
        video.playsInline = true;
        video.onerror = () => done(null);
        video.onloadedmetadata = () => {
            // A moment in, not frame zero, which is often black.
            video.currentTime = Math.min(0.1, (video.duration || 0) / 2);
        };
        video.onseeked = () => {
            const scale = fitScale(video.videoWidth, video.videoHeight);
            const canvas = document.createElement("canvas");
            canvas.width = Math.max(1, Math.floor(video.videoWidth * scale));
            canvas.height = Math.max(1, Math.floor(video.videoHeight * scale));
            const context = canvas.getContext("2d");
            if (!context || !video.videoWidth) return done(null);
            context.drawImage(video, 0, 0, canvas.width, canvas.height);
            void canvasBlob(canvas).then(done, () => done(null));
        };
        video.src = source;
    });
}

/** The picture for a stored PDF or video, or null when it has none. */
export function firstFrame(kind: FrameKind, blob: Blob): Promise<Blob | null> {
    if (blob.size === 0 || blob.size > frameLimit(kind)) return Promise.resolve(null);
    return within(kind === "pdf" ? pdfFirstPage(blob) : videoFirstFrame(blob));
}
