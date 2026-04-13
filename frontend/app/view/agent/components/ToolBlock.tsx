// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlock - Single-line collapsed-by-default tool display with
 * hover-to-expand and click-to-pin semantics.
 *
 * See docs/specs/tool-collapse.md for the product requirement.
 *
 * Behavior:
 *   - Collapsed (default): one line showing status icon + tool name + ellipsis.
 *     No wrapping, no content rendered inside.
 *   - Hover: expands instantly on mouseenter, collapses instantly on mouseleave.
 *   - Click: pins the expanded state. A pinned-open block stays open even
 *     after the mouse leaves. Clicking again unpins.
 *   - Force-expanded regardless of hover/pin state:
 *       * status === "running" — actively executing
 *       * status === "failed"  — user needs to see the error
 *
 * SolidJS reactivity note:
 *   Props are accessed via `props.X` (never destructured in the function
 *   signature). Destructuring a SolidJS component's props captures the
 *   value at mount time and breaks reactivity for any prop that changes
 *   without triggering a parent re-render of the component. This bit us
 *   in an earlier version of this file: `pinned` was destructured, and
 *   pin toggles — which mutate `documentState` but not the document
 *   array — never reached the component, so clicking to pin visibly
 *   worked while hovered but collapsed again on mouseleave.
 */

import clsx from "clsx";
import { Show, createSignal, type JSX } from "solid-js";
import { createBlock } from "@/store/global";
import type { ToolNode } from "../types";
import { BashOutputViewer } from "./BashOutputViewer";
import { CompactResult } from "./CompactResult";
import { DiffViewer } from "./DiffViewer";

interface ToolBlockProps {
    node: ToolNode;
    /** User has clicked to pin this tool block open. */
    pinned: boolean;
    /** Toggle the pinned state (called on click of the collapsed row). */
    onTogglePin: () => void;
}

const STATUS_ICON: Record<ToolNode["status"], string> = {
    running: "⏳",
    success: "✓",
    failed: "✗",
};

// Walk upward from `el` until we find a scrollable ancestor. Used to decide
// whether the overlay has room to pop down or must flip up.
function findScrollParent(el: HTMLElement): HTMLElement | null {
    let parent: HTMLElement | null = el.parentElement;
    while (parent && parent !== document.body) {
        const style = getComputedStyle(parent);
        if (style.overflowY === "auto" || style.overflowY === "scroll") {
            return parent;
        }
        parent = parent.parentElement;
    }
    return null;
}

// CSS max-height cap for the overlay. When there's less than this much space
// below a tool block in its scroll container, we flip the overlay to open
// upward instead of downward.
const OVERLAY_MAX_HEIGHT_PX = 400;

