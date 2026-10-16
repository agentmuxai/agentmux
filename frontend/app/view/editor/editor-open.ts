// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Open a file where the user is already looking at files: as a tab of the
 * Editor pane on screen, rather than a new pane each time — what clicking an
 * image in an agent's reply did for the Media pane, now that the Editor shows
 * every file. The pane is shown too, without taking focus.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.12.1 (M5);
 * the rules are SPEC_MEDIA_OPEN_SHOWS_THE_MEDIA_PANE_2026_10_12.md §3.1, §4.1.
 */

import { setActiveTab, workspace } from "@/app/store/global";
import { setBlockMeta } from "@/app/store/block-meta";
import { WorkspaceService } from "@/app/store/services";
import { createBlockOnModel, waitForLayoutModel } from "@/app/tab/tab-presets";
import { getObjectValue, makeORef } from "@/app/store/mos";
import { showBlockWithoutFocus } from "@/app/util/reveal-block";
import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";
import { effectiveStack } from "@/layout/lib/stackMembers";

const META_PENDING_OPEN_FILES = "editor:pending_open_files";

/** An Editor on this computer's files (not one on an SSH host, which reads
 *  its files there). */
function isLocalEditor(id: string | undefined): id is string {
    if (!id) return false;
    const meta = getObjectValue<Block>(makeORef("block", id))?.meta;
    return meta?.view === "editor" && !meta?.["connection"];
}

/**
 * The Editor pane to add a file to, among the panes of the window tab on
 * screen: the focused one if it is an Editor, else the first Editor the user
 * can already see (not minimized, the front tab of its stack), else the first
 * of any kind (the open then shows it). A background tab of a stack counts,
 * so a second Editor is not opened.
 */
export function editorPaneOnScreen(): string | null {
    const model = getLayoutModelForStaticTab();
    if (!model) return null;
    const focusedData = model.focusedNode()?.data;
    const focused = focusedData?.activeBlockId || focusedData?.blockId;
    if (isLocalEditor(focused)) return focused;
    let firstAny: string | null = null;
    for (const leaf of model.leafs()) {
        const data = leaf.data;
        if (!data) continue;
        const front = data.activeBlockId || data.blockId;
        for (const id of effectiveStack(data)) {
            if (!isLocalEditor(id)) continue;
            if (!leaf.minimized && id === front) return id; // already visible: nothing to restore
            firstAny ??= id;
        }
    }
    return firstAny;
}

export interface EditorOpenOpts {
    /** The tab follows newer files in the file's folder (an image or video
     *  from an agent's render folder). */
    followFolder?: boolean;
}

/** Add `path` as a tab of the Editor pane on screen, and show that pane.
 *  False when there is none (the caller opens a new pane). */
export async function openInEditorPaneOnScreen(path: string, opts: EditorOpenOpts = {}): Promise<boolean> {
    const blockId = editorPaneOnScreen();
    if (!blockId) return false;
    const queued = getObjectValue<Block>(makeORef("block", blockId))?.meta?.[META_PENDING_OPEN_FILES];
    const list: unknown[] = Array.isArray(queued) ? queued : [];
    const entry = opts.followFolder ? { path, follow_folder: true } : path;
    await setBlockMeta(blockId, { [META_PENDING_OPEN_FILES]: [...list, entry] } as MetaType);
    // The Editor shows the pane itself when it opens the batch; this covers a
    // pane that isn't mounted yet. No window-tab switch, no focus move.
    await showBlockWithoutFocus(blockId).catch(() => false);
    return true;
}

/** Open a file in a new editor pane inside a brand-new, otherwise-empty
 *  app tab. Deliberately bypasses createTab() in store/global.ts, which
 *  always layers on the agent/sysinfo/swarm default preset — wrong here,
 *  the user asked for THIS file, not a fresh default workspace.
 *
 *  Three things every other CreateTab caller in this codebase gets for
 *  free via applyTabPreset() (tab-presets.ts), which this method can't
 *  call directly since it always adds the default widget set — so each
 *  is replicated individually here:
 *
 *  1. waitForLayoutModel(tabId) before touching the new tab at all. The
 *     tab's MuxObj + LayoutState propagate via subscription some time
 *     after CreateTab returns, not synchronously with it.
 *  2. createBlockOnModel(...) — NOT the pane.open RPC used for
 *     openToTheSide above. Confirmed live: pane.open against a freshly
 *     created tab_id succeeds server-side with zero errors ("block
 *     created + layout updated" in the srv log) and the block STILL
 *     never renders — the tab's client-side layoutModel, even once
 *     waitForLayoutModel() confirms the object exists, isn't yet
 *     subscribed to receive the backend's layout:update broadcast for
 *     that specific brand-new tab. createBlockOnModel goes through
 *     ObjectService.CreateBlock + layoutModel.treeReducer(...) directly
 *     — the same client-side path applyTabPreset uses for every preset
 *     widget — which sidesteps the gap entirely. The constructed
 *     BlockDef mirrors exactly what the backend's build_pane_meta
 *     (crates/srv/src/server/app_api/pane.rs) would have produced for
 *     `pane.open { view: "editor", file }` on the openToTheSide path.
 *  3. Explicit setActiveTab() AFTER the block exists. Belt-and-suspenders
 *     alongside CreateTab's own `activate=true` (3rd arg below): the
 *     reducer itself only auto-activates a workspace's very FIRST tab
 *     ever (see create_tab_second_tab_does_not_steal_active in
 *     reducer.rs), but the service layer already compensates —
 *     `crates/srv/src/server/service/workspace.rs`'s CreateTab handler
 *     dispatches a follow-up SetActiveTab whenever `activate=true` and
 *     the reducer didn't auto-activate, so this call is redundant in
 *     practice, not a workaround for a live bug (#2155's "activate arg
 *     silently dropped" half was already fixed server-side by the time
 *     it was filed). Kept anyway: harmless, and it's still what makes
 *     the destination tab already have its content when the switch's
 *     reveal-gate opens — avoids a flash of an empty tab. */
export async function openFileInNewTab(filePath: string): Promise<void> {
    const ws = workspace();
    if (!ws) return;
    try {
        const tabId = await WorkspaceService.CreateTab(ws.oid, "", true, false);
        const layoutModel = await waitForLayoutModel(tabId);
        if (!layoutModel) return; // Tab never propagated — nothing safe to do.
        const isMarkdown = filePath.toLowerCase().endsWith(".md");
        await createBlockOnModel(
            tabId,
            layoutModel,
            {
                meta: {
                    view: "editor",
                    file: filePath,
                    ...(isMarkdown ? { "editor:tree_expanded": false } : {}),
                },
            },
            null,
            null,
        );
        await setActiveTab(tabId);
    } catch {
        // Fail silently, consistent with the file tree's other pane.open callers.
    }
}
