// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolOverlayLog — log body of the tool overlay (Phase 3 of
 * SPEC_TOOL_BLOCK_LIVE_LOG_2026_05_11.md §3.4).
 *
 * Renders `ToolNode.log.chunks` if the streaming runner has populated
 * them. Falls back to the per-tool rich result renderer
 * (tool-renderers/, e.g. BashOutputViewer, DiffViewer) when no chunks are present — preserves today's UX
 * for tools that don't stream (yet) or that have already terminated
 * before Phase 2's backend wraps the runner.
 *
 * No ANSI parsing in this PR (Phase 3): chunks render as plain text.
 * ANSI parsing lands in Phase γ (perf + worker offload) per the spec.
 */

import { Match, Show, Switch, createMemo, type JSX } from "solid-js";
// `Show` retained for fallback ToolOverlayResult sub-tree.
import type { ToolNode } from "../../../agent-model/types";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { ChunkPreview } from "./ChunkPreview";
import { renderCompactDefault } from "./tool-renderers/builtins";
import { registerToolRenderers } from "./tool-renderers";
import { resolveToolRenderer, type ToolRenderContext } from "./tool-renderers/registry";
import {
    useBranchHeightContinuity,
    useFollowEffects,
    useLogBranch,
    usePanelHidden,
    usePreviewScroll,
    useResizePin,
    useScrollHandoff,
    useShownContent,
    type LogChunk,
    type ToolOverlayLogProps,
} from "./tool-overlay-log-state";

// Every result renderer, from one explicit list (tool-renderers/index.ts).
registerToolRenderers();

export const ToolOverlayLog = (props: ToolOverlayLogProps): JSX.Element => {
    let scrollRef: HTMLDivElement | undefined;
    let contentRef: HTMLDivElement | undefined;
    const getEl = () => scrollRef;
    const hasResizeObserver = typeof ResizeObserver !== "undefined";

    const view = useLogBranch(props);
    const { branch } = view;
    const { shownNode, shownChunks, shownDispatch } = useShownContent(props, branch);

    useScrollHandoff(getEl);
    const panelHidden = usePanelHidden(getEl);
    const scroll = usePreviewScroll(props, getEl, panelHidden);
    useResizePin(getEl, () => contentRef, scroll, hasResizeObserver);
    useFollowEffects(props, view, panelHidden, scroll, getEl, hasResizeObserver);
    useBranchHeightContinuity(props, branch, getEl);

    /**
     * Render decision — exhaustive, mutually exclusive branches via
     * `<Switch>` rather than the prior 4-way `<Show>` cascade that
     * rendered `ToolOverlayResult` from TWO different branches. SolidJS's
     * reconciler saw the same component type in two sibling slots and
     * (during the running → success state transition) tried to re-parent
     * the DOM node from one Show slot to the other, calling
     * `replaceChild` on a node that was no longer a child of the
     * expected parent. `<Switch>` exits all other branches before
     * rendering the matched one — no shared DOM between branches.
     */
    return (
        <div
            class="agent-tool-overlay-log"
            ref={scrollRef}
            onScroll={() => scroll.follow.scrolled()}
        >
            <div class="agent-tool-overlay-log-content" ref={contentRef}>
            <Switch>
                <Match when={branch() === "chunks"}>
                    <ChunkList chunks={shownChunks()} />
                </Match>
                {/* One branch for the placeholder and the result: ToolOverlayResult
                    swaps them itself, without remounting. */}
                <Match when={branch() === "result" || branch() === "empty"}>
                    <ToolOverlayResult node={shownNode()} dispatchMatch={shownDispatch()} />
                </Match>
            </Switch>
            </div>
        </div>
    );
};

/** A running tool's streamed output: joined into lines and drawn exactly as
 *  its finished output will be (ChunkPreview.tsx). */
function ChunkList(props: { chunks: ReadonlyArray<LogChunk> }): JSX.Element {
    return <ChunkPreview chunks={props.chunks} class="agent-tool-log-feed" />;
}

ToolOverlayLog.displayName = "ToolOverlayLog";

/**
 * Per-tool rich result fallback — same content the old portal overlay
 * rendered. Used when there are no streaming chunks yet (or the tool
 * doesn't stream).
 */
function ToolOverlayResult(props: {
    node: ToolNode;
    dispatchMatch?: AgentDispatch;
}): JSX.Element {
    // NEVER destructure `const node = props.node`. The streaming
    // buffer keeps this component mounted across reducer updates;
    // the reducer's ToolChunkAppend replaces the ToolNode reference
    // for each chunk. A destructured `node` would capture the very
    // first reference (status="running", no log) and freeze — every
    // subsequent JSX evaluation would see the stale snapshot and
    // keep rendering "⏳ Running..." even after chunks landed and
    // status flipped. (Same pattern MarkdownBlock at lines 18-23
    // warns against; bit us on PR #887.)
    //
    // But not every new reference is a change: a stream flush hands each
    // mounted tool a new node object whether or not anything about it changed,
    // and the body below is built from the node as a whole, so reading
    // props.node directly rebuilt a finished tool's entire result (every line
    // of its output) on every flush: 2 to 6 times per tool in the lab, most of
    // the pane's DOM churn with several agents streaming
    // (SPEC_AGENT_PANE_STREAM_FRAME_BUDGET_2026_10_13.md §3 R3). This memo
    // passes a new node on only when one of its own fields changed.
    const node = createMemo(() => props.node, undefined, { equals: sameToolNode });
    return (
        <Show
            when={node().status !== "running"}
            fallback={
                <div class="agent-tool-loading">
                    <span class="agent-tool-spinner">⏳</span> Thinking...
                </div>
            }
        >
            {renderToolResultBody(node(), { dispatchMatch: props.dispatchMatch })}
        </Show>
    );
}

/** Same node for rendering: the same object, or one whose own fields are all
 *  the same values (nested objects compared by reference, as the reducer
 *  keeps the ones it didn't change). */
export function sameToolNode(a: ToolNode, b: ToolNode): boolean {
    if (a === b) return true;
    const ka = Object.keys(a) as (keyof ToolNode)[];
    if (ka.length !== Object.keys(b).length) return false;
    for (const k of ka) if (a[k] !== b[k]) return false;
    return true;
}

function renderToolResultBody(node: ToolNode, ctx?: ToolRenderContext): JSX.Element {
    // Hard fallback to the default renderer if (somehow) nothing is registered.
    const render = resolveToolRenderer(node) ?? renderCompactDefault;
    return render(node, ctx);
}
