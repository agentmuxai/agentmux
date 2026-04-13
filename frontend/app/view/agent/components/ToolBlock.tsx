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

export const ToolBlock = (props: ToolBlockProps): JSX.Element => {
    const [hovered, setHovered] = createSignal(false);

    // Force-expand rules — override hover/pin when the user must see content.
    // `failed` stays expanded so errors are immediately visible.
    // `running` stays expanded so the user can watch progress.
    const forceExpanded = () =>
        props.node.status === "running" || props.node.status === "failed";

    const expanded = () => props.pinned || hovered() || forceExpanded();

    const statusIcon = (): string => STATUS_ICON[props.node.status] || "•";

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
                running: props.node.status === "running",
                success: props.node.status === "success",
                failed: props.node.status === "failed",
            })}
            onMouseEnter={() => setHovered(true)}
            onMouseLeave={() => setHovered(false)}
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
