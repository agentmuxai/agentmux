// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a split of an editor pane creates: an empty editor with the source
// pane's settings, not a copy of its open documents. An allow-list, so a key
// added later stays with its pane unless it is named here.
// docs/specs/SPEC_EDITOR_MEDIA_SPLIT_OPENS_EMPTY_2026_10_10.md.

import { isSshConnection } from "@/app/view/term/ssh-connection";

/** The editor's per-pane settings a split carries across. Its documents
 *  (`doctabs`, `file`, `editor:line`, `editor:pending_open_files`,
 *  `editor:scratch`) are not among them. */
export const EDITOR_SPLIT_SETTINGS = [
    "editor:tree_width",
    "editor:tree_expanded",
    "editor:show_hidden",
    "editor:word_wrap",
    "editor:preview_height",
] as const;

export function editorSplitBlockDef(source: Block): BlockDef {
    const from = (source.meta ?? {}) as Record<string, unknown>;
    const meta: Record<string, unknown> = { view: "editor" };
    for (const key of EDITOR_SPLIT_SETTINGS) {
        if (from[key] !== undefined) meta[key] = from[key];
    }
    // The host its files are on, as the copy did: an editor on this computer
    // has no `connection`.
    if (isSshConnection(from["connection"])) meta["connection"] = from["connection"];
    return { meta };
}
