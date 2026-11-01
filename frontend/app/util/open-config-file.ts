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

/** The config files that sit beside each other in the config folder. */
export type ConfigFile = "settings.json" | "widgets.json";

/** What a new widgets.json starts as: srv reads an empty object as no widgets of the user's own. */
const EMPTY_FILE = "{}\n";

/** `name` in the folder `siblingPath` is in, with the folder's own separator. */
export function siblingOf(siblingPath: string, name: string): string {
    const cut = Math.max(siblingPath.lastIndexOf("/"), siblingPath.lastIndexOf("\\"));
    return cut < 0 ? name : siblingPath.slice(0, cut + 1) + name;
}

/** The path of `file`, which exists when this returns: settings.json is merged into the latest template first. */
export async function ensureConfigFile(file: ConfigFile): Promise<string> {
    const settingsPath = await getApi().ensureSettingsFile();
    if (file === "settings.json") return settingsPath;
    const path = siblingOf(settingsPath, file);
    const existing = await RpcApi.ReadEditorFileCommand(TabRpcClient, { path }).catch(() => null);
    if (existing == null) await RpcApi.WriteEditorFileCommand(TabRpcClient, { path, content: EMPTY_FILE });
    return path;
}

export interface OpenConfigFileOptions {
    /** The pane the person acted in, to find the tab's Editor and to place a new one beside. Default: the focused pane. */
    fromBlockId?: string;
}

/** Open `file` in the Editor. Rejects when the file can't be made or opened: the caller says so. */
export async function openConfigFileInEditor(file: ConfigFile, opts: OpenConfigFileOptions = {}): Promise<void> {
    const path = await ensureConfigFile(file);
    const from = opts.fromBlockId || getFocusedBlockId() || undefined;
    await RpcApi.PaneOpenCommand(
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
    );
}
