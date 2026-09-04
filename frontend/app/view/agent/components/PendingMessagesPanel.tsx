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
 *
 * No "Send now" button — Esc on an empty composer delivers the queue to the
 * live agent immediately instead (mirrors Claude Code CLI's "stop and
 * consider this now" gesture). See
 * docs/specs/SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06.md.
 */

import { For, Show, createMemo, type Accessor, type JSX } from "solid-js";
import type { PendingMessage } from "../state";

interface PendingMessagesPanelProps {
    pendingMessages: Accessor<PendingMessage[]>;
}

export const PendingMessagesPanel = (props: PendingMessagesPanelProps): JSX.Element => {
    // Only surface messages that were genuinely queued behind a running turn.
    // Idle-send messages (enqueuedWhileBusy: false) are transparent — they
    // initiated the current turn and should never appear in this zone.
    const queuedMessages = createMemo(() =>
        props.pendingMessages().filter((m) => m.enqueuedWhileBusy),
    );
    // SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md Phase 1: once
    // flushHeldMessages has actually started delivering an entry
    // (`PendingMessageFlushStarted`, dispatched right before its
    // `deliverToBackend` call), "sends at the agent's next step" is no
    // longer true — delivery is already in flight — and the recall/edit
    // affordance no longer applies to it. Only the still-open (non-flushing)
    // count keeps the original copy; a flushing-only queue gets its own,
    // honest wording instead of implying it's still waiting.
    const stillQueuedCount = createMemo(
        () => queuedMessages().filter((m) => !m.flushing).length,
    );
    return (
        <Show when={queuedMessages().length > 0}>
            <div class="agent-pending-zone">
                <div class="agent-pending-header">
                    <span class="agent-spinner-dot" />
                    <span class="agent-pending-header-text">
                        <Show
                            when={stillQueuedCount() > 0}
                            fallback={
                                <>
                                    Sending — reaching the agent any moment (
                                    {queuedMessages().length}
                                    {queuedMessages().length === 1 ? " message" : " messages"})
                                </>
                            }
                        >
                            Queued — sends at the agent's next step; ↑ to edit, Esc to send now (
                            {stillQueuedCount()}
                            {stillQueuedCount() === 1 ? " message" : " messages"})
                        </Show>
                    </span>
                </div>
                <For each={queuedMessages()}>
                    {(msg) => (
                        <div
                            class="agent-pending-item"
                            classList={{ "agent-pending-item-flushing": !!msg.flushing }}
                            data-message-id={msg.id}
                        >
                            <pre>{msg.text}</pre>
                        </div>
                    )}
                </For>
            </div>
        </Show>
    );
};

PendingMessagesPanel.displayName = "PendingMessagesPanel";
