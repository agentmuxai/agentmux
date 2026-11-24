// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Taking a file out of AgentMux by drag: a Files row dropped on the desktop, a file manager or another app's page.
 * docs/reports/REPORT_DRAG_FILES_OUT_OF_AGENTMUX_2026_11_17.md §5.
 *
 * The host adds the real files to the operating system's drag, and only for a drag that carries a nonce it issued: the
 * app asks for one when the pointer goes down on a row (a drag starts a few moments later, and its start cannot wait for
 * the host), and puts it on the drag as one custom data entry. Nothing else about the drag changes, so a drop inside
 * AgentMux works as before. A host that cannot take files out answers null once, and nothing is asked again.
 */

import { getApi } from "@/util/app-api";

/** The drag data type the host looks for (`commands/file_drag.rs`). */
export const FILE_DRAG_MARKER = "application/x-agentmux-file-drag";

let prepared: { key: string; nonce: string } | null = null;
let supported = true;

const keyOf = (paths: string[]): string => paths.join("\n");

/** The pointer went down on a row or tile that stands for `paths` (local files): register them, so a drag that follows can carry them. */
export function prepareFileDragOut(paths: string[]): void {
    prepared = null;
    const api = getApi();
    if (!supported || !api.registerFileDrag || paths.length === 0) return;
    const key = keyOf(paths);
    api.registerFileDrag(paths).then(
        (nonce) => {
            if (nonce == null) supported = false;
            else prepared = { key, nonce };
        },
        () => {
            // A file the host won't take out (a folder, a vault file, one that can't be read): the drag stays an in-app drag.
            prepared = null;
        },
    );
}

/** The drag of `paths` is starting: it carries the nonce if one was registered for exactly these paths, and only once. */
export function markFileDragOut(dt: DataTransfer, paths: string[]): boolean {
    const ready = prepared;
    prepared = null;
    if (!ready || ready.key !== keyOf(paths)) return false;
    dt.setData(FILE_DRAG_MARKER, ready.nonce);
    return true;
}
