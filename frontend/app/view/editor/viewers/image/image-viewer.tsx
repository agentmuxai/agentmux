// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's image viewer: moved from the retired Media pane with
 * its zoom and pan — the wheel zooms about the cursor (Ctrl+wheel
 * stays pane zoom), a drag pans, `+ - 0 1` and the arrows, pixel squares
 * past 2×, the zoom kept across a same-size re-render. Added: a toolbar (fit,
 * 100%, background), the image's size and type, and `[` / `]` for the
 * previous and next image in the folder. SVG is only ever an `<img>` from a
 * blob, so its scripts never run.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.2, §4.13.
 * A click zooms a step in about the pointer, Ctrl+click (⌥+click on macOS)
 * or Alt+click a step out; double-click no longer toggles fit.
 * docs/specs/SPEC_EDITOR_TREE_KEYBOARD_ICONS_AND_CLICK_ZOOM_2026_10_16.md §7.
 */

import { type JSX } from "solid-js";
import { ViewerBar } from "../viewer-bar";
import { createImageViewer, type ImageViewerProps } from "./image-viewer-state";
import { ImageStage, ImageTools } from "./image-viewer-parts";
import "./image-viewer.scss";

export { defaultBackground, type ImageBackground, type ImageViewerProps } from "./image-viewer-state";

export function ImageViewer(props: ImageViewerProps): JSX.Element {
    const t = createImageViewer(props);
    return (
        <div class="editor-viewer image-viewer">
            <ViewerBar {...props.bar} facts={t.facts()} start={<ImageTools t={t} />} />
            <ImageStage t={t} source={props} />
        </div>
    );
}
