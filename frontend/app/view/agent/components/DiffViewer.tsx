// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * DiffViewer - Displays unified diff format with syntax highlighting
 */

import React, { memo } from "react";
import type { EditParams, EditResult } from "../types";

interface DiffViewerProps {
    params: EditParams;
    result?: EditResult;
}

export const DiffViewer: React.FC<DiffViewerProps> = memo(({ params, result }) => {
    const diff = result?.diff;
    if (!diff) {
        return (
            <pre className="agent-diff-empty">
                No diff available
                {"\n"}
                File: {params.file_path}
            </pre>
        );
    }

    const lines = diff.split("\n");

    return (
        <pre className="agent-diff">
            <div className="agent-diff-header">{params.file_path}</div>
            {lines.map((line, i) => {
                const cls = line.startsWith("+")
                    ? "agent-diff-add"
                    : line.startsWith("-")
                      ? "agent-diff-del"
                      : line.startsWith("@")
                        ? "agent-diff-hunk"
                        : "agent-diff-ctx";
                return (
                    <div key={i} className={cls}>
                        {line}
                    </div>
                );
            })}
        </pre>
    );
});

DiffViewer.displayName = "DiffViewer";
