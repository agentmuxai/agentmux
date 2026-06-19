// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentMemoryModalPanel — pane-scoped modal for the agent's native
 * memory folder (~/.claude/projects/<sanitized>/memory/).
 *
 * Phase 1: placeholder — shows the memory folder path so users know
 * where Claude writes facts. Full file browser + editor lands in Phase 3
 * once the Rust backend RPCs (NativeMemoryList / Read / Write) ship.
 *
 * Spec: SPEC_AGENT_PANE_MEMORY_IDENTITY_MODALS_2026_06_19.md §5.
 */

import { type JSX } from "solid-js";

interface AgentMemoryModalPanelProps {
    workingDirectory: string;
    onClose: () => void;
}

/**
 * Build a display-only approximation of the memory folder path.
 * Claude Code's real sanitize algorithm replaces every non-alphanumeric
 * char with `-` then, if longer than 200 chars, truncates and appends a
 * base36 djb2 hash suffix. This function applies only the character
 * replacement and truncates at 48 chars for display; paths longer than
 * 48 sanitized chars will differ from the real folder name on disk.
 * Phase 3 will resolve the path via the backend RPCs instead.
 */
function previewMemoryPath(workDir: string): string {
    if (!workDir) return "~/.claude/projects/…/memory/";
    const sanitized = workDir.replace(/[^a-zA-Z0-9]/g, "-");
    const display = sanitized.length > 48 ? sanitized.slice(0, 48) + "…" : sanitized;
    return `~/.claude/projects/${display}/memory/`;
}

export const AgentMemoryModalPanel = (props: AgentMemoryModalPanelProps): JSX.Element => {
    return (
        <div class="agent-memory-modal-body">
            <div class="agent-memory-modal-coming-soon">
                <p class="agent-memory-modal-heading">
                    Native memory browser — coming soon
                </p>
                <p class="agent-memory-modal-desc">
                    Claude writes facts and session discoveries here across
                    sessions. This panel will let you view, edit, and prune
                    every file in the memory folder.
                </p>
                <p class="agent-memory-modal-path-label">Memory folder (mirrored path):</p>
                <code class="agent-memory-modal-path" title={props.workingDirectory}>
                    {previewMemoryPath(props.workingDirectory)}
                </code>
            </div>
            <div class="agent-modal-footer">
                <button
                    class="agent-modal-done-btn"
                    data-modal-dismiss
                    onClick={props.onClose}
                >
                    Close
                </button>
            </div>
        </div>
    );
};

AgentMemoryModalPanel.displayName = "AgentMemoryModalPanel";
