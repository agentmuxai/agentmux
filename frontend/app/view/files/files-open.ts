// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Opening a file from the Files pane by its kind (§5.4): most files in an
 * Editor pane beside the Files pane — text and code, and since the Media pane
 * was folded into the Editor also images, video, audio, PDFs, archives and
 * other binaries, which the Editor shows in their own viewer or its info
 * card. Office documents and design files go to the OS's default app. A
 * program asks first.
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.4,
 * SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.12.1.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { openInEditorPaneOnScreen } from "@/app/view/editor/editor-open";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";
import { addWidgetAsPaneTab, closeBlockInStack, effectiveStack } from "@/layout/lib/layoutStack";
import { TabRpcClient } from "@/app/store/rpc-util";
import { errorText, type FilesModel } from "./files-model";
import { extensionOf } from "./files-sort";

export type OpenTarget = "editor" | "os" | "program";

/** Documents the system's own apps edit: the Editor only lists their facts,
 *  so they open there. Everything else opens in the Editor. */
const OS_APP = new Set(
    (
        "doc docx xls xlsx ppt pptx odt ods odp pages numbers key " +
        "psd ai sketch fig xcf blend fbx glb gltf stl"
    ).split(" ")
);

/** Files that run something when the OS opens them. */
const PROGRAM = new Set("exe msi com scr lnk app pkg appx msix url reg".split(" "));

export function openTargetOf(name: string): OpenTarget {
    const ext = extensionOf(name);
    if (PROGRAM.has(ext)) return "program";
    if (OS_APP.has(ext)) return "os";
    return "editor";
}

/** Opens `path` in the Editor: a tab of the Editor pane on screen when there
 *  is one, else a new Editor pane to the right of the Files pane. A file on a
 *  host opens in an editor on that host. */
export async function openInPane(view: "editor", path: string, besideBlockId: string, connection?: string, confined?: boolean): Promise<void> {
    // A confined pane opens through srv, which checks the path against its folder.
    if (!connection && !confined && (await openInEditorPaneOnScreen(path))) return;
    await RpcApi.PaneOpenCommand(
        TabRpcClient,
        {
            view,
            file: path,
            split_direction: "right",
            split_reference_block_id: besideBlockId,
            // An editor on the host the file is on (editor-model's connection()).
            ...(connection ? { connection } : {}),
        },
        {}
    );
}

/** A file of `model`'s pane in an Editor pane beside it (a host's file in an editor on that host). */
export function openInEditor(model: FilesModel, path: string): Promise<void> {
    return openInPane("editor", path, model.blockId, model.connection() || undefined, !!model.root());
}

/** A file of `model`'s pane with the system's default app; not from a confined pane. */
export function openEntryWithOs(model: FilesModel, name: string): void {
    if (model.refuseInRoot("Opening with another program")) return;
    void openWithOs(model.pathOf(name), model.blockId).catch((err) =>
        model.setStatus({ text: `Couldn't open ${name}: ${errorText(err)}`, tone: "error" })
    );
}

export async function openWithOs(path: string, blockId?: string): Promise<void> {
    await RpcApi.FsOpenCommand(TabRpcClient, { path, block_id: blockId });
}

export async function revealInOs(path: string, blockId?: string): Promise<void> {
    await RpcApi.FsRevealCommand(TabRpcClient, { path, block_id: blockId });
}

/** Opens a terminal in `dir`, to the right of the Files pane; on
 *  `connection` (an SSH host) when given, as the folder is there. */
export async function openTerminalHere(dir: string, besideBlockId: string, connection?: string): Promise<void> {
    await RpcApi.PaneOpenCommand(
        TabRpcClient,
        {
            view: "term",
            cwd: dir,
            split_direction: "right",
            split_reference_block_id: besideBlockId,
            ...(connection ? { connection } : {}),
        },
        {}
    );
}

// ── Pane tabs (several Files tabs in one pane) ────────────────────────────
//
// Files uses the app's pane tabs rather than tabs of its own: each tab is a
// whole Files block with its own folder, history and selection, and gets
// reordering, dragging between panes, tear-off and layout persistence from
// the pane-tab system (SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md).

/** The pane (leaf) holding `blockId` in the window tab on screen. */
function paneOf(blockId: string): { model: LayoutModel; nodeId: string } | null {
    const model = getLayoutModelForStaticTab();
    const node = model?.getNodeByBlockId(blockId);
    return model && node ? { model, nodeId: node.id } : null;
}

/** A new Files tab on `dir`, beside `fromBlockId` in its pane; on
 *  `connection` (an SSH host) when given. */
export async function openFolderInNewTab(fromBlockId: string, dir: string, connection?: string): Promise<void> {
    const pane = paneOf(fromBlockId);
    if (!pane) throw new Error("This pane isn't in the window tab on screen.");
    await addWidgetAsPaneTab(pane.model, pane.nodeId, {
        meta: { view: "files", "files:path": dir, ...(connection ? { connection } : {}) },
    });
}

/** Close `blockId`'s pane tab, but never the pane itself: false when it is
 *  the pane's only tab (the pane's own × closes that). */
export async function closeOwnTab(blockId: string): Promise<boolean> {
    const pane = paneOf(blockId);
    if (!pane) return false;
    const node = pane.model.getNodeByBlockId(blockId);
    if (!node?.data || effectiveStack(node.data).length <= 1) return false;
    await closeBlockInStack(pane.model, pane.nodeId, blockId);
    return true;
}
