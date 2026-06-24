// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentShellHistoryPanel — collapsible panel showing recently sent messages.
 * Expands above the composer strip when `shellOpen` is true.
 *
 * Content comes from the per-pane `shell-history` registry (not the reducer).
 * Each row is single-line truncated; clicking a row pre-fills + submits it.
 *
 * Spec: docs/specs/SPEC_AGENT_COMPOSER_STRIP_REDESIGN_2026_06_23.md §1.6.
 */

import { For, Show, type JSX } from "solid-js";
import type { Accessor } from "solid-js";

interface AgentShellHistoryPanelProps {
    /** Reactive accessor returning the history array (newest-first). */
    history: Accessor<string[]>;
    /** Called when the user clicks a history row — should pre-fill + send. */
    onResend: (message: string) => void;
    /** Dispatch ShellClose. */
    onClose: () => void;
}

export const AgentShellHistoryPanel = (props: AgentShellHistoryPanelProps): JSX.Element => {
    return (
        <div class="agent-shell-history">
            <div class="agent-shell-history-header">
                <span class="agent-shell-history-title">Shell History</span>
                <button
                    type="button"
                    class="agent-shell-history-close"
                    aria-label="Close shell history"
                    onClick={props.onClose}
                >
                    ×
                </button>
            </div>
            <div class="agent-shell-history-body">
                <Show
                    when={props.history().length > 0}
                    fallback={
                        <span class="agent-shell-history-empty">No messages sent yet.</span>
                    }
                >
                    <For each={props.history()}>
                        {(msg) => (
                            <button
                                type="button"
                                class="agent-shell-history-row"
                                title={msg}
                                onClick={() => props.onResend(msg)}
                            >
                                {msg}
                            </button>
                        )}
                    </For>
                </Show>
            </div>
        </div>
    );
};

AgentShellHistoryPanel.displayName = "AgentShellHistoryPanel";
