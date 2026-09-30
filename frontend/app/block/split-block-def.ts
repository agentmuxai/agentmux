// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

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
    return declared ? declared() : fallback();
}
