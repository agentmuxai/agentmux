// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Editor copies from a tab that shows media: its right-click Copy and Copy path (the pane menu's Copy only
 * has a text selection to work with) and the same Copy for the image viewer's Ctrl+C / Cmd+C. The copying itself
 * is element/media-copy.ts, shared with an image in an agent's reply and the media viewer.
 */

import { inlineMediaKind } from "@/app/element/local-media";
import { copyMediaWithNote, mediaCopyMenuItems, type MediaKind, type MediaSource } from "@/app/element/media-copy";
import type { EditorViewModel } from "./editor-model";
import { droppedFileOf, isDroppedPath } from "./viewers/viewer-tabs";

type Source = Pick<EditorViewModel, "activeTabAtom" | "activeViewer" | "viewerTabs" | "blockId">;

/** Which sort of media a viewer shows (the audio and video viewer tells them by the probe's type, else the file's extension). */
function kindOf(viewer: string, mime: string, path: string): MediaKind | null {
    if (viewer === "image") return "image";
    if (viewer === "pdf") return "pdf";
    if (viewer !== "media") return null;
    if (mime.startsWith("video/")) return "video";
    if (mime.startsWith("audio/")) return "audio";
    return inlineMediaKind(path) === "audio" ? "audio" : "video";
}

/** The media the Editor's front tab shows, or null when the tab isn't a media viewer's. */
export function editorMediaSource(model: Source): MediaSource | null {
    const tab = model.activeTabAtom();
    if (!tab) return null;
    const path = tab.filePath ?? "";
    const dropped = isDroppedPath(path);
    const file = dropped ? droppedFileOf(path) : undefined;
    const mime = model.viewerTabs.probe(tab.id)?.mime || file?.type || "";
    const kind = kindOf(model.activeViewer(), mime, path);
    if (!kind) return null;
    return {
        kind,
        mime,
        path: dropped ? undefined : path,
        bytes: file ? async () => file : undefined,
        element: () => document.querySelector<HTMLVideoElement>(`[data-blockid="${model.blockId}"] video`),
    };
}

/** Copy the front tab's picture (the image viewer's Ctrl+C / Cmd+C) and say how it went. */
export function copyEditorMedia(model: Source): void {
    const source = editorMediaSource(model);
    if (source) void copyMediaWithNote(source);
}

/** The pane menu's Copy for the Editor: the media's own items, or none (the plain Copy) for a text tab. */
export function editorCopyMenuItems(model: Source, selection: string): ContextMenuItem[] {
    const source = editorMediaSource(model);
    return source ? mediaCopyMenuItems(source, selection) : [];
}
