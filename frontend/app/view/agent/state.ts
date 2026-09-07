// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * View-local state for the agent widget, using SolidJS signals.
 *
 * IMPORTANT: All signals are instance-scoped (created per ViewModel
 * instance) to prevent state bleeding between multiple agent widgets.
 *
 * **What lives here, and what does not (A6 of issue #1549).** This file
 * used to hold a 19-signal mirror of the two agent-pane stores: the view
 * created a signal per reducer field, handed each setter to the store at
 * registration, and the store wrote changed fields back through them.
 * Adding a reducer field meant touching four files, and the view could
 * (and did) write a mirrored signal directly, bypassing the reducer that
 * owned the field.
 *
 * Reducer-owned state is now read straight from the pane model:
 *   - `model.state.<field>`  — every `AgentPaneState` field, reactive
 *   - `model.document()`     — the document nodes, reactive
 * (see `agent-pane-state-store.ts`'s `AgentPaneView` and
 * `agent-pane-model.ts`). There is no second copy to keep in sync.
 *
 * What remains here is genuinely view-local UI state that no reducer owns:
 * the collapse/pin sets, scroll position, selection and filter that
 * `AgentDocumentView` and the virtualizer manage.
 */

import { createSignal, type Accessor, type Setter } from "solid-js";
import { DocumentState } from "./types";

/**
 * A signal pair: [getter, setter]
 */
export type SignalPair<T> = [Accessor<T>, Setter<T>];

/**
 * View-local signals for a single agent widget instance.
 */
export interface AgentAtoms {
    /**
     * Collapse/pin sets, `expandedTools` hold, scroll position, selection
     * and filter — UI state owned by the document view, not by a reducer.
     * `useSnapshotPersistence` snapshots it and `onSnapshotOverlay`
     * restores it.
     */
    documentStateAtom: SignalPair<DocumentState>;
}

/**
 * A message sent to the backend that hasn't been acknowledged yet. Lives in
 * the reducer (`AgentPaneState.pending`); the type is defined here because
 * the composer / pending-panel components and the stream hook share it.
 * Each entry has a client-generated id so the `agent-message-accepted`
 * event can match it.
 */
export interface PendingMessage {
    id: string;
    text: string;
    createdAt: number;
    /**
     * True when this message was queued while a turn was already in-flight
     * (Submitting | Streaming | Interrupting). False for messages that
     * initiated the current turn (idle sends).
     *
     * The PendingMessagesPanel gates its visibility on this flag so that
     * idle-send messages never flash in the amber queued zone — only messages
     * genuinely sitting behind a running turn should appear there.
     * See docs/analysis/ANALYSIS_IDLE_SEND_RACE_2026_06_11.md.
     */
    enqueuedWhileBusy: boolean;
    /**
     * Set by the `PendingMessageFlushStarted` reducer arm, dispatched from
     * `flushHeldMessages` (`useAgentCommands.ts`) right before its actual
     * `deliverToBackend` call — the moment delivery genuinely begins, not
     * merely "the turn this message was queued behind has ended." Those two
     * moments can be far apart when a controller refresh is still deferred
     * (`flushHeldMessages` bails without draining `heldQueue` in that case);
     * an earlier version of this flag was set unconditionally on `TurnEnd`
     * and could read `flushing` while delivery hadn't actually started yet
     * (codex P2 on PR #2970). Once set, the backend's async
     * `agent-message-accepted` ack (which is what actually removes this
     * entry, via `usePendingMessageAcceptance.ts`) or a delivery failure
     * (`PendingMessageRejected`) is the only thing left to wait for. Before
     * this flag existed, the panel had no way to distinguish "still behind a
     * running turn" from "turn ended, delivery in flight" — it read the same
     * "Queued — sends at the agent's next step" copy in both cases, which is
     * actively wrong once the turn is Done (there is no "next step" left to
     * wait for) and is what let "Queued message" visibly coexist with
     * "Worked" for the length of that ack round-trip. See
     * docs/specs/SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md Phase 1.
     */
    flushing?: boolean;
}

/**
 * Factory function: create fresh view-local signals for a new agent widget
 * instance.
 */
export function createAgentAtoms(): AgentAtoms {
    return {
        documentStateAtom: createSignal<DocumentState>({
            collapsedNodes: new Set<string>(),
            pinnedNodes: new Set<string>(),
            expandedTools: new Set<string>(),
            scrollPosition: 0,
            selectedNode: null,
            filter: {
                showThinking: false,
                showSuccessfulTools: true,
                showFailedTools: true,
                showIncoming: true,
                showOutgoing: true,
            },
        }),
    };
}
