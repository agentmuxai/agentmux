// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * TerminalOutputBlock - Renders raw terminal output from bootstrap phase
 * (npm install, CLI startup, auth) before JSON streaming begins.
 */

import clsx from "clsx";
import React, { memo } from "react";
import type { TerminalOutputNode } from "../types";

interface TerminalOutputBlockProps {
    node: TerminalOutputNode;
}

export const TerminalOutputBlock: React.FC<TerminalOutputBlockProps> = memo(({ node }) => {
    return (
        <div className={clsx("agent-terminal-output", { complete: node.complete })}>
            <div className="agent-terminal-header">
                <span className="agent-terminal-icon">{node.complete ? "\u2713" : "\u23F3"}</span>
                <span className="agent-terminal-label">
                    {node.complete ? "Bootstrap" : "Starting..."}
                </span>
            </div>
            <pre className="agent-terminal-content">{node.content}</pre>
        </div>
    );
});

TerminalOutputBlock.displayName = "TerminalOutputBlock";
