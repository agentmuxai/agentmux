// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlock — single-line collapsed-by-default tool display with
 * click-to-pin and active-state auto-expand.
 *
 * Behavior (since SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md):
 *   - Collapsed (default): one line showing status icon + tool name +
 *     duration + (while streaming) the live-tail line. Applies to ALL
 *     terminated statuses.
 *   - Auto-expand: `running` and `pending_approval` keep the panel open
 *     in flow. After a terminal transition the panel stays open while the
 *     tool is held (`props.heldOpen`, backed by `documentState.heldOpenNodes`)
 *     — i.e. while it's still on screen — and collapses once its row scrolls
 *     off the top. This replaced the old fixed post-completion timer; see
 *     docs/specs/PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md.
 *   - Click summary: pins the expanded state. Clicking again unpins.
 *   - Hover: no panel expand — the hover-to-peek model from the prior
 *     `tool-collapse.md` spec is still gone (that removed three overlapping
 *     visuals: browser title tooltip, larger log panel, fast
 *     expand/collapse — collapsed into the single auto-expand panel above).
 *     What DOES show on hover is the peek overlay: time, token estimate,
 *     and the full word-wrapped command/detail string (no output, no
 *     expand trigger). It shows in every state, collapsed or expanded —
 *     the header summary is ellipsis-truncated either way, and the
 *     expanded panel doesn't reliably show the full call
 *     (SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §1).
 *
 * SolidJS reactivity note:
 *   Props are accessed via `props.X` (never destructured in the function
 *   signature). Destructuring a SolidJS component's props captures the
 *   value at mount time and breaks reactivity for any prop that changes
 *   without triggering a parent re-render of the component. This bit us
 *   in an earlier version of this file: `pinned` was destructured, and
 *   pin toggles — which mutate `documentState` but not the document
 *   array — never reached the component, so the pin state appeared to
 *   reset on the next render cycle.
 */

import clsx from "clsx";
import { Show, createMemo, type JSX } from "solid-js";
import type { ToolNode } from "../../../agent-model/types";
import { AnsweredQuestionMessage } from "./AnsweredQuestionMessage";
import { toolHeaderParts } from "./tool-header";
import { ToolPanel, ToolPeekOverlay, ToolSummary } from "./tool-block-parts";
import {
    useHoldOpenOnFinish,
    useToolDisclosure,
    useToolPeek,
    useUnloadedResult,
    type ToolBlockProps,
} from "./tool-block-state";

// The tool preview used to carry its OWN Ctrl+wheel zoom here — a local
// `previewFontScale` signal applied as `font-size: N%` on the overlay-log
// wrapper. Removed 2026-09-06; see
// docs/reports/REPORT_ZOOM_BINDINGS_AUDIT_2026_09_06.md for the full audit.
//
// Short version: it could not work, and deleting it makes previews zoom
// MORE, not less. Pane zoom is CSS `zoom` on the pane root, which scales the
// whole box tree; the preview's font-size percentage only reached
// descendants authored in `em`/`%`, and `_document-nodes.scss` is 66
// hardcoded-px font-size rules against 8 relative ones. So it moved ~11% of
// the type in the panel, and whether it appeared to do anything at all
// depended on which renderer you happened to be looking at (DiffViewer,
// BashOutputViewer, HighlightedCode and CompactResult never referenced the
// scale). That is the "sometimes it doesn't even work" this removes.
//
// It also fought the pane: the same Ctrl+wheel did different things a few
// pixels apart (an `overPreviewBody` hit-test with no visual cue), the two
// scales composed multiplicatively under different clamps (0.5–2.0 vs
// 0.7–2.0), and the scale lived in a component-local signal inside a
// VIRTUALIZED list — so it silently reset on scroll, and `<Index>` slot
// reuse could land a stale scale on an unrelated tool block.
//
// Ctrl+wheel over a preview now simply zooms the pane, which already
// scales previews correctly, hardcoded pixels included.

// A resolved AskUserQuestion renders as a user message instead of the generic collapsed tool row —
// it substantively IS user input. Guarded on `answerText` (not just tool+status) so transcripts
// answered before this field existed keep the old rendering instead of showing an empty body. See
// SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md.
const isAnsweredQuestion = (node: ToolNode): boolean =>
    node.toolName === "AskUserQuestion" && node.status === "success" && node.answerText != null;

// A declined AskUserQuestion (Cancel button / Escape) — reuses the existing "denied" ToolNode
// status (already a full first-class status: icon, fail-terminal auto-collapse, STATUS_LABEL)
// rather than adding a new one, since it is the correct semantic fit: a rejected permission
// request, not an orphaned/force-terminated one ("canceled" is reserved for that — see the sibling
// status and useAgentQuestions.ts). Scoped by toolName so this can never collide with a real
// tool-permission denial (a different, unrelated feature — the tool:decision path). Gated on
// `questionText` the same way isAnsweredQuestion gates on `answerText`: a transcript denied before
// this field existed falls back to the generic collapsed row instead of showing an empty body.
const isCancelledQuestion = (node: ToolNode): boolean =>
    node.toolName === "AskUserQuestion" && node.status === "denied" && node.questionText != null;

export const ToolBlock = (props: ToolBlockProps): JSX.Element => {
    useHoldOpenOnFinish(props);
    const view = useToolDisclosure(props);
    const unloaded = useUnloadedResult(props, view);
    // Header parts, composed from the node's fields rather than the `summary` baked at parse time —
    // see tool-header.ts. Empty (and so not shown) when there's nothing tool-kind-specific to show.
    const header = createMemo(() => toolHeaderParts(props.node));
    const peek = useToolPeek(props, header);

    return (
        <Show
            when={!isAnsweredQuestion(props.node) && !isCancelledQuestion(props.node)}
            fallback={<AnsweredQuestionMessage node={props.node} cancelled={isCancelledQuestion(props.node)} />}
        >
            <div
                class={clsx("agent-tool-block", {
                    collapsed: !view.expanded(),
                    expanded: view.expanded(),
                    pinned: props.pinned,
                    running: props.node.status === "running",
                    success: props.node.status === "success",
                    failed: props.node.status === "failed",
                    canceled: props.node.status === "canceled",
                    pending_approval: props.node.status === "pending_approval",
                    awaiting_answer: props.node.status === "awaiting_answer",
                    denied: props.node.status === "denied",
                })}
                data-tool={props.node.tool.toLowerCase()}
                // Raw provider tool name, distinct from `data-tool` above: `tool` is normalized to
                // a coarse closed set (unrecognized names, e.g. "AskUserQuestion", collapse to
                // "other" — see normalizeToolName in stream-parser.ts), which loses exactly the
                // names CSS needs to target one specific open-ended tool without touching the
                // shared "other" styling every other unrecognized tool also falls back to.
                data-tool-name={props.node.toolName?.toLowerCase()}
                // Hold an open, pin-toggled (panel) tool while the mouse is anywhere in it.
                onMouseEnter={() => {
                    if (view.disclosure().open && view.disclosure().toggle === "pin") view.setUserHolding(true);
                }}
                onMouseLeave={() => view.setUserHolding(false)}
            >
                <ToolSummary node={props.node} header={header} dispatchMatch={props.dispatchMatch} peek={peek} view={view} />
                <ToolPeekOverlay node={props.node} header={header} peek={peek} />
                <ToolPanel node={props.node} dispatchMatch={props.dispatchMatch} view={view} unloaded={unloaded} />
            </div>
        </Show>
    );
};

ToolBlock.displayName = "ToolBlock";
