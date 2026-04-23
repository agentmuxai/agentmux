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
 * See AGENT_PANE_QUEUED_MESSAGE_FEEDBACK_SPEC.md.
 */

import { For, Show, type Accessor, type JSX } from "solid-js";
import type { PendingMessage } from "../state";

interface PendingMessagesPanelProps {
    pendingMessages: Accessor<PendingMessage[]>;
}

export const PendingMessagesPanel = (props: PendingMessagesPanelProps): JSX.Element => (
    <Show when={props.pendingMessages().length > 0}>
        <div class="agent-pending-zone">
            <div class="agent-pending-header">
                <span class="agent-spinner-dot" />
                <span>
                    Queued — will send when the agent is idle (
                    {props.pendingMessages().length}
                    {props.pendingMessages().length === 1 ? " message" : " messages"})
                </span>
            </div>
            <For each={props.pendingMessages()}>
                {(msg) => (
                    <div class="agent-pending-item" data-message-id={msg.id}>
                        <pre>{msg.text}</pre>
                    </div>
                )}
            </For>
        </div>
    </Show>
);

PendingMessagesPanel.displayName = "PendingMessagesPanel";
