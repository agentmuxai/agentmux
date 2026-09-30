// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether an agent-pane row is open, and how the user toggles it: the one
 * rule every row, the virtualizer and the estimator share
 * (SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26.md).
 *
 * Before this, the same question was answered by each component's own rule,
 * by `currentExpansion()` for the layout slice, and again by `estimateNode()`
 * for the perf probe, and they drifted: a canceled tool that had been
 * held open on completion rendered closed (ToolBlock skips the hold for a
 * dismissed tool) while the layout slice was told it was open.
 *
 * The rule takes a node and its three flags. Components pass the flags they
 * already receive as props (`pinned`, `heldOpen`, `collapsed`); the
 * virtualizer derives them from `documentState` with `rowFlags()`. Either way
 * the answer comes from `rowDisclosure()`.
 */

import { isContentFirstTool } from "../tool-meta/tool-descriptors";
import { TOOL_STATUS } from "../tool-meta/tool-status";
import type { DocumentNode, DocumentState } from "../types";

/** The user's state for one row, as stored in `documentState`. */
export interface RowFlags {
    /** In `pinnedNodes`: the user opened a closed-by-default row. */
    pinned?: boolean;
    /** In `collapsedNodes`: the user closed an open-by-default row. */
    collapsed?: boolean;
    /** In `expandedTools`: a tool held open after finishing on screen. */
    held?: boolean;
}

export interface Disclosure {
    open: boolean;
    /** Why it's open (the layout slice's `Expansion.via`); "default" when closed. */
    via: "default" | "auto" | "pin";
    /** What a header click or the `e` key flips: `pinnedNodes` ("pin"),
     *  `collapsedNodes` ("collapse"), or nothing. */
    toggle: "pin" | "collapse" | null;
}

export type DisclosureInputs = Pick<DocumentState, "collapsedNodes" | "pinnedNodes" | "expandedTools">;

export function rowFlags(id: string, state: DisclosureInputs): RowFlags {
    return {
        pinned: state.pinnedNodes.has(id),
        collapsed: state.collapsedNodes.has(id),
        held: state.expandedTools.has(id),
    };
}

const OPEN = (via: Disclosure["via"], toggle: Disclosure["toggle"]): Disclosure => ({ open: true, via, toggle });
const CLOSED = (toggle: Disclosure["toggle"]): Disclosure => ({ open: false, via: "default", toggle });
/** Open by default; the user can collapse it. */
const collapsible = (f: RowFlags): Disclosure => (f.collapsed ? CLOSED("collapse") : OPEN("default", "collapse"));
/** Closed by default; the user can pin it open. */
const pinnable = (f: RowFlags): Disclosure => (f.pinned ? OPEN("pin", "pin") : CLOSED("pin"));
const FIXED_OPEN: Disclosure = { open: true, via: "default", toggle: null };

export function rowDisclosure(node: DocumentNode, f: RowFlags): Disclosure {
    switch (node.type) {
        case "tool": {
            // A finished content-first tool (WebSearch) reads like a message.
            if (isContentFirstTool(node)) return collapsible(f);
            if (f.pinned) return OPEN("pin", "pin");
            const status = TOOL_STATUS[node.status];
            if (status?.active) return OPEN("auto", "pin");
            // A dismissed tool (denied / canceled) never holds open, even if
            // it entered `expandedTools` on its active → inactive transition.
            if (f.held && !status?.dismissed) return OPEN("auto", "pin");
            return CLOSED("pin");
        }

        case "agent_message":
        case "jekt_message":
            // A message must be visible by default, not opt-in.
            return collapsible(f);

        case "user_message":
            // Only the startup payload collapses
            // (SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24 §D).
            return node.isStartup ? pinnable(f) : FIXED_OPEN;

        case "context_delivery":
            // Title, size and excerpt by default; pin to read every item
            // (SPEC_CONTEXT_DELIVERY_2026_09_30 §3.2).
            return pinnable(f);

        case "shell":
            // Pin-to-expand only: unlike tools, a running shell stays
            // collapsed by default (its spec §11).
            return pinnable(f);

        case "markdown":
            // Canceled thinking is collapsed by default; the user can open it.
            return node.metadata?.canceled ? pinnable(f) : FIXED_OPEN;

        case "agent_error":
        case "context_compacted":
        case "compaction_started":
        case "session_outcome":
        case "day_divider":
        case "history_link":
        case "resume_preflight":
        case "memory_reinjection":
        case "ambient_narration":
            // Fixed-height or plain in-flow rows; never collapsible.
            return FIXED_OPEN;

        default: {
            // A new DocumentNode type with no case above is a compile error
            // here, not a runtime `undefined` (noImplicitReturns is off).
            const _exhaustive: never = node;
            void _exhaustive;
            return FIXED_OPEN;
        }
    }
}

/** The disclosure of a row from the pane's `documentState`. */
export function rowDisclosureIn(node: DocumentNode, state: DisclosureInputs): Disclosure {
    return rowDisclosure(node, rowFlags(node.id, state));
}

/** The state after the user toggles `node` (a no-op for fixed rows). */
export function toggleDisclosure<S extends DisclosureInputs>(node: DocumentNode, state: S): S {
    const t = rowDisclosureIn(node, state).toggle;
    if (t === null) return state;
    const key = t === "pin" ? "pinnedNodes" : "collapsedNodes";
    const next = new Set(state[key]);
    if (next.has(node.id)) next.delete(node.id);
    else next.add(node.id);
    return { ...state, [key]: next };
}
