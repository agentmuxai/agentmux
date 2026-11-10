// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The transcript an agent pane shows: the history load and the live feed behind it, and the
 * document they make, with the history link, gap rows and resume preflight injected and the
 * pane's view applied. Split out of AgentPresentationView; the hooks are called in the order they
 * were there.
 */

import { onCleanup, createMemo, type Accessor } from "solid-js";
import { LIVE_FEED_RESTORE_BYTES } from "../../agent-model/live-feed";
import type { DocumentNode } from "../../agent-model/types";
import type { AgentViewModel } from "./agent-model";
import { createAgentViewControls, registerAgentViewControls } from "./agent-view-controls";
import type { PaneFoundation } from "./agent-view-foundation";
import { markResolvedAuthErrors } from "./failure/resolved-auth-errors";
import { useHistoryPagination } from "./hooks/useHistoryPagination";
import { useLiveFeedRollOff } from "./hooks/useLiveFeedRollOff";
import { usePaneReveal } from "./hooks/usePaneReveal";
import { useResumePreflight } from "./hooks/useResumePreflight";
import { useSnapshotPersistence } from "./hooks/useSnapshotPersistence";
import { injectGapRows, injectHistoryLink } from "./inject-history-link";
import { buildResumePreflightNode, injectResumePreflight } from "./inject-resume-preflight";
import { createTranscriptSettleLatch } from "./transcript-cursor";
import { withoutPanelJekts } from "./messages/messages-state";
import { deriveDisplayNodes, isForcedRow } from "./virtualization/verbosity";

export type HistoryAndFeed = ReturnType<typeof useHistoryAndFeed>;

/** The reveal, the history load and the live feed that rolls turns off into it. */
export function useHistoryAndFeed(found: PaneFoundation, model: AgentViewModel, agentId: string) {
    const { paneModel, outputFormat, agentName, agentAtoms, block, renderDormant } = found;
    const { log } = found.shell;

    // AgentDocumentView registers its viewState.markHistoryReady here on mount, so useHistoryPagination can signal
    // "history done" into the view state that gates the streaming buffer's enter animation.
    let historyReadyFn: (() => void) | undefined;
    // The loading cover and when the pane may appear (hooks/usePaneReveal.ts).
    // Before useHistoryPagination below, so a pane mounting without a My
    // Agents click (startup restore) still records its history phases.
    const reveal = usePaneReveal({ blockId: model.blockId, agentName });
    const readiness = reveal.readiness;
    // Where the history load ended, handed to the live stream so it places
    // its records after that history (Phase 5a-4, transcript-cursor.ts).
    const transcriptSettle = createTranscriptSettleLatch();
    const history = useHistoryPagination({
        blockId: model.blockId,
        transcriptSettle,
        model: paneModel,
        outputFormat,
        agentName, // jekt direction on replay: FROM == this agent is an outgoing bubble
        definitionId: agentId,
        // Only with a turn cap set (by default the feed is bounded by size):
        // K + the turn in flight; Claude only. Read after this body runs.
        restoreTurns: () =>
            liveFeed.liveFeedOn() && outputFormat() === "claude-stream-json" && Number.isFinite(liveFeed.liveFeedTurns) ? liveFeed.liveFeedTurns + 1 : undefined,
        restoreBytes: () => (liveFeed.liveFeedOn() ? LIVE_FEED_RESTORE_BYTES : undefined), // what the feed keeps, by size
        fillPaused: () => renderDormant() || !liveFeed.followingBottom(), // see the option; a prewarming pane fills
        onHistoryReady: () => {
            historyReadyFn?.();
            // A pane opens with K turns, not the load window's worth (§6.9).
            liveFeed.scheduleRollOff();
            // Start the fade once the history has actually painted.
            reveal.startPaintWait();
        },
        // Schema v2: apply DocumentState + pane overlay after NDJSON replay.
        onSnapshotOverlay: ({ documentState, detailsOpen }) => {
            const [, setDocState] = agentAtoms().documentStateAtom;
            setDocState((prev) => ({ ...prev, ...documentState }));
            if (typeof detailsOpen === "boolean") {
                // Reducer-owned field: restore through the reducer, not by
                // writing a signal behind its back (A6 of issue #1549).
                paneModel.dispatchPane({ type: detailsOpen ? "DetailsExpand" : "DetailsCollapse" }, "system");
            }
        },
        log,
    });

    // Will the next spawn continue the conversation shown, or start a new one? Asked on mount, before the first
    // send (hooks/useResumePreflight.ts).
    const resumePreflight = useResumePreflight(model.blockId);

    // The live feed: the turn in flight plus the last K finished turns; older ones roll off into History
    // (hooks/useLiveFeedRollOff.ts). Created after the history hook, whose callbacks above call into it only
    // once this body has finished.
    const liveFeed = useLiveFeedRollOff({
        blockId: model.blockId,
        paneModel,
        outputFormat,
        block,
        agentAtoms,
        hidden: renderDormant,
        history,
    });

    return {
        reveal,
        readiness,
        transcriptSettle,
        history,
        resumePreflight,
        liveFeed,
        registerHistoryReady: (fn: () => void) => {
            historyReadyFn = fn;
        },
    };
}

/** What the pane shows: the document under the pane's view, kept saved. */
export function useShownDocument(found: PaneFoundation, model: AgentViewModel, agentId: string, feed: HistoryAndFeed) {
    const { paneModel, agentAtoms } = found;
    const { log } = found.shell;
    const { history, resumePreflight } = feed;
    const { gapsBefore, earlierHistoryAvailable, earlierTurnsKnown } = feed.liveFeed;

    // The document with the history link, gap rows and resume preflight injected as ordinary scrolling nodes.
    // The views only read it; writes go through `paneModel.dispatchDoc`.
    const displayDocument: Accessor<DocumentNode[]> = () =>
        injectResumePreflight(
            injectHistoryLink(injectGapRows(paneModel.document(), gapsBefore()), earlierHistoryAvailable(), {
                earlierTurns: earlierTurnsKnown(),
            }),
            buildResumePreflightNode(
                paneModel.document(),
                resumePreflight.result(),
                resumePreflight.showSteps(),
            ),
        );

    // What the pane shows: the document under the pane's view (virtualization/verbosity.ts). Full is the document itself.
    const viewControls = createAgentViewControls({
        blockId: model.blockId,
        nodes: displayDocument,
        setDocumentState: (update) => agentAtoms().documentStateAtom[1](update),
    });
    onCleanup(registerAgentViewControls(model.blockId, viewControls));
    // The Messages panel holds the incoming jekts: the conversation leaves them out (messages/messages-state.ts),
    // before the view folds rows into runs, so a hidden run never counts or holds a jekt that the panel has.
    const shownDocument = createMemo(() =>
        deriveDisplayNodes(
            withoutPanelJekts(markResolvedAuthErrors(displayDocument()), model.blockId, isForcedRow),
            viewControls.resolved().view,
            agentAtoms().documentStateAtom[0]()
        )
    );

    // Read-only; writes dispatch through the document store so its slot state stays in sync.
    const getDocument = paneModel.document;

    // State persistence (RFC #857, hooks/useSnapshotPersistence.ts).
    useSnapshotPersistence({
        blockId: model.blockId,
        definitionId: agentId,
        getAtoms: agentAtoms,
        getDocument,
        getDetailsOpen: () => paneModel.state.detailsOpen,
        snapshotIsForeignBlock: () => history.snapshotIsForeignBlock(),
        log,
    });

    return { displayDocument, viewControls, shownDocument, getDocument };
}
