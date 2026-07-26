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

import { BlockNodeModel } from "@/app/block/blocktypes";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { getWebServerEndpoint } from "@/util/endpoints";
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
    const [errorMsg, setErrorMsg] = createSignal("");
    const [mediaReady, setMediaReady] = createSignal(false);

    let watchedDir: string | null = null;
    let unsubFileChanged: () => void = () => {};

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
                if (path) setDisplayPath(path);
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
    });

    createEffect(() => {
        displayPath();
        setMediaReady(false);
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
                <Show when={!errorMsg() && kind() === "image"}>
                    <img
                        class="media-view-media max-w-full max-h-full"
                        style={{ "object-fit": "contain", opacity: mediaReady() ? 1 : 0, transition: "opacity 120ms ease" }}
                        src={streamUrl(displayPath())}
                        onLoad={() => setMediaReady(true)}
                    />
                </Show>
                <Show when={!errorMsg() && kind() === "video"}>
                    <video
                        class="media-view-media max-w-full max-h-full"
                        style={{ "object-fit": "contain", opacity: mediaReady() ? 1 : 0, transition: "opacity 120ms ease" }}
                        src={streamUrl(displayPath())}
                        controls
                        onLoadedData={() => setMediaReady(true)}
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
