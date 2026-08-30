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
 * The tool's post-completion hold IS captured here now, via `expandedTools`: a
 * completed tool stays open while held (added on live completion, removed when
 * its row scrolls off the top), so heights track the visual. This replaced the
 * old 3 s `ToolBlock` timer — see
 * docs/specs/PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md.
 *
 * Still NOT captured here (a genuinely component-local transient):
 *   - a user's expand CLICK on a canceled-thinking block (`MarkdownBlock.expanded`).
 *     NOTE the canceled block's collapsed *default* IS captured below (it's
 *     `metadata.canceled`); only the click that overrides it is component-local,
 *     handled by a click-time dispatch in the wiring layer, not this pure mapper.
 */

import type { Expansion } from "@/app/store/agent-pane-layout/types";
import type { DocumentNode, DocumentState } from "../types";

/** The only `documentState` the mapping depends on — the collapse/pin sets plus
 *  the scroll-driven `expandedTools` hold. Narrowed from the full
 *  `DocumentState` so the dependency is explicit (a full `DocumentState` still
 *  satisfies it at the call site). */
export type ExpansionInputs = Pick<
    DocumentState,
    "collapsedNodes" | "pinnedNodes" | "expandedTools"
>;

const OPEN_DEFAULT: Expansion = { open: true, via: "default" };
const CLOSED: Expansion = { open: false };

export function currentExpansion(
    node: DocumentNode,
    state: ExpansionInputs,
): Expansion {
    switch (node.type) {
        case "tool":
            // pin wins; otherwise a live tool is auto-expanded. A completed tool
            // stays open while held in `expandedTools` (added on live completion,
            // removed once it scrolls off the top — the scroll-driven replacement
            // for the old 3 s post-completion timer).
            if (state.pinnedNodes.has(node.id)) return { open: true, via: "pin" };
            if (node.status === "running" || node.status === "pending_approval") {
                return { open: true, via: "auto" };
            }
            if (state.expandedTools.has(node.id)) return { open: true, via: "auto" };
            return CLOSED;

        case "agent_message":
            // Default OPEN; collapsed only when the user collapsed it.
            return state.collapsedNodes.has(node.id) ? CLOSED : OPEN_DEFAULT;

        case "jekt_message":
            // Same policy as agent_message, and for the same reason spec G1
            // asks for: a jekt must be visible by default, not opt-in.
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

        case "shell":
            // Pin-to-expand only: unlike tools, a running shell stays collapsed
            // by default (spec §11). Only a pin opens it.
            if (state.pinnedNodes.has(node.id)) return { open: true, via: "pin" };
            return CLOSED;

        case "agent_error":
            // Error nodes are fixed-height and never user-collapsible.
            return OPEN_DEFAULT;

        case "context_compacted":
            // Fixed-height divider — never collapsible.
            return OPEN_DEFAULT;

        case "compaction_started":
            // Fixed-height announcement — never collapsible.
            return OPEN_DEFAULT;

        case "session_outcome":
            // Fixed-height divider — never collapsible, same as context_compacted.
            return OPEN_DEFAULT;

        case "day_divider":
            // Fixed-height calendar separator — never collapsible.
            return OPEN_DEFAULT;

        case "history_link":
            // Fixed-height link row — never collapsible.
            return OPEN_DEFAULT;

        case "resume_preflight":
            // Fixed-height continuity notice — never collapsible.
            return OPEN_DEFAULT;
    }
}
