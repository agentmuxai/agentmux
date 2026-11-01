// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Open a config file the person edits by hand (settings.json, widgets.json) in AgentMux's own Editor,
// the file tree collapsed, instead of the operating system's default editor. One helper for every
// place that offers one: a config file opened from the UI goes through here.
//
// The file goes into an Editor pane the tab already shows, as another document tab, and only a tab with
// none gets a new pane (srv decides, `reuse_editor_pane`). A new pane's tree is collapsed: the Editor's
// default for a pane opened on a file (`tree_expanded` is left unset on purpose: srv does not reuse a
// pane when one is given).

import { getApi } from "@/app/store/app-api";
import { getFocusedBlockId } from "@/app/store/block-component-registry";
import { atoms } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { showBlockWithoutFocus } from "@/app/util/reveal-block";
import { getLayoutModelForStaticTab } from "@/layout/lib/layoutModelHooks";

/** The config files that sit beside each other in the config folder. */
export type ConfigFile = "settings.json" | "widgets.json";

/** What a new widgets.json starts as: srv reads an empty object as no widgets of the user's own. */
const EMPTY_FILE = "{}\n";

/** The folder of `path`, with its own separator at the end; empty for a bare name. */
function folderOf(path: string): string {
    const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return cut < 0 ? "" : path.slice(0, cut + 1);
}

/** `name` in the folder `siblingPath` is in, with the folder's own separator. */
export function siblingOf(siblingPath: string, name: string): string {
    return folderOf(siblingPath) + name;
}

/**
 * Whether `name` is in `folder`, by listing it: a read that fails for any reason (too big, locked,
 * no permission) must never be mistaken for a missing file, because creating over a real one erases it.
 * A listing that fails throws, and nothing is written.
 */
async function existsIn(folder: string, name: string): Promise<boolean> {
    const listing = await RpcApi.ListEditorDirCommand(TabRpcClient, { path: folder });
    // Windows and macOS file systems ignore case; a differently-cased file is the same file there.
    return listing.entries.some((e) => e.name.toLowerCase() === name.toLowerCase());
}

/** The path of `file`, which exists when this returns: settings.json is merged into the latest template first. */
export async function ensureConfigFile(file: ConfigFile): Promise<string> {
    const settingsPath = await getApi().ensureSettingsFile();
    if (file === "settings.json") return settingsPath;
    const path = siblingOf(settingsPath, file);
    if (!(await existsIn(folderOf(settingsPath) || ".", file))) {
        await RpcApi.WriteEditorFileCommand(TabRpcClient, { path, content: EMPTY_FILE });
    }
    return path;
}

export interface OpenConfigFileOptions {
    /** The pane the person acted in, to find the tab's Editor and to place a new one beside. Default: the focused pane. */
    fromBlockId?: string;
}

/** A pane of the tab on screen (not of a background tab), for srv to find the tab by when nothing has the focus. */
function anyBlockId(): string | undefined {
    const data = getLayoutModelForStaticTab()?.leafs()[0]?.data;
    return data?.activeBlockId || data?.blockId;
}

const SHOW_TRIES = 10;
const SHOW_TRY_MS = 50;

/** Show the pane the file went into, without moving focus: beside a magnified pane it would stay hidden. */
async function reveal(blockId: string | undefined): Promise<void> {
    if (!blockId) return;
    // The frontend places a new pane a moment after srv answers, so try for a short while.
    for (let i = 0; i < SHOW_TRIES; i++) {
        if (await showBlockWithoutFocus(blockId).catch(() => false)) return;
        await new Promise((r) => setTimeout(r, SHOW_TRY_MS));
    }
}

/** Open `file` in the Editor. Rejects when the file can't be made or opened: the caller says so. */
export async function openConfigFileInEditor(file: ConfigFile, opts: OpenConfigFileOptions = {}): Promise<void> {
    const path = await ensureConfigFile(file);
    // srv finds the tab, and the Editor in it, from a pane; with none to name it can only place a new one.
    const from = opts.fromBlockId || getFocusedBlockId() || anyBlockId();
    const opened = (await RpcApi.PaneOpenCommand(
        TabRpcClient,
        {
            view: "editor",
            file: path,
            reuse_editor_pane: true,
            ...(from
                ? { split_reference_block_id: from, split_direction: "right" }
                : { origin_tab_id: atoms.activeTabId() }),
        },
        {}
    )) as { block_id?: string } | undefined;
    await reveal(opened?.block_id);
}
