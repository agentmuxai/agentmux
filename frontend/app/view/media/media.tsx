// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Media pane — view an image or video file from local disk, picked via a
// native file dialog (no path text entry). Once a file is picked, its
// containing directory is watched — if a newer matching file appears there
// (a fresh ComfyUI render landing in a project's `clips/` folder, for
// example) the pane live-swaps to it automatically, no manual reload.
//
// Spec: docs/specs/SPEC_MEDIA_PANE_2026_07_26.md

import { getApi } from "@/app/store/app-api";
import { BlockNodeModel } from "@/app/block/blocktypes";
import { useBlockAtom } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fetch } from "@/util/fetchutil";
import { fireAndForget } from "@/util/util";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";

const META_PATH = "media:path" as const;

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp"];
const VIDEO_EXTENSIONS = ["webm", "mp4", "mov"];
// PCM WAV only — Chromium's <audio> element supports it natively and
// unconditionally (open format, no codec-licensing gate), unlike MP4/MOV
// which need the CEF build's proprietary-codec flag. Not adding mkv:
// Chromium's <video> element doesn't reliably accept the Matroska
// container itself for direct playback regardless of codec support
// (browsers generally only support WebM, a constrained Matroska profile —
// see the "Post-implementation corrections" note in the spec doc).
const AUDIO_EXTENSIONS = ["wav"];
// Fixed default filter for directory-mode watching — not user-configurable
// in v1 (SPEC_MEDIA_PANE_2026_07_26.md open question #3 leans toward this).
const ALL_MEDIA_EXTENSIONS = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS, ...AUDIO_EXTENSIONS];

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

// Surfaces the browser's actual MediaError code/message instead of a
// generic "failed" string — code 4 (MEDIA_ERR_SRC_NOT_SUPPORTED) is what
// Chromium reports when it has no registered decoder for the file's codec,
// which is exactly what happens for H.264/AAC MP4s on a CEF build compiled
// without proprietary_codecs=true. Distinguishing that from a genuinely
// corrupt file (would show as code 3, MEDIA_ERR_DECODE) is the whole point
// of showing this instead of one flat message for every failure.
function describeMediaError(el: HTMLMediaElement | undefined): string {
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

function streamUrl(path: string): string {
    return getWebServerEndpoint() + "/agentmux/stream-local-file?path=" + encodeURIComponent(path);
}

// `<img src>`/`<video src>` can't attach the `X-AuthKey` header
// stream-local-file requires (it lives in `authed_routes`, and the
// query-string `?authkey=` fallback is deliberately restricted to the
// `/ws` upgrade route only — see auth_middleware's 2026-05-11 audit
// comment in agentmux-srv/src/server/mod.rs). Fetch the bytes ourselves
// with the header (same pattern as fetchWaveFile in wave-file.ts) and
// hand the element a blob object URL instead. Caller owns revoking it.
async function fetchMediaBlob(path: string): Promise<Blob> {
    const headers: Record<string, string> = {};
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) headers["X-AuthKey"] = authKey;
    }
    const resp = await fetch(streamUrl(path), { headers });
    if (!resp.ok) {
        throw new Error(`${resp.status} ${resp.statusText}`);
    }
    return await resp.blob();
}

class MediaViewModel implements ViewModel {
    viewType: string;
    blockId: string;
    viewName: Accessor<string>;

    constructor(blockId: string) {
        this.viewType = "media";
        this.blockId = blockId;

        // Header title — file basename of the persisted path, so an
        // OpenMedia call's default (untitled) pane is distinguishable from
        // another rather than showing a generic "Media" label for all of
        // them. Mirrors EditorViewModel.viewName's pattern exactly
        // (editor-model.ts:290-296): wrapped in useBlockAtom (creates a
        // tracking root) rather than a bare createMemo, so block-frame
        // subscribers reliably see updates.
        this.viewName = useBlockAtom(blockId, "media-view-name", () =>
            createMemo<string>(() => {
                const blockData = getWaveObjectAtom<Block>(makeORef("block", blockId))();
                const path = blockData?.meta?.[META_PATH];
                return typeof path === "string" && path.length > 0 ? basenameOf(path) : "Media";
            }),
        );
    }

    get viewComponent(): ViewComponent {
        return MediaView as unknown as ViewComponent;
    }
}

