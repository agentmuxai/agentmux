// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a Shortcuts pane's tiles start from: the pane that was split to open it
 * (docs/specs/SPEC_SHORTCUTS_PANE_FOR_SPLITS_2026_10_28.md §2.4). The split puts that pane's
 * block id in the new pane's meta, and a Terminal or Hangar tile chosen there starts in the same
 * directory and on the same connection, as the copy of the pane used to.
 */

/** The Shortcuts pane's meta key naming the block that was split. */
export const SHORTCUTS_FROM_META = "shortcuts:from";

/** The directory key each tile's view starts in. */
const DIRECTORY_KEY: Record<string, string> = { term: "cmd:cwd", files: "files:path" };

/**
 * `def`, with the directory and the connection of `source` (the meta of the pane that was split)
 * added when `def` is a Terminal or Hangar tile and says nothing of its own. Local connections are
 * not carried: a "local" connection shows the connection overlay. Pure.
 */
export function blockDefFromSource(def: BlockDef, source: Record<string, unknown> | undefined): BlockDef {
    const view = def.meta?.["view"] as string | undefined;
    const directoryKey = view ? DIRECTORY_KEY[view] : undefined;
    if (!source || !directoryKey) return def;
    const inherited: Record<string, unknown> = {};
    const directory = source["cmd:cwd"] ?? source["files:path"];
    if (typeof directory === "string" && directory) inherited[directoryKey] = directory;
    const connection = source["connection"];
    if (typeof connection === "string" && connection && connection !== "local") inherited["connection"] = connection;
    // What the tile says itself wins: only the keys it leaves out are taken from the source.
    for (const key of Object.keys(inherited)) if (def.meta && key in def.meta) delete inherited[key];
    if (Object.keys(inherited).length === 0) return def;
    return { ...def, meta: { ...inherited, ...def.meta } };
}
