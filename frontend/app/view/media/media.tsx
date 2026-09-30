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

import type { PaneTabHostContext, PaneTabManifest } from "@/app/block/pane-tab-registry";
import { getApi } from "@/app/store/app-api";
import {
    AUDIO_EXTENSIONS,
    basenameOf,
    describeMediaError,
    dirnameOf,
    extOf,
    fetchMediaBlob,
    IMAGE_EXTENSIONS,
    VIDEO_EXTENSIONS,
} from "@/app/element/local-media";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { makeORef } from "@/app/store/mos";
import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { fireAndForget } from "@/util/util";
import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { registerFileDropTarget, type DragFiles, type DropVerdict, type FileDropHook } from "@/app/drag/file-drop";
import { notifyDrop } from "@/app/drag/file-drop-actions";

export { basenameOf, dirnameOf, extOf };

const META_PATH = "media:path" as const;

// Fixed default filter for directory-mode watching — not user-configurable
// in v1 (SPEC_MEDIA_PANE_2026_07_26.md open question #3 leans toward this).
const ALL_MEDIA_EXTENSIONS = [...IMAGE_EXTENSIONS, ...VIDEO_EXTENSIONS, ...AUDIO_EXTENSIONS];

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

const isMediaName = (name: string) => ALL_MEDIA_EXTENSIONS.includes(extOf(name));

/**
 * Whether a file drag can open in a media pane: exactly one file of a type it
 * plays. Names decide when known, else the MIME type; an unknown type is let
 * through and the drop reports it if it can't be shown.
 * SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.3.
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

/** The pane's title: the file's basename, or "Media" before one is picked. */
export function mediaTitle(meta: MetaType | undefined): string {
    const path = meta?.[META_PATH];
    return typeof path === "string" && path.length > 0 ? basenameOf(path) : "Media";
}

/**
 * Media as a native pane tab (Pane Tab contract Phase 2c): no ViewModel — the
 * view reads and writes its picked path through the host context, and the
 * file's name titles the pane.
 */
export const mediaPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "media",
    label: "Media",
    icon: "photo-film",
    create: (ctx) => ({
        component: () => <MediaView ctx={ctx} />,
        liveTitle: () => ({ text: mediaTitle(ctx.meta()) }),
    }),
};

function MediaView(props: { ctx: PaneTabHostContext }): JSX.Element {
    const ctx = props.ctx;
    const [displayPath, setDisplayPath] = createSignal("");
    // Bumped on every MPS change event, even ones that leave displayPath's
    // string value unchanged (a pipeline overwriting a stable filename in
    // place) — Solid's signal wouldn't otherwise notice anything changed
    // and the fetch effect below would never re-run. Codex review.
    const [revision, setRevision] = createSignal(0);
    const [objectUrl, setObjectUrl] = createSignal<string | null>(null);
    const [errorMsg, setErrorMsg] = createSignal("");
    const [mediaReady, setMediaReady] = createSignal(false);
    // A dropped file with no host path, shown from its bytes (no live updates).
    const [localName, setLocalName] = createSignal("");

    let watchedDir: string | null = null;
    let unsubFileChanged: () => void = () => {};
    let currentObjectUrl: string | null = null;
    let fetchToken = 0;

    const stopWatching = () => {
        if (watchedDir != null) {
            fireAndForget(() =>
                RpcApi.UnwatchMediaDirCommand(TabRpcClient, { path: watchedDir!, block_id: ctx.blockId }),
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
                block_id: ctx.blockId,
                extensions: ALL_MEDIA_EXTENSIONS,
            }),
        );
        unsubFileChanged = muxEventSubscribe({
            eventType: WpsEvent.MediaFileChanged,
            scope: makeORef("block", ctx.blockId),
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
    // pipeline), the pane live-swaps to it via the MPS handler above.
    const showPath = (path: string) => {
        setErrorMsg("");
        setLocalName("");
        setDisplayPath(path);
        const dir = dirnameOf(path);
        if (dir) startWatching(dir);
    };

    // Native "open file" dialog — the only way to point this pane at
    // something (no path text entry, per design). Persists the pick so it
    // survives a pane reload.
    const openPath = (path: string) => {
        fireAndForget(() => ctx.setMeta({ [META_PATH]: path }));
        showPath(path);
    };

    const pickFile = async () => {
        const path = await getApi()?.showOpenFileDialog?.();
        if (!path) return; // user cancelled
        openPath(path);
    };

    const showFile = (file: File) => {
        stopWatching();
        fetchToken++; // drop any fetch still in flight for the previous path
        fireAndForget(() => ctx.setMeta({ [META_PATH]: "" }));
        setDisplayPath("");
        setErrorMsg("");
        const url = URL.createObjectURL(file);
        const prev = currentObjectUrl;
        currentObjectUrl = url;
        setLocalName(file.name);
        setMediaReady(false);
        setObjectUrl(url);
        if (prev) URL.revokeObjectURL(prev);
    };

    onMount(() => {
        const dispose = registerFileDropTarget(
            ctx.blockId,
            createMediaDropHook({
                showPath: openPath,
                showFile,
                cantOpen: (name) => notifyDrop.cantOpen(name, "media pane"),
            }),
        );
        onCleanup(dispose);
        const saved = ctx.meta()?.[META_PATH];
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
        // A dropped file with no path is showFile's: nothing to fetch, and
        // clearing here (this can run after showFile) would revoke its URL.
        if (!path && localName()) return;
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
        const ext = extOf(displayPath() || localName());
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
