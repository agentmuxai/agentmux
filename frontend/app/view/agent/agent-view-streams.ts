// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What feeds an agent pane while it runs: the controller's status events, the re-poll on refocus,
 * the activity titles, the next-prompt suggestion, the narration lines and the subprocess stream
 * that becomes the document. Split out of AgentPresentationView; the hooks are called in the order
 * they were there.
 */

import { makeWindowFocusSignal } from "@/app/window/window-focus";
import type { AgentViewModel } from "./agent-model";
import type { PaneFoundation } from "./agent-view-foundation";
import type { PaneLate } from "./agent-view-late";
import type { TurnAndLaunch } from "./agent-view-turn";
import type { HistoryAndFeed } from "./agent-view-transcript";
import { useAgentActivitySummary } from "./hooks/useAgentActivitySummary";
import { useAmbientNarration } from "./hooks/useAmbientNarration";
import { useBackgroundHandoffNarration } from "./hooks/useBackgroundHandoffNarration";
import { useBlockActivity } from "./hooks/useBlockActivity";
import { useControllerStatusEvents } from "./hooks/useControllerStatusEvents";
import { useNextPromptSuggestion } from "./hooks/useNextPromptSuggestion";
import { useFocusRepoll } from "./hooks/useTurnReconciliation";
import { useAgentStream } from "./useAgentStream";

export function useTurnStreams(p: { found: PaneFoundation; model: AgentViewModel; feed: HistoryAndFeed; turn: TurnAndLaunch; late: PaneLate }) {
    const { found, model, late } = p;
    const { paneModel, outputFormat, agentName, providerKey } = found;
    const { log } = found.shell;
    const { turnJustEndedAtom, trackTurnJustEnded, reconcileTurnActive } = p.turn.turnConfirmation;

    // Log controllerstatus events and reconcile the turn phase from the live `turn_active`, both ways: the
    // mount-time GetControllerStatus is one-shot, so a turn that ended with its session_end missed would leave
    // the phase stuck at Streaming.
    useControllerStatusEvents({
        blockId: model.blockId,
        log,
        onTurnActive: (active) => {
            reconcileTurnActive(active);
            trackTurnJustEnded(active);
        },
        // An ACTIVE turn proves the CLI is alive and running: clear a "Retry Login" notice and an auth failure
        // row left by the mount-time launch flow, which would outlive the failure and fast-fail every send.
        // An idle heartbeat proves nothing about the credential, so only an active turn counts. A turn opens
        // when the message is sent, before any model call: so for a provider with no sign-in check (Muse) it
        // clears the notice but does not turn the strip's tag to "Logged in" while the pane says it isn't.
        onActiveTurnConfirmed: () => late.declareAuthHealthy((found.provider()?.authCheckCommand?.length ?? 0) > 0),
    });

    // Re-poll turn state on refocus, to recover a missed turn-end push (hooks/useTurnReconciliation.ts).
    useFocusRepoll({
        blockId: model.blockId,
        windowFocused: makeWindowFocusSignal(),
        trackTurnJustEnded,
        flushPendingControllerRefresh: () => late.commands.flushPendingControllerRefresh(),
    });

    // OSC window titles -> `term:osc_title`, the free fallback for the activity summary (readActivitySummary()
    // in agent-model.ts).
    useBlockActivity({ blockId: model.blockId });

    // A Haiku session-goal title -> `term:ambient_summary`, re-evaluated when the user submits a message and
    // preferred over the OSC title (SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md).
    useAgentActivitySummary({
        blockId: model.blockId,
        turnPhase: () => paneModel.state.turnPhase,
        getRootWidth: () => late.root?.offsetWidth,
    });

    // Ghost-text next-prompt suggestion. AgentFooter supplies the composer's emptiness once it mounts; until
    // then it counts as empty, and no suggestion exists before a completed turn anyway.
    useNextPromptSuggestion({
        blockId: model.blockId,
        turnPhase: () => paneModel.state.turnPhase,
        turnJustEndedAtom,
        isComposerEmpty: () => late.composerIsEmpty?.() ?? true,
        document: paneModel.document,
    });

    // Subscribe to subprocess output and parse it into DocumentNodes. The document store's reducer owns dedup
    // against in-flight history loads and the truncate-suppress invariant.
    const pendingMessages = () => paneModel.state.pending;
    // Short lines from AgentMux about its own actions (a tool call the harness detached), inserted in-flow.
    // Straight to the document store, not the stream-flush queue: that queue reports its batch as turn
    // activity, and AgentMux talking to itself is no evidence the model is doing anything.
    useAmbientNarration(model.blockId, (node) => {
        paneModel.dispatchDoc({ type: "StreamFlush", newNodes: [node], updatedNodes: [] });
    });
    // The hand-off to the background and its end, in the model's own words for the call.
    useBackgroundHandoffNarration(paneModel.document, (nodes) => {
        paneModel.dispatchDoc({ type: "StreamFlush", newNodes: nodes, updatedNodes: [] });
    });
    // Handed to ActivityDock for background tasks the transcript has no record of
    // (REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md, Tier 1).
    const backgroundTasksAtom = useAgentStream({
        blockId: model.blockId,
        transcriptSettle: p.feed.transcriptSettle,
        model: paneModel,
        outputFormat: outputFormat(),
        documentNodes: paneModel.document,
        turnPhase: () => paneModel.state.turnPhase, // to see a user-initiated stop and append "⏹ Interrupted by user"
        compacting: () => paneModel.state.compacting, // pushes the "Compacting conversation…" node whenever it is set
        pendingMessages,
        enabled: true,
        provider: providerKey(), // attributes completed-turn tokens to its row in the token-usage store
        agentName: agentName(), // jekt direction on the live stream: FROM == this agent is an outgoing bubble
        // A turn that starts from the queue-drain path re-engages auto-scroll.
        onTurnStartFromQueue: () => late.scrollToBottom?.("queued-turn"),
        // A commit point for the one-way scroll path: the pending panel leaving, even mid-turn.
        onPendingLeft: () => late.scrollToBottom?.("accepted"),
    });

    return { backgroundTasksAtom, pendingMessages };
}
