// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DiffViewer - Displays unified diff format with syntax highlighting
 */

import { For, Show, type JSX } from "solid-js";
import type { EditParams, EditResult } from "../types";

interface DiffViewerProps {
    params: EditParams;
    result?: EditResult;
}

export const DiffViewer = ({ params, result }: DiffViewerProps): JSX.Element => {
    const diff = result?.diff;

    if (!diff) {
        return (
            <pre class="agent-diff-empty">
                No diff available
                {"\n"}
                File: {params.file_path}
            </pre>
        );
    }

    const lines = diff.split("\n");

    return (
        <pre class="agent-diff">
            <div class="agent-diff-header">{params.file_path}</div>
            <For each={lines}>
                {(line) => {
                    const cls = line.startsWith("+")
                        ? "agent-diff-add"
                        : line.startsWith("-")
                          ? "agent-diff-del"
                          : line.startsWith("@")
                            ? "agent-diff-hunk"
                            : "agent-diff-ctx";
                    return <div class={cls}>{line}</div>;
                }}
            </For>
        </pre>
    );
};

DiffViewer.displayName = "DiffViewer";
