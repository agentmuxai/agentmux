// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Loading local media files into the UI, shared by the Media pane
 * (`view/media/media.tsx`) and inline media in agent messages
 * (`markdown-media.tsx`, SPEC_AGENT_PANE_RICH_OUTPUT_2026_09_27.md §4).
 */

import { getApi } from "@/app/store/app-api";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fetch } from "@/util/fetchutil";

/**
 * Includes SVG: both the Media pane and inline media only ever show it through
 * `<img>` from a blob URL, where its scripts never run.
 */
export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "svg"];
export const VIDEO_EXTENSIONS = ["webm", "mp4", "mov"];
// PCM WAV only — Chromium's <audio> element supports it natively and
// unconditionally (open format, no codec-licensing gate), unlike MP4/MOV
// which need the CEF build's proprietary-codec flag. Not adding mkv:
// Chromium's <video> element doesn't reliably accept the Matroska
// container itself for direct playback regardless of codec support
// (browsers generally only support WebM, a constrained Matroska profile —
// see the "Post-implementation corrections" note in SPEC_MEDIA_PANE_2026_07_26.md).
export const AUDIO_EXTENSIONS = ["wav"];

/** Images an agent message may show inline: the same set as the Media pane. */
export const INLINE_IMAGE_EXTENSIONS = IMAGE_EXTENSIONS;

/** Client-side caps for inline media (§4.2, §5); srv's own limit is 500 MB. */
export const INLINE_IMAGE_MAX_BYTES = 25 * 1024 * 1024;
export const INLINE_AV_MAX_BYTES = 200 * 1024 * 1024;

export function extOf(path: string): string {
    const idx = path.lastIndexOf(".");
    return idx === -1 ? "" : path.slice(idx + 1).toLowerCase();
}

// Containing directory of `path`, matching whichever separator style it
// uses. Empty string if `path` has no separator (shouldn't happen for an
// absolute path from the native dialog, but fail closed rather than throw).
export function dirnameOf(path: string): string {
    const lastSlash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return lastSlash === -1 ? "" : path.slice(0, lastSlash);
}

// File name of `path` (the part after the last separator), matching
// whichever separator style it uses. Returns `path` unchanged if it has no
// separator.
export function basenameOf(path: string): string {
    const lastSlash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return lastSlash === -1 ? path : path.slice(lastSlash + 1);
}

/** What an inline reference to `path` renders as, by extension. */
export function inlineMediaKind(path: string): "image" | "video" | "audio" | null {
    const ext = extOf(path);
    if (INLINE_IMAGE_EXTENSIONS.includes(ext)) return "image";
    if (VIDEO_EXTENSIONS.includes(ext)) return "video";
    if (AUDIO_EXTENSIONS.includes(ext)) return "audio";
    return null;
}

const ABSOLUTE_PATH = /^(?:[A-Za-z]:[\\/]|[\\/]|~[\\/])/;

/**
 * The file a markdown image `src` refers to: absolute (`C:/…`, `/…`, `~/…`,
 * `file:///…`) as-is, anything else relative to `baseDir`. `null` for a
 * relative path with no `baseDir` to resolve it against. Percent-escapes are
 * decoded, since the markdown pipeline encodes spaces in `<…>` paths.
 */
export function resolveMediaPath(src: string, baseDir: string): string | null {
    let path = src;
    try {
        path = decodeURI(path);
    } catch {
        // A literal `%` that isn't an escape: use the text as written.
    }
    if (/^file:\/\//i.test(path)) {
        path = path.slice("file://".length);
        if (/^\/[A-Za-z]:[\\/]/.test(path)) path = path.slice(1);
        // `file://host/share/…` names a network host: keep it one, so
        // isNetworkPath refuses it rather than it resolving as a local path.
        else if (!path.startsWith("/")) path = "//" + path;
    }
    if (ABSOLUTE_PATH.test(path)) return path;
    if (!baseDir) return null;
    return baseDir.replace(/[\\/]+$/, "") + "/" + path.replace(/^\.[\\/]/, "");
}

/**
 * A UNC path (`//host/…`, `\\host\…`). Windows opens one as an SMB connection
 * to that host, which can hand it the user's credentials, so inline media never
 * loads one (Codex P1 on #4064): a prompt-injected agent could otherwise
 * trigger that with no click, the thing the remote-image chip exists to stop.
 */
export function isNetworkPath(path: string): boolean {
    return /^[\\/]{2}/.test(path);
}

