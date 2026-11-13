// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { EditorViewModel } from "./editor-model";
import type { CmHandle } from "./editor-cm-handle";
import { chromeZoomAtom } from "@/app/store/chrome-zoom";
import { isOverChrome } from "@/app/store/chrome-zoom-target";
import { usePaneZoomWheel } from "@/app/store/zoom";
import { useZoomAnchor } from "@/app/store/zoom-anchor";
import { editorZoomAnchor } from "./editor-zoom-anchor";
import { contentZoomOf, hasContentZoom } from "./viewers/content-zoom";

/**
 * A viewer that zooms its own content (a PDF) takes the pane's zoom, and the Editor's chrome around
 * it follows chrome zoom, as the browser pane's bar does (SPEC_ZOOM_ANCHOR_AND_AUDIT §10). Text
 * takes it on the whole pane, as CSS `zoom` on .editor-view.
 */
export function useEditorZoom(model: EditorViewModel, cm: CmHandle, root: () => HTMLDivElement | undefined) {
    const contentZoomed = () => hasContentZoom(model.activeViewer());
    const rootZoom = () => (contentZoomed() ? chromeZoomAtom() : model.zoomAtom());
    // Ctrl+scroll zoom, before CodeMirror's own wheel handling. Over the chrome of a content-zoomed
    // viewer it is left to the window's handler, which steps chrome zoom.
    usePaneZoomWheel(root, () => model.blockId, undefined, (ev) => !(contentZoomed() && isOverChrome(ev.target as HTMLElement)));
    // What is under the pointer stays under it across a zoom: CodeMirror's scroller moves by how far
    // that character moved (SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md §4.3); a viewer's content by its own.
    const textAnchor = editorZoomAnchor({ view: () => cm.view, zoom: model.zoomAtom });
    const contentHandle = () => (contentZoomed() ? contentZoomOf(model.blockId) : undefined);
    useZoomAnchor({
        blockId: model.blockId,
        zoom: model.zoomAtom,
        scroller: () => (contentHandle() ? contentHandle()!.scroller() : cm.view?.scrollDOM),
        following: () => false,
        adapter: {
            capture: (point) => {
                const handle = contentHandle();
                const token = handle ? handle.adapter.capture(point) : textAnchor.capture(point);
                return token == null ? null : { handle, token };
            },
            restore: ({ handle, token }) => (handle ? handle.adapter.restore(token) : textAnchor.restore(token as never)),
        },
    });
    return { contentZoomed, rootZoom };
}

/**
 * Drags until release: tracks global mousemove so the user can drag past the handle without losing
 * the grip. `onMove` updates live; `onEnd` persists once on mouseup.
 */
function dragUntilRelease(e: MouseEvent, cursor: string, onMove: (ev: MouseEvent) => void, onEnd: () => void): void {
    e.preventDefault();
    const onUp = () => {
        document.removeEventListener("mousemove", onMove);
        document.removeEventListener("mouseup", onUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        onEnd();
    };
    document.body.style.cursor = cursor;
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
}

/**
 * The width the tree column is drawn at, in the pane's own CSS pixels. In a narrow pane the column
 * shrinks below the stored width (`flex: 0 1 <width>px`), and a drag starts from where the divider
 * is. `stored` stands in when the column has no box to measure (not laid out).
 */
function renderedTreeWidth(handle: Element | null, stored: number, zoom: number): number {
    const column = handle?.previousElementSibling;
    const drawn = column ? column.getBoundingClientRect().width / zoom : 0;
    return drawn > 0 ? drawn : stored;
}

/** The tree column's resize handle: live width while dragging, saved once on release. */
export function treeResizeHandler(model: EditorViewModel, rootZoom: () => number) {
    return (e: MouseEvent) => {
        const startX = e.clientX;
        const startWidth = renderedTreeWidth(e.currentTarget as Element | null, model.treeWidthAtom(), rootZoom());
        // document mousemove coords are in viewport CSS pixels; divide by zoom so the delta
        // maps correctly to local CSS pixels inside the zoomed .editor-view element.
        dragUntilRelease(e, "col-resize", (ev) => model.setTreeWidth(startWidth + (ev.clientX - startX) / rootZoom()), () => void model.commitTreeWidth());
    };
}

/** The preview panel's resize handle: drag up = taller, drag down = shorter. */
export function previewResizeHandler(model: EditorViewModel, rootZoom: () => number) {
    return (e: MouseEvent) => {
        const startY = e.clientY;
        const startH = model.previewHeightAtom();
        dragUntilRelease(e, "row-resize", (ev) => model.setPreviewHeight(startH + (startY - ev.clientY) / rootZoom()), () => void model.commitPreviewHeight());
    };
}
