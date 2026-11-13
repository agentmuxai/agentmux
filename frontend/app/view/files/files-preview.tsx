// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's preview panel (spec §6.6): the selected file, read-only.
 * Code and text are highlighted, Markdown rendered, images shown; anything
 * else gets its facts and an Open button. One file at a time: a preview
 * starts after a short pause in the selection and is abandoned the moment
 * the selection moves on, and text reads only its first 256 KB. What is read
 * and when is in files-preview-state.ts, the markup in files-preview-parts.tsx.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §6.6.
 */

import type { JSX } from "solid-js";
import { PreviewBody } from "./files-preview-parts";
import { createFilesPreview, type FilesPreviewProps } from "./files-preview-state";

export type { Preview } from "./files-preview-state";

export function FilesPreview(props: FilesPreviewProps): JSX.Element {
    const preview = createFilesPreview(props);
    return (
        <aside class="files-preview" aria-label="Preview">
            <PreviewBody preview={preview} onOpen={props.onOpen} onOpenWithOs={props.onOpenWithOs} />
        </aside>
    );
}
