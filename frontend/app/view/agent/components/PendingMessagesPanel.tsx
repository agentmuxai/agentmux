// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PendingMessagesPanel — messages queued on the frontend waiting for the
 * backend to accept them. Rendered between the conversation document and
 * the composer so the user sees their input held in a distinct zone (not
 * interleaved with the agent's replies).
 *
 * When the backend emits `agent-message-accepted` for an id, `useAgentStream`
 * removes the entry from here and appends the matching `user_message` node
 * into the conversation document — the color/border shift from amber to
 * accent blue is the user's visible "accepted" signal.
 *
 * Only messages with `enqueuedWhileBusy: true` are shown — those are the
 * ones the user sent while a turn was already running. Idle-send messages
 * (`enqueuedWhileBusy: false`) initiated the current turn and must never
 * appear here, regardless of phase. This closes the race where the panel
 * would flash between Streaming promotion and agent-message-accepted.
 * See docs/analysis/ANALYSIS_IDLE_SEND_RACE_2026_06_11.md.
 *
 * See AGENT_PANE_QUEUED_MESSAGE_FEEDBACK_SPEC.md.
 */

import { For, Show, createMemo, type Accessor, type JSX } from "solid-js";
import type { PendingMessage } from "../state";

interface PendingMessagesPanelProps {
    pendingMessages: Accessor<PendingMessage[]>;
    /** True when the user should be offered a "Send now" shortcut —
     *  typically: a turn is running AND the queue is non-empty. */
    showSendNow?: Accessor<boolean>;
    /** Fires when the user clicks "Send now". Caller is expected to
     *  SIGINT the running turn so the queued messages drain. */
    onSendImmediately?: () => void;
}

export const PendingMessagesPanel = (props: PendingMessagesPanelProps): JSX.Element => {
    // Only surface messages that were genuinely queued behind a running turn.
    // Idle-send messages (enqueuedWhileBusy: false) are transparent — they
    // initiated the current turn and should never appear in this zone.
    const queuedMessages = createMemo(() =>
        props.pendingMessages().filter((m) => m.enqueuedWhileBusy),
    );
    return (
        <Show when={queuedMessages().length > 0}>
            <div class="agent-pending-zone">
                <div class="agent-pending-header">
                    <span class="agent-spinner-dot" />
                    <span class="agent-pending-header-text">
                        Queued — will send when the agent is idle (
                        {queuedMessages().length}
                        {queuedMessages().length === 1 ? " message" : " messages"})
                    </span>
                    <Show when={props.showSendNow?.()}>
                        <button
                            type="button"
                            class="agent-send-immediately-btn"
                            onClick={() => props.onSendImmediately?.()}
                            title="Stop the current turn and process the queue now"
                        >
                            <span class="agent-send-immediately-icon">⏭</span>
                            <span>Send now</span>
                        </button>
                    </Show>
                </div>
                <For each={queuedMessages()}>
                    {(msg) => (
                        <div class="agent-pending-item" data-message-id={msg.id}>
                            <pre>{msg.text}</pre>
                        </div>
                    )}
                </For>
            </div>
        </Show>
    );
};

PendingMessagesPanel.displayName = "PendingMessagesPanel";
