// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Media pane — view an image or video file from local disk. If the target
// path is a directory, shows the most-recently-modified matching file in it
// and live-updates whenever a new/changed file appears (a fresh ComfyUI
// render landing in a project's `clips/` folder, for example) — no manual
// reload. If the target path is a single file, it's shown as-is with no
// watch (nothing to "watch for" on a fixed single file in v1).
//
// Spec: docs/specs/SPEC_MEDIA_PANE_2026_07_26.md

import { getApi } from "@/app/store/app-api";
import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fetch } from "@/util/fetchutil";
import { fireAndForget } from "@/util/util";
import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";

const META_PATH = "media:path" as const;

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp"];
const VIDEO_EXTENSIONS = ["webm", "mp4", "mov"];
// Fixed default filter for directory-mode watching — not user-configurable
// in v1 (SPEC_MEDIA_PANE_2026_07_26.md open question #3 leans toward this).
const ALL_MEDIA_EXTENSIONS = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS];

export function extOf(path: string): string {
    const idx = path.lastIndexOf(".");
    return idx === -1 ? "" : path.slice(idx + 1).toLowerCase();
}

// Best-effort join matching whichever separator style the parent path
// already uses — good enough for the paths ListEditorDirCommand echoes back
// (it round-trips the OS's own separator), not a general path library.
export function joinPath(dir: string, name: string): string {
    const sep = dir.includes("\\") && !dir.includes("/") ? "\\" : "/";
    return dir.endsWith(sep) ? dir + name : dir + sep + name;
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

    constructor(blockId: string) {
        this.viewType = "media";
        this.blockId = blockId;
    }

    get viewComponent(): ViewComponent {
        return MediaView as unknown as ViewComponent;
    }
}

function MediaView({ model }: { model: MediaViewModel }): JSX.Element {
    const [pathInput, setPathInput] = createSignal("");
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

    // Resolve `path` to what should actually be shown: if it's a directory,
    // pick the newest matching file in it and start watching for new ones;
    // otherwise show it directly as a single fixed file (no watch).
    const resolveAndShow = async (path: string) => {
        setErrorMsg("");
        if (!path) {
            stopWatching();
            setDisplayPath("");
            return;
        }
        try {
            const { entries } = await RpcApi.ListEditorDirCommand(TabRpcClient, { path });
            const candidates = entries
                .filter((e) => !e.is_dir && ALL_MEDIA_EXTENSIONS.includes(extOf(e.name)))
                .sort((a, b) => (b.mtime ?? 0) - (a.mtime ?? 0));
            startWatching(path);
            if (candidates.length === 0) {
                setDisplayPath("");
                setErrorMsg("No image/video files in this directory yet.");
                return;
            }
            setDisplayPath(joinPath(path, candidates[0].name));
        } catch {
            // Not a directory (or doesn't exist as one) — treat as a direct
            // file path. stream-local-file itself reports a clearer error
            // (not found / not a file) if this guess is wrong too.
            stopWatching();
            setDisplayPath(path);
        }
    };

    onMount(() => {
        const blockData = getWaveObjectAtom<Block>(makeORef("block", model.blockId))();
        const saved = blockData?.meta?.[META_PATH];
        if (typeof saved === "string" && saved.length > 0) {
            setPathInput(saved);
            void resolveAndShow(saved);
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

    const commitPath = () => {
        const path = pathInput().trim();
        fireAndForget(() =>
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: makeORef("block", model.blockId),
                meta: { [META_PATH]: path || null },
            }),
        );
        void resolveAndShow(path);
    };

    const kind = () => {
        const ext = extOf(displayPath());
        if (IMAGE_EXTENSIONS.includes(ext)) return "image";
        if (VIDEO_EXTENSIONS.includes(ext)) return "video";
        return "none";
    };

    return (
        <div class="media-view flex flex-col w-full h-full">
            <div class="media-view-pathbar flex" style={{ gap: "6px", padding: "6px 8px" }}>
                <input
                    class="media-view-path-input"
                    style={{ flex: 1 }}
                    value={pathInput()}
                    placeholder="Absolute path to a file or directory…"
                    onInput={(e) => setPathInput(e.currentTarget.value)}
                    onKeyDown={(e) => {
                        if (e.key === "Enter") commitPath();
                    }}
                    onBlur={commitPath}
                />
            </div>
            <div class="media-view-content flex-1 flex items-center justify-center" style={{ position: "relative", overflow: "hidden" }}>
                <Show when={errorMsg()}>
                    <div class="media-view-empty">{errorMsg()}</div>
                </Show>
                <Show when={!errorMsg() && displayPath() && !objectUrl()}>
                    <div class="media-view-empty">Loading…</div>
                </Show>
                <Show when={!errorMsg() && kind() === "image" && objectUrl()}>
                    <img
                        class="media-view-media max-w-full max-h-full"
                        style={{ "object-fit": "contain", opacity: mediaReady() ? 1 : 0, transition: "opacity 120ms ease" }}
                        src={objectUrl()}
                        onLoad={() => setMediaReady(true)}
                        onError={() => setErrorMsg("Failed to display media (unsupported format or corrupt file).")}
                    />
                </Show>
                <Show when={!errorMsg() && kind() === "video" && objectUrl()}>
                    <video
                        class="media-view-media max-w-full max-h-full"
                        style={{ "object-fit": "contain", opacity: mediaReady() ? 1 : 0, transition: "opacity 120ms ease" }}
                        src={objectUrl()}
                        controls
                        onLoadedData={() => setMediaReady(true)}
                        onError={() => setErrorMsg("Failed to display media (unsupported format or corrupt file).")}
                    />
                </Show>
                <Show when={!errorMsg() && kind() === "none" && !displayPath()}>
                    <div class="media-view-empty">Point this pane at an image, video, or a directory containing one.</div>
                </Show>
            </div>
        </div>
    );
}

export { MediaViewModel };
