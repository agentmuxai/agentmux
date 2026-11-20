// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Reveal in File Explorer" (Finder, Open Containing Folder) on a document tab's menu: shows the tab's file in
 * the computer's own file manager. docs/specs/SPEC_EDITOR_TAB_REVEAL_IN_FILE_MANAGER_2026_11_17.md §3.1, §3.2.
 */

import type { DocTab } from "@/app/doc-tabs/doc-tabs";
import type { EditorBuffer } from "@/app/store/editor-pane-state-store";
import { revealInFileManagerLabel } from "@/app/util/file-manager-label";
import { MENU_ICON } from "@/app/util/menu-icons";
import type { EditorViewModel } from "./editor-model";
import { isDroppedPath } from "./viewers/viewer-tabs";

/**
 * The menu row, or none. An Untitled tab has no file on disk yet, and a dropped file has no path on disk, so
 * neither gets a row. A tab on an SSH host gets the row disabled and says why: the file is not on this
 * computer, and a row that vanished would leave the person wondering where it went.
 */
export function revealMenuItems(model: Pick<EditorViewModel, "connection" | "revealInExplorer">, tab: DocTab<EditorBuffer>): ContextMenuItem[] {
    const { filePath, isScratch } = tab.payload;
    if (isScratch || !filePath || isDroppedPath(filePath)) return [];
    const host = model.connection();
    return [
        {
            label: revealInFileManagerLabel(),
            icon: MENU_ICON.reveal,
            enabled: !host,
            sublabel: host ? `The file is on ${host}` : undefined,
            click: () => void model.revealInExplorer(filePath),
        },
    ];
}
