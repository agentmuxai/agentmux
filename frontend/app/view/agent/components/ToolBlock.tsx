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
import { Show, createEffect, createSignal, onCleanup, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
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
    // Position of the collapsed row in viewport coordinates, recomputed on
    // mouseenter. The overlay is rendered via a <Portal> to document.body to
    // escape the paint containment imposed by .agent-document-node-wrapper's
    // `content-visibility: auto` — otherwise the absolute-positioned overlay
    // gets clipped at the wrapper's edge and the hover expansion is invisible.
    const [overlayRect, setOverlayRect] = createSignal<{
        left: number;
        right: number;
        top: number;    // used when overlayUp = false (drop down below the row)
        bottom: number; // used when overlayUp = true  (pop up above the row)
        width: number;
    } | null>(null);
    const [overlayUp, setOverlayUp] = createSignal(false);

    let blockRef: HTMLDivElement | undefined;

    // Collapsed-by-default for every status. Running, success, and failed
    // all get the same one-line treatment (SPEC_AGENT_PANE_FOLLOWUPS items
    // #4 and #5). Hover reveals the body via the portal overlay; click
    // pins it open. Progress + error content are still accessible — just
    // not always-on visually.
    //
    // The running state shows a spinner in the collapsed summary; the
    // failed state shows a red ✗. Both indicate what's happening without
    // consuming vertical space from the pane.
    const expanded = () => props.pinned || hovered();

    // All transient / pinned expansion now goes through the portal overlay.
    // Nothing renders inline in the document flow anymore — so we don't
    // need a separate `overlayMode` predicate; every expansion IS overlay
    // mode. Kept as an accessor for parity with the existing SCSS hooks.
    const overlayMode = () => hovered() || props.pinned;

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

    // Hover-leave is deferred by a microtask so that mouseleave on the
    // summary block followed by mouseenter on the portal overlay doesn't
    // cause a visible collapse. The portal is rendered to document.body,
    // not as a DOM descendant of the block — a direct onMouseLeave →
    // setHovered(false) would momentarily hide the overlay as the cursor
    // transits the one-pixel gap between the summary row and the portal.
    let leavePending = 0;
    const handleMouseEnter = () => {
        if (leavePending) {
            clearTimeout(leavePending);
            leavePending = 0;
        }
        measure();
        setHovered(true);
    };

    const handleMouseLeave = () => {
        if (leavePending) clearTimeout(leavePending);
        leavePending = window.setTimeout(() => {
            leavePending = 0;
            setHovered(false);
        }, 0);
    };

    onCleanup(() => {
        if (leavePending) clearTimeout(leavePending);
    });

    // Reposition the portal overlay on scroll so a pinned overlay stays
    // anchored to its block. Only attached while overlayMode() is active
    // to avoid one listener per tool block in the document.
    let scrollParentRef: HTMLElement | null = null;
    const handleScroll = () => measure();

    createEffect(() => {
        const active = overlayMode();
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

    // Overlay style — only meaningful when overlayMode() is true. Computed
    // from overlayRect() which is set during handleMouseEnter / handleScroll.
    const overlayStyle = (): Record<string, string> => {
        const r = overlayRect();
        if (!r) return { display: "none" };
        if (overlayUp()) {
            return {
                position: "fixed",
                left: `${r.left}px`,
                width: `${r.width}px`,
                bottom: `${r.bottom}px`,
            };
        }
        return {
            position: "fixed",
            left: `${r.left}px`,
            width: `${r.width}px`,
            top: `${r.top}px`,
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
                        // Use handleMouseEnter (not an inline setHovered(true))
                        // so the pending leavePending timeout from the block's
                        // mouseleave is cleared. Otherwise the timeout fires
                        // on the next tick after the portal's mouseenter
                        // has set hovered=true, flipping it back to false
                        // and collapsing the overlay mid-transit.
                        onMouseEnter={handleMouseEnter}
                        onMouseLeave={handleMouseLeave}
                    >
                        {renderToolContent()}
                    </div>
                </Portal>
            </Show>
        </div>
    );
};

ToolBlock.displayName = "ToolBlock";