// Surfaces the browser's actual MediaError code/message instead of a
// generic "failed" string — code 4 (MEDIA_ERR_SRC_NOT_SUPPORTED) is what
// Chromium reports when it has no registered decoder for the file's codec,
// which is exactly what happens for H.264/AAC MP4s on a CEF build compiled
// without proprietary_codecs=true. Distinguishing that from a genuinely
// corrupt file (would show as code 3, MEDIA_ERR_DECODE) is the whole point
// of showing this instead of one flat message for every failure.
export function describeMediaError(el: HTMLMediaElement | undefined): string {
    const err = el?.error;
    if (!err) return "unknown error";
    const codeNames: Record<number, string> = {
        1: "MEDIA_ERR_ABORTED",
        2: "MEDIA_ERR_NETWORK",
        3: "MEDIA_ERR_DECODE",
        4: "MEDIA_ERR_SRC_NOT_SUPPORTED (likely a missing/disabled codec for this container, not a corrupt file)",
    };
    const codeName = codeNames[err.code] ?? `code ${err.code}`;
    return err.message ? `${codeName}: ${err.message}` : codeName;
}

export function streamUrl(path: string): string {
    return getWebServerEndpoint() + "/agentmux/stream-local-file?path=" + encodeURIComponent(path);
}

/** The file exists but is over the caller's `maxBytes`; nothing was downloaded. */
export class MediaTooLargeError extends Error {
    constructor(readonly size: number) {
        super(`file is ${size} bytes`);
    }
}

/** srv answered with an error status (404 for a missing file). */
export class MediaFetchError extends Error {
    constructor(readonly status: number, statusText: string) {
        super(`${status} ${statusText}`);
    }
}

export interface FetchMediaOpts {
    /** Refuse, before downloading the body, anything larger. */
    maxBytes?: number;
    /** Force the blob's MIME type (SVG must be image/svg+xml to render). */
    type?: string;
    signal?: AbortSignal;
}

// `<img src>`/`<video src>` can't attach the `X-AuthKey` header
// stream-local-file requires (it lives in `authed_routes`, and the
// query-string `?authkey=` fallback is deliberately restricted to the
// `/ws` upgrade route only — see auth_middleware's 2026-05-11 audit
// comment in agentmux-srv/src/server/mod.rs). Fetch the bytes ourselves
// with the header (same pattern as fetchMuxFile in mux-file.ts) and
// hand the element a blob object URL instead. Caller owns revoking it.
export async function fetchMediaBlob(path: string, opts: FetchMediaOpts = {}): Promise<Blob> {
    const headers: Record<string, string> = {};
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) headers["X-AuthKey"] = authKey;
    }
    const resp = await fetch(streamUrl(path), { headers, signal: opts.signal });
    if (!resp.ok) {
        throw new MediaFetchError(resp.status, resp.statusText);
    }
    if (opts.maxBytes != null) {
        const declared = Number(resp.headers.get("Content-Length"));
        if (declared > opts.maxBytes) {
            void resp.body?.cancel().catch(() => {});
            throw new MediaTooLargeError(declared);
        }
    }
    const blob = await resp.blob();
    if (opts.maxBytes != null && blob.size > opts.maxBytes) throw new MediaTooLargeError(blob.size);
    return opts.type && blob.type !== opts.type ? new Blob([blob], { type: opts.type }) : blob;
}

export interface MediaRange {
    /** The requested bytes, or `null` if srv ignored the Range (then nothing was read). */
    blob: Blob | null;
    /** The whole file's size. */
    total: number;
}

/**
 * Reads bytes `start..=end` of a local file (srv answers `206`), and the whole
 * file's size from `Content-Range`: a poster frame or a size, without
 * downloading the file (§5).
 */
export async function fetchMediaRange(
    path: string,
    start: number,
    end: number,
    opts: { signal?: AbortSignal; type?: string } = {},
): Promise<MediaRange> {
    const headers: Record<string, string> = { Range: `bytes=${start}-${end}` };
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) headers["X-AuthKey"] = authKey;
    }
    const resp = await fetch(streamUrl(path), { headers, signal: opts.signal });
    if (!resp.ok) throw new MediaFetchError(resp.status, resp.statusText);
    if (resp.status !== 206) {
        void resp.body?.cancel().catch(() => {});
        return { blob: null, total: Number(resp.headers.get("Content-Length")) || 0 };
    }
    const total = Number(/\/(\d+)\s*$/.exec(resp.headers.get("Content-Range") ?? "")?.[1] ?? 0);
    const blob = await resp.blob();
    return { blob: opts.type && blob.type !== opts.type ? new Blob([blob], { type: opts.type }) : blob, total };
}

/** "31.2 MB", "840 KB": for a file card. */
export function formatBytes(n: number): string {
    if (n >= 1024 * 1024) return `${Math.round((n / (1024 * 1024)) * 10) / 10} MB`;
    if (n >= 1024) return `${Math.round(n / 1024)} KB`;
    return `${n} B`;
}
