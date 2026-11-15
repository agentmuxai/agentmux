// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Show As" in a document tab's menu: the viewers the tab's file can be
 * shown in (text, image, hex…), the one showing it checked. It is the way
 * to another viewer for a text file, whose tab has no viewer bar.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.2 ("Reopen with…").
 */

import type { DocTab } from "@/app/doc-tabs/doc-tabs";
import type { EditorBuffer } from "@/app/store/editor-pane-state-store";
import { MENU_ICON } from "@/app/util/menu-icons";
import type { EditorViewModel } from "./editor-model";
import { isViewerId, VIEWER_LABELS, type ViewerId } from "./viewers/viewer-choice";
import { droppedFileOf } from "./viewers/viewer-tabs";

/** The menu rows: one submenu, or none when the file has one viewer (or isn't read yet). */
export function showAsMenuItems(model: Pick<EditorViewModel, "viewerTabs" | "reopenWith">, tab: DocTab<EditorBuffer>): ContextMenuItem[] {
    const current: ViewerId = isViewerId(tab.payload.viewer) ? tab.payload.viewer : "text";
    const options = model.viewerTabs.options(tab.id, current, droppedFileOf(tab.payload.filePath));
    if (options.length < 2) return [];
    return [
        {
            label: "Show As",
            icon: MENU_ICON.show,
            type: "submenu",
            // Unsaved text would be lost with its buffer: save or discard it first.
            enabled: !tab.dirty,
            submenu: options.map((viewer) => ({
                label: VIEWER_LABELS[viewer].label,
                type: "radio" as const,
                checked: viewer === current,
                click: () => model.reopenWith(tab.id, viewer),
            })),
        },
    ];
}
