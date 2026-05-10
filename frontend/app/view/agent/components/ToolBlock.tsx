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
 *     Applies to ALL statuses — running, success, and failed.
 *     Running tools show a ⏳ spinner in the summary; failed tools show ✗.
 *     No content body is rendered in the document flow.
 *   - Click summary or strip Expand button: pins the expanded state. The portal
 *     overlay renders the tool content. Clicking again unpins / collapses.
 *
 * Prior behavior removed in SPEC_AGENT_PANE_FOLLOWUPS items #4 + #5:
 *   running and failed states used to force-expand inline, taking 2+ lines
 *   per tool. That violated the explicit one-line rule in
 *   docs/specs/tool-collapse.md and the user's feedback memory. Now all
 *   statuses collapse by default; progress and error content are still
 *   accessible via hover/pin.
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
import { Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { createBlock } from "@/store/global";
import type { ToolNode } from "../types";
import { BashOutputViewer } from "./BashOutputViewer";
import { CompactResult } from "./CompactResult";
import { DiffViewer } from "./DiffViewer";
import { HighlightedCode } from "./HighlightedCode";
import { detectLanguage } from "./detectLanguage";

interface ToolBlockProps {
    node: ToolNode;
    /** User has clicked to pin this tool block open. */
    pinned: boolean;
    /** Toggle the pinned state (called on click of the collapsed row). */
    onTogglePin: () => void;
}

const STATUS_ICON: Record<ToolNode["status"], string> = {
    running: "⏳",
    pending_approval: "⚠",
    success: "✓",
    failed: "✗",
    denied: "⊘",
};

// Walk upward from `el` to find the first ancestor with a non-1 CSS zoom.
// The portal renders to document.body (outside the zoom context), so we must
// apply the same zoom to the portal container and shrink the width constraints
// by the zoom factor so the visual size matches the underlying block.
function getAncestorZoom(el: HTMLElement): number {
    let cur: HTMLElement | null = el;
    while (cur) {
        const z = parseFloat(getComputedStyle(cur).zoom ?? "1");
        if (!isNaN(z) && z !== 1) return z;
        cur = cur.parentElement;
    }
    return 1;
}

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
    // Position of the collapsed row in viewport coordinates, recomputed when
    // pinned flips true. The overlay is rendered via a <Portal> to document.body
    // to escape the paint containment imposed by .agent-document-node-wrapper's
    // `content-visibility: auto` — otherwise the overlay gets clipped.
    const [overlayRect, setOverlayRect] = createSignal<{
        left: number;
        right: number;
        top: number;    // used when overlayUp = false (drop down below the row)
        bottom: number; // used when overlayUp = true  (pop up above the row)
        width: number;
    } | null>(null);
    const [overlayUp, setOverlayUp] = createSignal(false);

    let blockRef: HTMLDivElement | undefined;

    const expanded = () => props.pinned;
    const overlayMode = () => props.pinned;

    const statusIcon = (): string => STATUS_ICON[props.node.status] || "•";

    const measure = () => {
        if (!blockRef) return;
        const blockRect = blockRef.getBoundingClientRect();
        const scrollParent = findScrollParent(blockRef);
        const parentBottom = scrollParent
            ? scrollParent.getBoundingClientRect().bottom
            : window.innerHeight;
        const spaceBelow = parentBottom - blockRect.bottom;
        setOverlayUp(spaceBelow < OVERLAY_MAX_HEIGHT_PX);
        setOverlayRect({
            left: blockRect.left,
            right: window.innerWidth - blockRect.right,
            top: blockRect.bottom,
            bottom: window.innerHeight - blockRect.top,
            width: blockRect.width,
        });
    };

    // Measure position when pin flips true so the portal has coordinates on first render.
    createEffect(() => {
        if (props.pinned && blockRef) {
            measure();
        }
    });

    // Reposition the portal overlay on scroll so a PINNED overlay stays
    // anchored to its block. Only attached while pinned — hover-only overlays
    // don't need scroll tracking because mouseleave fires before any
    // significant drift, and removing the listener eliminates the one-frame
    // reposition lag that causes the hover overlay to visually twitch.
    let scrollParentRef: HTMLElement | null = null;
    const handleScroll = () => measure();

    createEffect(() => {
        const active = props.pinned; // scroll tracking only while pinned
        if (active && blockRef) {
            scrollParentRef = findScrollParent(blockRef);
            scrollParentRef?.addEventListener("scroll", handleScroll, { passive: true });
            window.addEventListener("resize", handleScroll, { passive: true });
        } else if (scrollParentRef) {
            scrollParentRef.removeEventListener("scroll", handleScroll);
            window.removeEventListener("resize", handleScroll);
            scrollParentRef = null;
        }
    });

    onCleanup(() => {
        if (scrollParentRef) {
            scrollParentRef.removeEventListener("scroll", handleScroll);
            window.removeEventListener("resize", handleScroll);
            scrollParentRef = null;
        }
    });

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

            case "Read": {
                const filePath = (node.params as any).file_path ?? "";
                const content: string | undefined = (node.result as any)?.content;
                return (
                    <div class="agent-tool-read">
                        <div class="agent-tool-file-path">{filePath}</div>
                        <Show
                            when={content}
                            fallback={
                                <Show when={node.result}>
                                    <CompactResult tool={node.tool} params={node.params as any} result={node.result} />
                                </Show>
                            }
                        >
                            <HighlightedCode
                                code={content!}
                                lang={detectLanguage(filePath, content!.split("\n")[0])}
                                class="agent-tool-read-content"
                            />
                        </Show>
                    </div>
                );
            }

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

    // Overlay style — only meaningful when overlayMode() is true. Computed
    // from overlayRect() which is set during measure() / handleScroll.
    //
    // Zoom correction: the portal renders to document.body, outside the
    // .agent-view zoom context. getBoundingClientRect() returns viewport
    // coordinates (already zoomed), so top/left stay as-is. But the portal
    // container's width/height are in un-zoomed CSS px, so min/max-width must
    // be divided by zoom to produce the correct visual size. We also apply
    // `zoom` to the portal container itself so font-sizes and spacing scale
    // to match the surrounding document.
    const overlayStyle = (): Record<string, string> => {
        const r = overlayRect();
        if (!r) return { display: "none" };
        const zoom = blockRef ? getAncestorZoom(blockRef) : 1;
        // Clamp right edge to viewport so the overlay never bleeds off-screen.
        const maxRight = (window.innerWidth - r.left - 16) / zoom;
        const minWidth = r.width / zoom;
        if (overlayUp()) {
            return {
                position: "fixed",
                left: `${r.left}px`,
                minWidth: `${minWidth}px`,
                maxWidth: `${maxRight}px`,
                bottom: `${r.bottom}px`,
                zoom: String(zoom),
            };
        }
        return {
            position: "fixed",
            left: `${r.left}px`,
            minWidth: `${minWidth}px`,
            maxWidth: `${maxRight}px`,
            top: `${r.top}px`,
            zoom: String(zoom),
        };
    };

    return (
        <div
            ref={blockRef}
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
        >
            <div class="agent-tool-summary" onClick={props.onTogglePin}>
                <span class="agent-tool-status-icon">{statusIcon()}</span>
                <span class="agent-tool-name" title={props.node.summary}>{props.node.summary}</span>
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
            {/* All expansion now routes through the portal overlay — running,
                failed, success all collapse by default (SPEC_AGENT_PANE_FOLLOWUPS
                items #4 and #5). The portal escapes the paint containment on
                `.agent-document-node-wrapper` (content-visibility: auto). */}
            <Show when={expanded()}>
                <Portal>
                    <div
                        class={clsx("agent-tool-content", "agent-tool-content--portal", {
                            "overlay-up": overlayUp(),
                        })}
                        style={overlayStyle()}
                        onClick={(e) => e.stopPropagation()}
                    >
                        {renderToolContent()}
                    </div>
                </Portal>
            </Show>
        </div>
    );
};

ToolBlock.displayName = "ToolBlock";
