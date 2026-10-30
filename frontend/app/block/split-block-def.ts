// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { SHORTCUTS_FROM_META } from "@/app/view/shortcuts/shortcuts-from";
import { paneTabCapability } from "./pane-tab-registry";

/**
 * The block a split of `source` creates. A view that declares `splitBlockDef`
 * gets exactly that (agent: a fresh picker); any other view gets `fallback()`,
 * the caller's own default: a copy of the pane for the pane menu, a terminal
 * (or the configured new block) for the command palette and the split
 * shortcuts. Every split path goes through this, so an agent pane splits into
 * a picker however the split was asked for.
 * SPEC_AGENT_PANE_SPLIT_OPENS_PICKER_2026_09_30.md.
 */
export function splitBlockDefFor(source: Block | undefined, fallback: () => BlockDef): BlockDef {
    const declared = paneTabCapability(source?.meta?.view, "splitBlockDef");
    return declared && source ? declared(source) : fallback();
}

/**
 * What every split of `source` creates: a Shortcuts pane, which remembers the pane it came from
 * (its Terminal and Files tiles start where that pane was). Splitting a Shortcuts pane passes on
 * the pane it came from. The caller's `fallback` is for a split with no pane to split.
 * SPEC_SHORTCUTS_PANE_FOR_SPLITS_2026_10_28.md §2.1.
 */
export function shortcutsSplitBlockDef(source: Block | undefined, fallback: () => BlockDef): BlockDef {
    if (!source?.oid) return fallback();
    const view = source.meta?.view;
    const from = view === "shortcuts" || view === "launcher" ? (source.meta?.[SHORTCUTS_FROM_META] as string | undefined) : undefined;
    return { meta: { view: "shortcuts", [SHORTCUTS_FROM_META]: from ?? source.oid } };
}
