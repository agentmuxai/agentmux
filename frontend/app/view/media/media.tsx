// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Media pane — view image, video and audio files from local disk, each file
// a document tab (media-pane.tsx, SPEC_DOCUMENT_TABS_2026_10_02.md §6.3). A
// file is picked via a native file dialog (no path text entry), dropped, or
// sent here from elsewhere in the app; its containing directory is then
// watched, so a newer matching file landing there (a fresh ComfyUI render,
// for example) replaces it on screen automatically (media-view.tsx).
//
// Spec: docs/specs/SPEC_MEDIA_PANE_2026_07_26.md

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { basenameOf, dirnameOf, extOf } from "@/app/element/local-media";
import { MediaPane, MediaPaneModel, META_PATH } from "./media-pane";

export { basenameOf, dirnameOf, extOf };
export { createMediaDropHook, mediaDropVerdict, type MediaDropActions } from "./media-drop";

/** The pane's title from its meta: the file's basename, or "Media" before
 *  one is picked. */
export function mediaTitle(meta: MetaType | undefined): string {
    const path = meta?.[META_PATH];
    return typeof path === "string" && path.length > 0 ? basenameOf(path) : "Media";
}

/**
 * Media as a native pane tab (Pane Tab contract Phase 2c): its files are
 * document tabs inside it, and the file in front titles the pane.
 */
export const mediaPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "media",
    label: "Media",
    icon: "photo-film",
    defaultHue: 120,
    create: (ctx) => {
        const pane = new MediaPaneModel(ctx);
        return {
            component: () => <MediaPane pane={pane} ctx={ctx} />,
            liveTitle: () => ({ text: pane.title() }),
            dispose: () => pane.dispose(),
        };
    },
};
