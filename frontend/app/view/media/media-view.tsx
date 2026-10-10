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

import { hostHas } from "@/app/host/host-caps";
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
import { Button } from "@/app/element/ui";
import "./media-view.scss";
import {
    actualSizeScale,
    FIT,
    panBy,
    showsPixels,
    toggleZoom,
    wheelFactor,
    zoomAt,
    zoomPercent,
    clampPan,
    type ZoomBox,
    type ZoomState,
} from "./media-zoom";

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
    /** Dropped bytes with no host path this tab shows (kept by the pane, so
     *  they survive a tab switch; not across a restart). */
    file?: File;
    /** Dropped bytes: the pane keeps them on a tab (this one when it shows
     *  nothing, else a new one) and returns true. */
    openDroppedFile?: (file: File) => boolean;
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

    // Image zoom and pan (media-zoom.ts): the wheel zooms about the cursor,
    // a drag pans, double-click toggles fit and actual size.
    const [zoom, setZoom] = createSignal<ZoomState>(FIT);
    const [dragging, setDragging] = createSignal(false);
    let contentRef: HTMLDivElement | undefined;
    let imgRef: HTMLImageElement | undefined;
    // The last image's own size: a newer render of the same size keeps the
    // zoom and pan (comparing renders), another size goes back to fit.
    let lastNatural = "";

    const zoomBox = (): ZoomBox | null => {
        if (!contentRef || !imgRef || !imgRef.naturalWidth) return null;
        return {
            fitWidth: imgRef.offsetWidth,
            fitHeight: imgRef.offsetHeight,
            viewWidth: contentRef.offsetWidth,
            viewHeight: contentRef.offsetHeight,
            naturalWidth: imgRef.naturalWidth,
            naturalHeight: imgRef.naturalHeight,
        };
    };
    // A point on screen, in the view's own CSS pixels from its centre (the
    // pane may be zoomed, which scales screen pixels against CSS pixels).
    const fromCentre = (clientX: number, clientY: number): { x: number; y: number } => {
        const rect = contentRef!.getBoundingClientRect();
        const k = contentRef!.offsetWidth ? rect.width / contentRef!.offsetWidth : 1;
        return {
            x: (clientX - rect.left) / k - contentRef!.offsetWidth / 2,
            y: (clientY - rect.top) / k - contentRef!.offsetHeight / 2,
        };
    };
    const screenScale = (): number => {
        const rect = contentRef?.getBoundingClientRect();
        return rect && contentRef!.offsetWidth ? rect.width / contentRef!.offsetWidth : 1;
    };
    const centre = { x: 0, y: 0 };
    const showsImage = (): boolean => !errorMsg() && kind() === "image" && !!objectUrl() && mediaReady();

    const onWheel = (e: WheelEvent) => {
        // Ctrl/Cmd+wheel zooms the pane itself (app.tsx), as everywhere.
        if (e.ctrlKey || e.metaKey || !showsImage()) return;
        const box = zoomBox();
        if (!box) return;
        e.preventDefault();
        setZoom(zoomAt(zoom(), wheelFactor(e.deltaY, e.deltaMode), fromCentre(e.clientX, e.clientY), box));
    };

    let dragFrom: { x: number; y: number } | null = null;
    const onPointerDown = (e: PointerEvent) => {
        if (e.button !== 0 || zoom().scale <= 1) return;
        e.preventDefault();
        (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
        dragFrom = { x: e.clientX, y: e.clientY };
        setDragging(true);
    };
    const onPointerMove = (e: PointerEvent) => {
        const box = zoomBox();
        if (!dragFrom || !box) return;
        const k = screenScale();
        setZoom(panBy(zoom(), (e.clientX - dragFrom.x) / k, (e.clientY - dragFrom.y) / k, box));
        dragFrom = { x: e.clientX, y: e.clientY };
    };
    const endDrag = () => {
        dragFrom = null;
        setDragging(false);
    };
    const onDoubleClick = (e: MouseEvent) => {
        const box = zoomBox();
        if (box) setZoom(toggleZoom(zoom(), fromCentre(e.clientX, e.clientY), box));
    };
    // + / - zoom about the centre, 0 fits, 1 shows actual size; zoomed in,
    // the arrow keys pan a tenth of the view.
    const onKeyDown = (e: KeyboardEvent) => {
        if (e.ctrlKey || e.metaKey || e.altKey || !showsImage()) return;
        const box = zoomBox();
        if (!box) return;
        let next: ZoomState | null = null;
        if (e.key === "+" || e.key === "=") next = zoomAt(zoom(), 1.25, centre, box);
        else if (e.key === "-" || e.key === "_") next = zoomAt(zoom(), 1 / 1.25, centre, box);
        else if (e.key === "0") next = FIT;
        else if (e.key === "1") next = zoomAt(zoom(), actualSizeScale(box) / zoom().scale, centre, box);
        else if (zoom().scale > 1 && e.key.startsWith("Arrow")) {
            const step = { x: box.viewWidth / 10, y: box.viewHeight / 10 };
            const d = { ArrowLeft: [step.x, 0], ArrowRight: [-step.x, 0], ArrowUp: [0, step.y], ArrowDown: [0, -step.y] }[e.key];
            if (d) next = panBy(zoom(), d[0], d[1], box);
        }
        if (!next) return;
        e.preventDefault();
        e.stopPropagation();
        setZoom(next);
    };
    const onImageLoad = () => {
        const natural = imgRef ? `${imgRef.naturalWidth}x${imgRef.naturalHeight}` : "";
        if (natural !== lastNatural) setZoom(FIT);
        lastNatural = natural;
        setMediaReady(true);
    };

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

    // Without native dialogs there's nothing to click: a file arrives by path or drop.
    const canPick = hostHas("nativeDialogs");
    const pickFile = async () => {
        if (!canPick) return;
        const path = await getApi()?.showOpenFileDialog?.();
        if (!path) return; // user cancelled
        openPath(path);
    };

    let shownFile: File | null = null;
    const showFile = (file: File) => {
        shownFile = file;
        stopWatching();
        fetchToken++; // drop any fetch still in flight for the previous path
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
        // A real listener, not passive, so it can keep the wheel from scrolling.
        contentRef?.addEventListener("wheel", onWheel, { passive: false });
        onCleanup(() => contentRef?.removeEventListener("wheel", onWheel));
        // A resized pane keeps the image in reach.
        if (typeof ResizeObserver !== "undefined" && contentRef) {
            const resize = new ResizeObserver(() => {
                const box = zoomBox();
                if (box && zoom().scale > 1) setZoom(clampPan(zoom(), box));
            });
            resize.observe(contentRef);
            onCleanup(() => resize.disconnect());
        }
        const dispose = registerFileDropTarget(
            blockId,
            createMediaDropHook({
                showPath: (path) => {
                    if (!props.openDropped?.(path)) openPath(path);
                },
                showFile: (file) => {
                    if (!props.openDroppedFile?.(file)) showFile(file);
                },
                cantOpen: (name) => notifyDrop.cantOpen(name, "media pane"),
            }),
        );
        onCleanup(dispose);
        if (props.file) showFile(props.file);
        else if (props.path) showPath(props.path);
    });

    // The pane put dropped bytes on this tab.
    createEffect(
        on(
            () => props.file,
            (file) => {
                if (file && file !== shownFile) showFile(file);
            },
            { defer: true }
        )
    );

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
            <div
                ref={contentRef}
                class="media-view-content flex-1"
                style={{ position: "relative", overflow: "hidden" }}
                tabIndex={-1}
                onKeyDown={onKeyDown}
            >
                <Show when={errorMsg()}>
                    <div style={emptyStateStyle} onClick={() => void pickFile()}>
                        <div>{errorMsg()}</div>
                        <Show when={canPick}>
                            <div style={explainerStyle}>Click anywhere to pick a different file.</div>
                        </Show>
                    </div>
                </Show>
                <Show when={!errorMsg() && displayPath() && !objectUrl()}>
                    <div class="flex items-center justify-center w-full h-full" style={{ color: "var(--secondary-text-color, #888)" }}>
                        Loading…
                    </div>
                </Show>
                <Show when={!errorMsg() && kind() === "image" && objectUrl()}>
                    <img
                        ref={imgRef}
                        class="media-view-media max-w-full max-h-full"
                        classList={{
                            "media-view-zoomed": zoom().scale > 1,
                            "media-view-dragging": dragging(),
                            "media-view-pixels": (() => {
                                const box = zoomBox();
                                return !!box && showsPixels(zoom(), box);
                            })(),
                        }}
                        style={{
                            "object-fit": "contain",
                            opacity: mediaReady() ? 1 : 0,
                            transition: "opacity 120ms ease",
                            position: "absolute",
                            inset: "0",
                            margin: "auto",
                            transform: `translate(${zoom().x}px, ${zoom().y}px) scale(${zoom().scale})`,
                        }}
                        src={objectUrl()}
                        draggable={zoom().scale <= 1}
                        onLoad={onImageLoad}
                        onDblClick={onDoubleClick}
                        onPointerDown={onPointerDown}
                        onPointerMove={onPointerMove}
                        onPointerUp={endDrag}
                        onPointerCancel={endDrag}
                        onError={() => setErrorMsg("Failed to display media (unsupported format or corrupt file).")}
                    />
                </Show>
                <Show when={showsImage() && zoom().scale > 1}>
                    <div class="media-view-zoom-badge">
                        <Button density="compact" title="Fit to the pane (0)" onClick={() => setZoom(FIT)}>
                            {(() => {
                                const box = zoomBox();
                                return box ? `${zoomPercent(zoom(), box)}%` : "";
                            })()}{" "}
                            · Fit
                        </Button>
                    </div>
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
                        <div style={{ "font-size": "1.05em" }}>{canPick ? "Click to load media" : "No media loaded"}</div>
                        <div style={explainerStyle}>
                            {supportedTypesText} If you pick a file from a folder your agent is
                            actively generating into, this pane updates automatically as new
                            matching files appear there — no need to reopen it.
                        </div>
                    </div>
                </Show>
                <Show when={canPick && !errorMsg() && displayPath()}>
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
