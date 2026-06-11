// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `currentExpansion` — the single, pure mapping from a `DocumentNode` (+ the
 * `documentState` collapse/pin sets) to the agent-pane-layout slice's
 * `Expansion` value.
 *
 * Phase 1 of SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02. Today the "is this row
 * open in flow" decision is scattered across `renderers.ts` estimators,
 * `ToolBlock`, `UserMessageBlock`, `MarkdownBlock`, and `AgentDocumentView`
 * toggles — each kind keying off a different signal (`pinnedNodes` for tools /
 * startup user-messages, `collapsedNodes` for agent-messages, `node.collapsed`
 * for sections, kind-default for the rest). This function is the one place that
 * encodes that table, so the layout slice can be driven from a single source.
 *
 * It mirrors the height decisions in `renderers.ts` with ONE deliberate
 * improvement: `estimateTool` keys only off `pinnedNodes`, so a running tool
 * estimates as collapsed even though it renders expanded (a latent estimator
 * gap that measurement has to paper over). Here a `running` /
 * `pending_approval` tool maps to `{ open: true, via: "auto" }`, matching what
 * actually renders.
 *
 * NOT captured here (component-local transients, not derivable from a node /
 * `documentState`, that Phase 2 moves into the slice via commands):
 *   - a tool's 3 s post-completion hold (`ToolBlock.postCompletionHold`)
 *   - a user's expand CLICK on a canceled-thinking block (`MarkdownBlock.expanded`).
 *     NOTE the canceled block's collapsed *default* IS captured below (it's
 *     `metadata.canceled`); only the click that overrides it is component-local.
 * Until then these are handled by the store-layer hold timer / a click-time
 * `UserExpanded` dispatch in the wiring layer, not by this pure mapper.
 */

import type { Expansion } from "@/app/store/agent-pane-layout/types";
import type { DocumentNode, DocumentState } from "../types";

/** The only `documentState` the mapping depends on — the two collapse/pin
 *  sets. Narrowed from the full `DocumentState` so the dependency is explicit
 *  (a full `DocumentState` still satisfies it at the call site). */
export type ExpansionInputs = Pick<DocumentState, "collapsedNodes" | "pinnedNodes">;

const OPEN_DEFAULT: Expansion = { open: true, via: "default" };
const CLOSED: Expansion = { open: false };

export function currentExpansion(
    node: DocumentNode,
    state: ExpansionInputs,
): Expansion {
    switch (node.type) {
        case "tool":
            // pin wins; otherwise a live tool is auto-expanded.
            if (state.pinnedNodes.has(node.id)) return { open: true, via: "pin" };
            if (node.status === "running" || node.status === "pending_approval") {
                return { open: true, via: "auto" };
            }
            return CLOSED;

        case "agent_message":
            // Default OPEN; collapsed only when the user collapsed it.
            return state.collapsedNodes.has(node.id) ? CLOSED : OPEN_DEFAULT;

        case "user_message":
            // Only startup payloads collapse, and they key off `pinnedNodes`
            // (SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24 §D).
            // Normal typed input is always in flow.
            if (node.isStartup) {
                return state.pinnedNodes.has(node.id) ? { open: true, via: "pin" } : CLOSED;
            }
            return OPEN_DEFAULT;

        case "section":
            // Section height is fixed, but track the flag for fidelity.
            return node.collapsed ? CLOSED : OPEN_DEFAULT;

        case "markdown":
            // Canceled-thinking renders COLLAPSED by default — and that default
            // IS derivable here (`metadata.canceled`); only the user's expand
            // click is component-local (Phase 2). Capturing it improves on
            // `estimateMarkdown`, which ignores the collapse (the same gap this
            // mapper fixes for running tools).
            return node.metadata?.canceled ? CLOSED : OPEN_DEFAULT;

        case "subagent_link":
            return OPEN_DEFAULT;

        case "shell":
            // Mirrors tool: pin wins; a running shell is auto-expanded.
            if (state.pinnedNodes.has(node.id)) return { open: true, via: "pin" };
            if (node.status === "running") return { open: true, via: "auto" };
            return CLOSED;
    }
}