function MediaView({ model }: { model: MediaViewModel }): JSX.Element {
    const [displayPath, setDisplayPath] = createSignal("");
    // Bumped on every WPS change event, even ones that leave displayPath's
    // string value unchanged (a pipeline overwriting a stable filename in
    // place) — Solid's signal wouldn't otherwise notice anything changed
    // and the fetch effect below would never re-run. Codex review.
    const [revision, setRevision] = createSignal(0);
    const [objectUrl, setObjectUrl] = createSignal<string | null>(null);
    const [errorMsg, setErrorMsg] = createSignal("");
    const [mediaReady, setMediaReady] = createSignal(false);

    let watchedDir: string | null = null;
    let unsubFileChanged: () => void = () => {};
    let currentObjectUrl: string | null = null;
    let fetchToken = 0;

    const stopWatching = () => {
        if (watchedDir != null) {
            fireAndForget(() =>
                RpcApi.UnwatchMediaDirCommand(TabRpcClient, { path: watchedDir!, block_id: model.blockId }),
            );
            watchedDir = null;
        }
        unsubFileChanged();
        unsubFileChanged = () => {};
    };

    const startWatching = (dir: string) => {
        if (watchedDir === dir) return;
        stopWatching();
        watchedDir = dir;
        fireAndForget(() =>
            RpcApi.WatchMediaDirCommand(TabRpcClient, {
                path: dir,
                block_id: model.blockId,
                extensions: ALL_MEDIA_EXTENSIONS,
            }),
        );
        unsubFileChanged = waveEventSubscribe({
            eventType: WpsEvent.MediaFileChanged,
            scope: makeORef("block", model.blockId),
            handler: (event) => {
                const path = (event as any)?.data?.path as string | undefined;
                if (!path) return;
                // Clear a stale "no files yet" message the moment a
                // matching file actually arrives — both render branches
                // below require !errorMsg(), so leaving it set would keep
                // showing the empty-directory message forever. Codex review.
                setErrorMsg("");
                setDisplayPath(path);
                setRevision((r) => r + 1);
            },
        });
    };

    // Show `path` directly, and start watching its containing directory —
    // if a newer matching file lands there (a fresh render from the same
    // pipeline), the pane live-swaps to it via the WPS handler above.
    const showPath = (path: string) => {
        setErrorMsg("");
        setDisplayPath(path);
        const dir = dirnameOf(path);
        if (dir) startWatching(dir);
    };

    // Native "open file" dialog — the only way to point this pane at
    // something (no path text entry, per design). Persists the pick so it
    // survives a pane reload.
    const pickFile = async () => {
        const path = await getApi()?.showOpenFileDialog?.();
        if (!path) return; // user cancelled
        fireAndForget(() =>
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: makeORef("block", model.blockId),
                meta: { [META_PATH]: path },
            }),
        );
        showPath(path);
    };

    onMount(() => {
        const blockData = getWaveObjectAtom<Block>(makeORef("block", model.blockId))();
        const saved = blockData?.meta?.[META_PATH];
        if (typeof saved === "string" && saved.length > 0) {
            showPath(saved);
        }
    });

    onCleanup(() => {
        stopWatching();
        if (currentObjectUrl) URL.revokeObjectURL(currentObjectUrl);
    });

    // Fetch the current path's bytes (with auth) into a blob object URL
    // whenever it changes — including a same-path "revision" bump from an
    // in-place file overwrite, which wouldn't otherwise re-trigger a plain
    // signal dependency on the path string alone.
    createEffect(() => {
        const path = displayPath();
        revision();
        const myToken = ++fetchToken;
        setMediaReady(false);

        if (!path) {
            if (currentObjectUrl) {
                URL.revokeObjectURL(currentObjectUrl);
                currentObjectUrl = null;
            }
            setObjectUrl(null);
            return;
        }

        void (async () => {
            try {
                const blob = await fetchMediaBlob(path);
                if (myToken !== fetchToken) return; // superseded by a newer request
                const url = URL.createObjectURL(blob);
                const prev = currentObjectUrl;
                currentObjectUrl = url;
                setObjectUrl(url);
                setErrorMsg("");
                if (prev) URL.revokeObjectURL(prev);
            } catch (e) {
                if (myToken !== fetchToken) return;
                if (currentObjectUrl) {
                    URL.revokeObjectURL(currentObjectUrl);
                    currentObjectUrl = null;
                }
                setObjectUrl(null);
                setErrorMsg(`Failed to load media: ${(e as Error)?.message ?? e}`);
            }
        })();
    });

    const kind = () => {
        const ext = extOf(displayPath());
        if (IMAGE_EXTENSIONS.includes(ext)) return "image";
        if (VIDEO_EXTENSIONS.includes(ext)) return "video";
        if (AUDIO_EXTENSIONS.includes(ext)) return "audio";
        return "none";
    };

    // Fills the whole content area (not just the text's own bounding box) so
    // clicking anywhere in the pane's background opens the file picker, not
    // only the label itself.
    const emptyStateStyle: JSX.CSSProperties = {
        position: "absolute",
        inset: "0",
        cursor: "pointer",
        color: "var(--secondary-text-color, #888)",
        "text-align": "center",
        display: "flex",
        "flex-direction": "column",
        "align-items": "center",
        "justify-content": "center",
        gap: "6px",
        padding: "24px",
    };
    const explainerStyle: JSX.CSSProperties = {
        "font-size": "0.85em",
        opacity: 0.7,
        "max-width": "320px",
    };
    const supportedTypesText =
        `Images (${IMAGE_EXTENSIONS.join(", ").toUpperCase()}), videos (${VIDEO_EXTENSIONS.join(", ").toUpperCase()}), and audio (${AUDIO_EXTENSIONS.join(", ").toUpperCase()}).`;

    return (
        <div class="media-view flex flex-col w-full h-full">
            <div class="media-view-content flex-1" style={{ position: "relative", overflow: "hidden" }}>
                <Show when={errorMsg()}>
                    <div style={emptyStateStyle} onClick={() => void pickFile()}>
                        <div>{errorMsg()}</div>
                        <div style={explainerStyle}>Click anywhere to pick a different file.</div>
                    </div>
                </Show>
                <Show when={!errorMsg() && displayPath() && !objectUrl()}>
                    <div class="flex items-center justify-center w-full h-full" style={{ color: "var(--secondary-text-color, #888)" }}>
                        Loading…
                    </div>
                </Show>
                <Show when={!errorMsg() && kind() === "image" && objectUrl()}>
                    <img
                        class="media-view-media max-w-full max-h-full"
                        style={{ "object-fit": "contain", opacity: mediaReady() ? 1 : 0, transition: "opacity 120ms ease", position: "absolute", inset: "0", margin: "auto" }}
                        src={objectUrl()}
                        onLoad={() => setMediaReady(true)}
                        onError={() => setErrorMsg("Failed to display media (unsupported format or corrupt file).")}
                    />
                </Show>
                <Show when={!errorMsg() && kind() === "video" && objectUrl()}>
                    <video
                        class="media-view-media max-w-full max-h-full"
                        style={{ "object-fit": "contain", opacity: mediaReady() ? 1 : 0, transition: "opacity 120ms ease", position: "absolute", inset: "0", margin: "auto" }}
                        src={objectUrl()}
                        controls
                        onLoadedData={() => setMediaReady(true)}
                        onError={(e) => setErrorMsg(`Failed to play video — ${describeMediaError(e.currentTarget)}`)}
                    />
                </Show>
                <Show when={!errorMsg() && kind() === "audio" && objectUrl()}>
                    <audio
                        style={{ opacity: mediaReady() ? 1 : 0, transition: "opacity 120ms ease", width: "80%" }}
                        src={objectUrl()}
                        controls
                        onLoadedData={() => setMediaReady(true)}
                        onError={(e) => setErrorMsg(`Failed to play audio — ${describeMediaError(e.currentTarget)}`)}
                    />
                </Show>
                <Show when={!errorMsg() && kind() === "none" && !displayPath()}>
                    <div style={emptyStateStyle} onClick={() => void pickFile()}>
                        <div style={{ "font-size": "1.05em" }}>Click to load media</div>
                        <div style={explainerStyle}>
                            {supportedTypesText} If you pick a file from a folder your agent is
                            actively generating into, this pane updates automatically as new
                            matching files appear there — no need to reopen it.
                        </div>
                    </div>
                </Show>
                <Show when={!errorMsg() && displayPath()}>
                    <button
                        title="Pick a different file"
                        onClick={() => void pickFile()}
                        style={{
                            position: "absolute",
                            top: "6px",
                            right: "6px",
                            "z-index": 1,
                            opacity: 0.6,
                            background: "var(--block-bg-color, rgba(0,0,0,0.4))",
                            border: "none",
                            "border-radius": "4px",
                            padding: "4px 7px",
                            cursor: "pointer",
                        }}
                    >
                        <i class="fa fa-folder-open" />
                    </button>
                </Show>
            </div>
        </div>
    );
}

export { MediaViewModel };
