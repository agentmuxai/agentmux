// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Media document — view an image, video or audio file from local disk,
// picked via a native file dialog (no path text entry). Once a file is
// picked, its containing directory is watched — if a newer matching file
// appears there (a fresh ComfyUI render landing in a project's `clips/`
// folder, for example) the view live-swaps to it automatically, no manual
// reload.
//
// Spec: docs/specs/SPEC_MEDIA_PANE_2026_07_26.md; one of these per document
// tab, docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §6.3.

import { getApi } from "@/app/store/app-api";
import {
    AUDIO_EXTENSIONS,
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
import { createEffect, createSignal, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { registerFileDropTarget } from "@/app/drag/file-drop";
import { notifyDrop } from "@/app/drag/file-drop-actions";
import { ALL_MEDIA_EXTENSIONS, createMediaDropHook } from "./media-drop";

/**
 * One Media document: a file, shown, with its folder watched for newer
 * renders. A Media pane mounts one of these for the tab in front.
 */
export function MediaView(props: {
    blockId: string;
    /** The file this tab shows ("" for none yet). */
    path: string;
    /** The tab now shows `path` (picked, dropped, or a newer render).
     *  False when the tab gave way to another tab already showing it. */
    onPathChange: (path: string) => boolean | void;
    /** A file dropped on the pane: true when the pane opened it elsewhere
     *  (a new tab), false to show it here. */
    openDropped?: (path: string) => boolean;
}): JSX.Element {
    const blockId = props.blockId;
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
                RpcApi.UnwatchMediaDirCommand(TabRpcClient, { path: watchedDir!, block_id: blockId }),
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
                block_id: blockId,
                extensions: ALL_MEDIA_EXTENSIONS,
            }),
        );
        unsubFileChanged = muxEventSubscribe({
            eventType: WpsEvent.MediaFileChanged,
            scope: makeORef("block", blockId),
            handler: (event) => {
                const path = (event as { data?: { path?: string } } | undefined)?.data?.path;
                if (!path) return;
                // Clear a stale "no files yet" message the moment a
                // matching file actually arrives — both render branches
                // below require !errorMsg(), so leaving it set would keep
                // showing the empty-directory message forever. Codex review.
                setErrorMsg("");
                setDisplayPath(path);
                setRevision((r) => r + 1);
                // The tab follows: its title, and what a restart shows.
                props.onPathChange(path);
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
    // something (no path text entry, per design). The tab keeps the pick, so
    // it survives a pane reload.
    const openPath = (path: string) => {
        if (props.onPathChange(path) === false) return;
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
        props.onPathChange("");
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
            blockId,
            createMediaDropHook({
                showPath: (path) => {
                    if (!props.openDropped?.(path)) openPath(path);
                },
                showFile,
                cantOpen: (name) => notifyDrop.cantOpen(name, "media pane"),
            }),
        );
        onCleanup(dispose);
        if (props.path) showPath(props.path);
    });

    // The pane gave this tab a file (one sent here while it showed none).
    createEffect(
        on(
            () => props.path,
            (path) => {
                if (path && path !== displayPath()) showPath(path);
            },
            { defer: true }
        )
    );

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
