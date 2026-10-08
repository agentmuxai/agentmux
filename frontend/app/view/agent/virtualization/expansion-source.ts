// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `currentExpansion` — the layout slice's view of a row's open state
 * (Phase 1 of SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02).
 *
 * The rule itself lives in `disclosure.ts` (`rowDisclosure`), shared with the
 * rows that render and the estimator (SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26).
 * This file only maps it onto the slice's `Expansion` shape. Before that, this
 * function kept its own copy of the rule and drifted from what rendered: a
 * canceled tool held open on completion rendered closed but was reported open
 * here, and a canceled thought expanded by the user was invisible to layout.
 */

import type { Expansion } from "@/app/store/agent-pane-layout/types";
import type { DocumentNode, DocumentState } from "../types";
import { rowDisclosureIn } from "./disclosure";

/** The only `documentState` the mapping depends on — the collapse/pin sets plus
 *  the scroll-driven `heldOpenNodes` hold. Narrowed from the full
 *  `DocumentState` so the dependency is explicit (a full `DocumentState` still
 *  satisfies it at the call site). */
export type ExpansionInputs = Pick<
    DocumentState,
    "collapsedNodes" | "pinnedNodes" | "heldOpenNodes"
>;

/** The layout slice's view of `rowDisclosure()` (disclosure.ts). */
export function currentExpansion(node: DocumentNode, state: ExpansionInputs): Expansion {
    const d = rowDisclosureIn(node, state);
    return d.open ? { open: true, via: d.via } : { open: false };
}
