// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlock - Collapsible tool execution display with smart rendering
 */

import clsx from "clsx";
import React, { memo, useState } from "react";
import type { ToolNode } from "../types";
import { BashOutputViewer } from "./BashOutputViewer";
import { DiffViewer } from "./DiffViewer";

interface ToolBlockProps {
    node: ToolNode;
    collapsed: boolean;
    onToggle: () => void;
}

const TOOL_RESULT_MAX_LENGTH = 50000; // 50KB

export const ToolBlock: React.FC<ToolBlockProps> = memo(({ node, collapsed, onToggle }) => {
    const [showFullResult, setShowFullResult] = useState(false);

    // Determine if result needs truncation
    const resultString = node.result ? JSON.stringify(node.result) : "";
    const isTruncated = resultString.length > TOOL_RESULT_MAX_LENGTH;
    const displayResult =
        isTruncated && !showFullResult
            ? resultString.slice(0, TOOL_RESULT_MAX_LENGTH)
            : resultString;

    // Render tool-specific content
    const renderToolContent = () => {
        if (node.status === "running") {
            return (
                <div className="agent-tool-loading">
                    <span className="agent-tool-spinner">⏳</span> Running...
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
                    <div className="agent-tool-read">
                        <div className="agent-tool-file-path">{(node.params as any).file_path}</div>
                        {node.result && (
                            <pre className="agent-tool-read-content">
                                {(node.result as any).content || displayResult}
                            </pre>
                        )}
                    </div>
                );

            case "Write":
                return (
                    <div className="agent-tool-write">
                        <div className="agent-tool-file-path">{(node.params as any).file_path}</div>
                        <div className="agent-tool-write-info">
                            {node.result && `Wrote ${(node.result as any).bytesWritten || 0} bytes`}
                        </div>
                    </div>
                );

            case "Grep":
            case "Glob":
                return (
                    <div className="agent-tool-search">
                        <div className="agent-tool-pattern">Pattern: {(node.params as any).pattern}</div>
                        <pre className="agent-tool-search-results">{displayResult}</pre>
                    </div>
                );

            case "Task":
                return (
                    <div className="agent-tool-task">
                        <pre className="agent-tool-task-info">{displayResult}</pre>
                    </div>
                );

            default:
                return <pre className="agent-tool-generic">{displayResult}</pre>;
        }
    };

    return (
        <div
            className={clsx("agent-tool-block", {
                collapsed,
                expanded: !collapsed,
                running: node.status === "running",
                success: node.status === "success",
                failed: node.status === "failed",
            })}
            onClick={onToggle}
        >
            <div className="agent-tool-summary">
                <span className="agent-tool-chevron">{collapsed ? "▸" : "▾"}</span>
                <span className="agent-tool-name">{node.summary}</span>
                {node.duration && (
                    <span className="agent-tool-duration">({node.duration.toFixed(1)}s)</span>
                )}
            </div>
            {!collapsed && (
                <div className="agent-tool-content" onClick={(e) => e.stopPropagation()}>
                    {renderToolContent()}
                    {isTruncated && !showFullResult && (
                        <button
                            className="agent-tool-show-more"
                            onClick={(e) => {
                                e.stopPropagation();
                                setShowFullResult(true);
                            }}
                        >
                            Show full output ({(resultString.length / 1024).toFixed(0)}KB)
                        </button>
                    )}
                </div>
            )}
        </div>
    );
});

ToolBlock.displayName = "ToolBlock";
