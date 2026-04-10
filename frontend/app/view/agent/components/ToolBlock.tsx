// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlock - Collapsible tool execution display with smart rendering
 */

import clsx from "clsx";
import { Show, type JSX } from "solid-js";
import { createBlock } from "@/store/global";
import type { ToolNode } from "../types";
import { BashOutputViewer } from "./BashOutputViewer";
import { CompactResult } from "./CompactResult";
import { DiffViewer } from "./DiffViewer";

interface ToolBlockProps {
    node: ToolNode;
    collapsed: boolean;
    onToggle: () => void;
}

export const ToolBlock = ({ node, collapsed, onToggle }: ToolBlockProps): JSX.Element => {
    // Render tool-specific content
    const renderToolContent = (): JSX.Element => {
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
                collapsed,
                expanded: !collapsed,
                running: node.status === "running",
                success: node.status === "success",
                failed: node.status === "failed",
            })}
            onClick={onToggle}
        >
            <div class="agent-tool-summary">
                <span class="agent-tool-chevron">{collapsed ? "▸" : "▾"}</span>
                <span class="agent-tool-name">{node.summary}</span>
                <Show when={node.duration}>
                    <span class="agent-tool-duration">({node.duration.toFixed(1)}s)</span>
                </Show>
                <Show when={node.tool === "Agent"}>
                    <button
                        class="agent-tool-open-pane"
                        title="Open subagent in new pane"
                        onClick={(e) => {
                            e.stopPropagation();
                            const agentId = (node.params as any).subagent_id || node.id;
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
            </div>
            <Show when={!collapsed}>
                <div class="agent-tool-content" onClick={(e) => e.stopPropagation()}>
                    {renderToolContent()}
                </div>
            </Show>
        </div>
    );
};

ToolBlock.displayName = "ToolBlock";
