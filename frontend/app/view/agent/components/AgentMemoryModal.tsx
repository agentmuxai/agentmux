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
    agentName: string;
    workingDirectory: string;
    onClose: () => void;
}

/**
 * Approximate the memory folder path using the same sanitize algorithm
 * Claude Code uses (`sessionStoragePortable.ts` — replace every non-
 * alphanumeric char with `-`, truncate at 200 + base36 hash suffix if
 * longer). Spec §5.2.
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
                <p class="agent-memory-modal-note">
                    Edits in this panel will write directly to disk and take
                    effect at the next session start.
                </p>
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
