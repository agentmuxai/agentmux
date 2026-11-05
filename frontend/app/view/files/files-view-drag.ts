// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dragging in the Files pane: the pane as a drop target, and the rows as drag sources. Split out of
 * FilesView; the hooks are called in the order they were there.
 */

import { beginPathDrag, pathDragPaths, pathDragSource, registerFileDropTarget } from "@/app/drag/file-drop";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { onCleanup, onMount } from "solid-js";
import { samePath } from "./files-path";
import type { FilesLocal } from "./files-view-state";

export function useFilesDrag(fv: FilesLocal) {
    const { model, setDropRow } = fv;

    // Files dropped on the pane go into the folder it shows: an OS drop or
    // another pane's row is copied; a row from another Files pane on the
    // same drive is moved (§8.2).
    onMount(() => {
        const dispose = registerFileDropTarget(model.blockId, {
            needsPaths: true,
            onNoPaths: () => model.setStatus({ text: "Those files have no path on disk, so they can't be copied here.", tone: "error" }),
            accept(drag) {
                if (model.phase() !== "ready") return { ok: false, reason: "Open a folder first" };
                // From this same pane only a folder row is somewhere new; the
                // verdict is asked once per drag, so the drop itself checks.
                if (pathDragSource() === model.blockId) return { ok: true, message: "Drop on a folder to move it there", icon: "fa-folder-open" };
                const what = drag.count === 1 ? (drag.names?.[0] ?? "1 item") : `${drag.count} items`;
                const inApp = pathDragPaths();
                const kind = inApp ? model.dropKind(inApp, pathDragSource() != null) : "copy";
                return kind === "move"
                    ? { ok: true, message: `Move ${what} here`, icon: "fa-right-left" }
                    : { ok: true, message: `Copy ${what} here`, icon: "fa-copy" };
            },
            async drop({ paths }) {
                // Onto a folder row: into that folder; anywhere else: here.
                const row = fv.dropRow();
                setDropRow(null);
                if (paths.length === 0) return;
                if (!row && pathDragSource() === model.blockId) return; // Already here.
                const dest = row ? model.pathOf(row) : model.path();
                // A folder dropped onto itself goes nowhere.
                const sources = paths.filter((p) => !samePath(p, dest));
                if (sources.length === 0) return;
                await model.transfer(model.dropKind(sources, pathDragSource() != null, dest), sources, dest);
            },
        });
        onCleanup(dispose);
    });

    const clearDropRow = () => setDropRow(null);
    window.addEventListener("dragend", clearDropRow, true);
    onCleanup(() => window.removeEventListener("dragend", clearDropRow, true));
    const onRowDragOver = (entry: FsEntry): void => {
        const dragged = pathDragPaths();
        // Not onto a folder that is itself being dragged.
        const self = dragged?.some((p) => samePath(p, model.pathOf(entry.name)));
        setDropRow(entry.is_dir && !self ? entry.name : null);
    };

    const onRowDragStart = (e: DragEvent, entry: FsEntry): void => {
        if (!e.dataTransfer) return;
        // A host's paths mean nothing to this computer's panes and apps; a confined folder's are not handed out.
        if (model.connection() || model.root()) {
            e.preventDefault();
            return;
        }
        if (!model.selection().names.has(entry.name)) {
            model.setSelection({ names: new Set([entry.name]), focus: entry.name, anchor: entry.name });
        }
        const paths = model.selectedEntries().map((x) => model.pathOf(x.name));
        beginPathDrag(e.dataTransfer, paths, model.blockId);
    };

    return { onRowDragOver, onRowDragStart };
}