export const ToolBlock = (props: ToolBlockProps): JSX.Element => {
    const [hovered, setHovered] = createSignal(false);
    // `true` when the overlay should pop UP above the summary row instead of
    // down below it. Decided on each mouseenter by measuring against the
    // scroll parent — not reactive to layout changes, just a one-time check
    // at hover time. Recomputed on every mouseenter so scrolling between
    // hovers picks up the new position.
    const [overlayUp, setOverlayUp] = createSignal(false);

    // Force-expand rules — override hover/pin when the user must see content.
    // `failed` stays expanded so errors are immediately visible.
    // `running` stays expanded so the user can watch progress.
    const forceExpanded = () =>
        props.node.status === "running" || props.node.status === "failed";

    const expanded = () => props.pinned || hovered() || forceExpanded();

    // Overlay mode applies ONLY for transient (hover) and explicit (pinned)
    // expansion — NOT for persistent state (running/failed). Persistent state
    // stays inline so the user can scroll past it. See
    // specs/SPEC_TOOL_OVERLAY_AND_SCROLL_ON_TYPE_2026_04_13.md §2.5.
    const overlayMode = () => (hovered() || props.pinned) && !forceExpanded();

    const statusIcon = (): string => STATUS_ICON[props.node.status] || "•";

    const handleMouseEnter = (e: MouseEvent) => {
        // Measure room BEFORE flipping the overlay class — the measurement
        // is done on the collapsed block (1-line height), so `blockRect.bottom`
        // is the start-of-overlay position. If there's <400px between that
        // and the scroll parent's bottom, flip up.
        const block = e.currentTarget as HTMLElement;
        const blockRect = block.getBoundingClientRect();
        const scrollParent = findScrollParent(block);
        const parentBottom = scrollParent
            ? scrollParent.getBoundingClientRect().bottom
            : window.innerHeight;
        const spaceBelow = parentBottom - blockRect.bottom;
        setOverlayUp(spaceBelow < OVERLAY_MAX_HEIGHT_PX);
        setHovered(true);
    };

    const handleMouseLeave = () => {
        setHovered(false);
        // overlayUp stays — it'll be recomputed on next mouseenter
    };

    // Render tool-specific content — only evaluated when expanded.
    const renderToolContent = (): JSX.Element => {
        const node = props.node;
        if (node.status === "running") {
            return (
                <div class="agent-tool-loading">
                    <span class="agent-tool-spinner">⏳</span> Running...
                </div>
            );
        }

        switch (node.tool) {
            case "Edit":
                return <DiffViewer params={node.params as any} result={node.result as any} />;

            case "Bash":
                return <BashOutputViewer params={node.params as any} result={node.result as any} />;

            case "Read":
                return (
                    <div class="agent-tool-read">
                        <div class="agent-tool-file-path">{(node.params as any).file_path}</div>
                        <Show when={node.result}>
                            {(node.result as any).content ? (
                                <pre class="agent-tool-read-content">{(node.result as any).content}</pre>
                            ) : (
                                <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                            )}
                        </Show>
                    </div>
                );

            case "Write":
                return (
                    <div class="agent-tool-write">
                        <div class="agent-tool-file-path">{(node.params as any).file_path}</div>
                        <div class="agent-tool-write-info">
                            {node.result && `Wrote ${(node.result as any).bytesWritten || 0} bytes`}
                        </div>
                    </div>
                );

            case "Grep":
            case "Glob":
                return (
                    <div class="agent-tool-search">
                        <div class="agent-tool-pattern">Pattern: {(node.params as any).pattern}</div>
                        <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                    </div>
                );

            case "Agent":
                return (
                    <div class="agent-tool-agent">
                        <Show when={(node.params as any).description}>
                            <div class="agent-tool-agent-desc">{(node.params as any).description}</div>
                        </Show>
                        <Show when={node.result}>
                            <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                        </Show>
                    </div>
                );

            case "Task":
                return (
                    <div class="agent-tool-task">
                        <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                    </div>
                );

            default:
                return <CompactResult tool={node.tool} params={node.params as any} result={node.result} />;
        }
    };

    return (
        <div
            class={clsx("agent-tool-block", {
                collapsed: !expanded(),
                expanded: expanded(),
                pinned: props.pinned,
                "overlay-mode": overlayMode(),
                "overlay-up": overlayMode() && overlayUp(),
                running: props.node.status === "running",
                success: props.node.status === "success",
                failed: props.node.status === "failed",
            })}
            onMouseEnter={handleMouseEnter}
            onMouseLeave={handleMouseLeave}
        >
            <div class="agent-tool-summary" onClick={props.onTogglePin}>
                <span class="agent-tool-status-icon">{statusIcon()}</span>
                <span class="agent-tool-name">{props.node.summary}</span>
                <Show when={props.node.duration}>
                    <span class="agent-tool-duration">({props.node.duration.toFixed(1)}s)</span>
                </Show>
                <Show when={props.node.tool === "Agent"}>
                    <button
                        class="agent-tool-open-pane"
                        title="Open subagent in new pane"
                        onClick={(e) => {
                            e.stopPropagation();
                            const agentId = (props.node.params as any).subagent_id || props.node.id;
                            createBlock({
                                meta: {
                                    view: "subagent",
                                    "subagent:id": agentId,
                                } as any,
                            });
                        }}
                    >
                        ⧉
                    </button>
                </Show>
                <span class="agent-tool-ellipsis">…</span>
            </div>
            <Show when={expanded()}>
                <div class="agent-tool-content" onClick={(e) => e.stopPropagation()}>
                    {renderToolContent()}
                </div>
            </Show>
        </div>
    );
};

ToolBlock.displayName = "ToolBlock";
