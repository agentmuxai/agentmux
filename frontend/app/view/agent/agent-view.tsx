// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignInNotice } from "./sign-in-notice";
import { createSystemNotifier } from "./system-notification";
import { isAuthFailure, isStopping } from "@/app/store/agent-pane-state/types";
import { AgentPaneProviders } from "./agent-media";
import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { resolveContextMenuRegion } from "@/app/block/context-menu-region";
import { showCopyContextMenu } from "@/app/store/contextmenu";
import { getBlockMetaKeyAtom } from "@/app/store/global";
import { paneZoom } from "@/app/store/pane-zoom-local";
import { PaneLoadingCover } from "@/app/element/PaneLoadingCover";
import { ModalLayerContext } from "@/app/element/modal-layer";
import { makeWindowFocusSignal } from "@/app/window/window-focus";
import { createMemo, onCleanup, onMount, Show, useContext, type Accessor, type JSX } from "solid-js";
import { createPromotionClock } from "./activity/promotion-clock";
import { useAttachedTaskAxis } from "./activity/useAttachedTaskAxis";
import { AgentProgressBar } from "./components/AgentProgressBar";
import { useWorkingIndicator } from "./hooks/useWorkingIndicator";
import { quickForkAgent } from "./quick-fork";
import { askSideQuestion } from "./btw";
import type { AgentViewModel } from "./agent-model";
import "./agent-view.scss";
import { AgentBottomPanels } from "./components/AgentBottomPanels";
import { AgentPreparingRow } from "./components/AgentPreparingRow";
import { AgentComposerStrip } from "./components/AgentComposerStrip";
import { AgentShellDrawer } from "./components/AgentShellDrawer";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { HistoryLoadingRow } from "./components/HistoryLoadingRow";
import { AgentFooter } from "./components/AgentFooter";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { BtwOverlay } from "./components/BtwOverlay";
import { SlashCommandPicker } from "./components/SlashCommandPicker";
import { SlashHelpPanel } from "./components/SlashHelpPanel";
import { usePaneReveal } from "./hooks/usePaneReveal";
import { useLiveFeedRollOff } from "./hooks/useLiveFeedRollOff";
import { useRenderDormant } from "./render-dormant";
import { LIVE_FEED_RESTORE_BYTES } from "./live-feed";
import { useShellLogBridge } from "./hooks/useShellLogBridge";
import { useContextReading } from "./hooks/useContextReading";
import { useTurnLedger } from "./hooks/useTurnLedger";
import { useFocusRepoll, useHeldMessageDelivery } from "./hooks/useTurnReconciliation";
import { useAmbientNarration } from "./hooks/useAmbientNarration";
import { useAgentActivitySummary } from "./hooks/useAgentActivitySummary";
import { useAgentCommands } from "./hooks/useAgentCommands";
import { useAgentControllerStatus } from "./hooks/useAgentControllerStatus";
import { useAgentDecisions } from "./hooks/useAgentDecisions";
import { useAgentDropAttach } from "./hooks/useAgentDropAttach";
import { lastUserMessage, useOfflineInput } from "./offline/useOfflineInput";
import { useAgentAccountBinding } from "./failure/useAgentAccountBinding";
import { useAuthHealth, useSyntheticAuthRow } from "./failure/useAuthHealth";
import { useAgentKeyboard } from "./hooks/useAgentKeyboard";
import { useAgentQuestions } from "./hooks/useAgentQuestions";
import { useBlockActivity } from "./hooks/useBlockActivity";
import { useControllerStatusEvents } from "./hooks/useControllerStatusEvents";
import { createTurnConfirmation } from "./hooks/turn-confirmation";
import { useHistoryPagination } from "./hooks/useHistoryPagination";
import { createTranscriptSettleLatch } from "./transcript-cursor";
import { useInSessionSearch } from "./hooks/useInSessionSearch";
import { useNextPromptSuggestion } from "./hooks/useNextPromptSuggestion";
import { computeTermSizeFromEl, usePtyWidth } from "./hooks/usePtyWidth";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { useScrollToNode } from "./hooks/useScrollToNode";
import { useSnapshotPersistence } from "./hooks/useSnapshotPersistence";
import { injectGapRows, injectHistoryLink } from "./inject-history-link";
import { buildResumePreflightNode, injectResumePreflight } from "./inject-resume-preflight";
import { useResumePreflight } from "./hooks/useResumePreflight";
import { openOrFocusHistoryTab } from "./open-history-tab";
import { getProvider } from "./providers";
import { createAgentAtoms } from "./state";
import type { DocumentNode } from "./types";
import { ShutdownOverlay } from "./shutdown/ShutdownOverlay";
import { useAgentStream } from "./useAgentStream";
import { clearFailure } from "./failure/failure-writer";
import { registerAgentViewCommands } from "./agent-view-commands";
import { createAgentViewControls, registerAgentViewControls, viewChipLabel } from "./agent-view-controls";
import { AgentViewPopover } from "./components/AgentViewPopover";
import { deriveDisplayNodes } from "./virtualization/verbosity";
import { paneMeta } from "./pane-meta";
import { startupOnReady } from "./startup/startupOnReady";
import { useFailureRow } from "./failure/useFailureRow";
import { usePaneRegistration } from "./hooks/usePaneRegistration";
import { useShellDrawerLifecycle } from "./hooks/useShellDrawerLifecycle";
import { useTurnEndSweep } from "./hooks/useTurnEndSweep";

// The palette's "Agent view" commands exist from the first agent pane on; they are available only while one is focused.
registerAgentViewCommands();

export const AgentPresentationView = ({
    model,
    agentId,
    agentDefinitions,
    progressBarMount,
}: {
    model: AgentViewModel;
    agentId: string;
    /** The wrapper's definition list: a second `useAgentDefinitions()` here would repeat its RPC and subscription. */
    agentDefinitions: () => AgentDefinition[];
    /** Where the progress bar portals: a node AgentPaneChrome owns, bridged through the model (`progressBarMount` in agent-model.ts). */
    progressBarMount: () => HTMLDivElement | undefined;
}): JSX.Element => {
    const block = model.blockAtom;
    const meta = paneMeta(block, agentId);
    const { providerKey, outputFormat, agentName } = meta;
    // True while the user can't see this tab: hidden in the pane tab strip, or its window tab isn't displayed
    // (SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md). Render work pauses; the timed decisions (the question
    // timeout, the failure auto-retry) run on.
    const visibility = usePaneTabVisibility(model.blockId);
    const hidden = (): boolean => visibility() !== "active";
    const renderDormant = useRenderDormant(hidden, model.blockId); // `hidden`, or the window itself (minimized)
    const provider = () => getProvider(providerKey());
    // For the identity and memory modal requests.
    const currentAgent = createMemo(() => agentDefinitions().find((a) => a.id === agentId));

    // The right-click menu opens its modals in this pane's own layer (agent-body-menu.ts).
    const modalLayer = useContext(ModalLayerContext);
    onMount(() => {
        model._openModal = (req) => modalLayer?.open(req);
    });
    onCleanup(() => {
        model._openModal = null;
    });

    const agentAtoms = createMemo(() => createAgentAtoms());
    const { paneModel, layoutView } = usePaneRegistration({ blockId: model.blockId, agentId, agentName });

    // The context meter and its Swarm mirror (hooks/useContextReading.ts).
    const { reading: contextReading, note: contextNote, autoCompact } = useContextReading(model.blockId, paneModel, block);
    // srv's turn ledger, so the working row covers the whole turn (hooks/useTurnLedger.ts).
    useTurnLedger(model.blockId, paneModel);

    // The shell drawer's log bridge: which log lines reach the shell terminal, and the backlog replayed when it mounts.
    const { log, onTermReady: handleShellTermReady, onTermDispose: handleShellTermDispose, clearTermWrite, withShellFocus } = useShellLogBridge(model.blockId);
    const { handleShellExited } = useShellDrawerLifecycle({ blockId: model.blockId, shellSubBlockId: meta.shellSubBlockId, paneModel, clearTermWrite });

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
    const { liveFeedOn, gapsBefore, earlierHistoryAvailable, earlierTurnsKnown } = liveFeed;

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
    const shownDocument = createMemo(() => deriveDisplayNodes(displayDocument(), viewControls.resolved().view, agentAtoms().documentStateAtom[0]()));

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
        hidden: renderDormant, log,
    });

    // Permission decision queue + decide handler. See hooks/useAgentDecisions.ts.
    const { pendingDecisions, handleDecide } = useAgentDecisions({
        blockId: model.blockId,
        getDocument,
        log,
    });

    // AskUserQuestion queue and answers (hooks/useAgentQuestions.ts). `sendMessage` is a thunk: the follow-up
    // fallback runs later, from the async catch.
    const { pendingQuestions, handleAnswer, handleCancel } = useAgentQuestions({
        blockId: model.blockId,
        getDocument,
        sendMessage: (message: string) => commands.sendWithState(message),
        log,
    });

    // The root element: read by the PTY width, file drop and launch term-size hooks; assigned during render.
    let rootRef: HTMLDivElement | undefined;

    // Whether the backend's turn is confirmed active or idle, and the turn-end edge (hooks/turn-confirmation.ts).
    // `commands` is read only from async event callbacks, after setup has run.
    const turnConfirmation = createTurnConfirmation({
        reconcile: (active) =>
            paneModel.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active }, "system"),
        onTurnEnded: () => void commands.flushPendingControllerRefresh(),
    });
    const { turnJustEndedAtom, reconcileTurnActive, trackTurnJustEnded } = turnConfirmation;

    useTurnEndSweep({ turnEnded: turnJustEndedAtom, paneModel, onTurnEnded: liveFeed.scheduleRollOff });

    const postSystemNotification = createSystemNotifier(paneModel.document, (action) => paneModel.dispatchDoc(action));

    const signIn = createSignInNotice(postSystemNotification);
    // On a first connect (no existing session) the startup sequence is the opening turn.
    const startup = startupOnReady({
        blockId: model.blockId,
        agentId,
        block,
        currentAgent,
        peerAgents: agentDefinitions,
        providerDisplayName: () => provider()?.displayName ?? providerKey(),
        send: (payload) => commands.sendWithState(payload),
        log,
    });
    const status = useAgentControllerStatus({
        blockId: model.blockId,
        provider,
        log,
        onLoginSuccess: (email) => {
            signIn.answered(email ? `Logged in as **${email}**` : "Login successful");
        },
        onNotify: (text, style, kind) => (kind === "sign-in" ? signIn.needed(text, style) : postSystemNotification(text, style)),
        onAccountBound: signIn.bound,
        onReady: startup,
        // A relogin or terminal login refreshed the credential: retry the failed turn, so the agent recovers in one click.
        onRecovered: () => {
            // Another recovery flow is still running: leave the failure row and `loginWaiting` alone. This
            // flow's credential is good, but a retry now could start a turn the other flow's restart would
            // kill, and if that flow fails nothing retries, so the row is the user's only way back. If it
            // succeeds, its own onRecovered runs this check with `loginWaiting()` false and does the retry.
            if (status.loginWaiting()) return;
            // Clear the failure first: a send reads the failure showing before TurnStart, so a stale auth
            // failure would reject the very resend recovery enabled. Every caller that gets here wants a
            // retry; loginViaTerminal's no-retry path calls onReady() instead.
            clearFailure(paneModel, { reason: "recovered", system: true });
            retryLastTurn();
        },
        getInitialTermSize: () => computeTermSizeFromEl(rootRef),
        // Mount-time turn-phase reconciliation (REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md Finding 1).
        // Through paneModel, which never throws: this can resolve before registerPane runs for a pane mid-mount.
        onControllerStatus: (rts) => {
            reconcileTurnActive(!!rts.turn_active);
        },
    });

    // Hold the reveal until the launch flow is past the phases that can pop the auth panel in (hooks/usePaneReveal.ts).
    reveal.connectLaunchStatus(status);

    // `isLoading()` stays false through relogin() and loginViaTerminal(), where the agent is already ready, so
    // the working row, its Cancel, the progress bar and the composer strip also key on launchPhase().
    const showingLaunchActivity = () => status.isLoading() || status.launchPhase() != null;

    // The composer strip's logged-in/out tag. `authStatus()` is durable, but a mid-turn 401 shows first as an
    // auth failure row, and that overrides the tag at once.
    const loginStatus = createMemo((): "authenticated" | "unauthenticated" | "unknown" => {
        if (isAuthFailure(paneModel.state.failure)) return "unauthenticated";
        return status.authStatus();
    });

    onMount(() => {
        const provName = provider()?.displayName ?? providerKey();
        log("agent", `${agentName()} selected (provider: ${provName})`);
        if (meta.cwd()) log("env", `working directory: ${meta.cwd()}`);
        status.startLaunchFlow();
    });

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
        // An idle heartbeat proves nothing about the credential, so only an active turn counts.
        onActiveTurnConfirmed: () => declareAuthHealthy(),
    });

    // Re-poll turn state on refocus, to recover a missed turn-end push (hooks/useTurnReconciliation.ts).
    useFocusRepoll({
        blockId: model.blockId,
        windowFocused: makeWindowFocusSignal(),
        trackTurnJustEnded,
        flushPendingControllerRefresh: () => commands.flushPendingControllerRefresh(),
    });

    // OSC window titles -> `term:osc_title`, the free fallback for the activity summary (readActivitySummary()
    // in agent-model.ts).
    useBlockActivity({ blockId: model.blockId });

    // A Haiku session-goal title -> `term:ambient_summary`, re-evaluated when the user submits a message and
    // preferred over the OSC title (SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md).
    useAgentActivitySummary({
        blockId: model.blockId,
        turnPhase: () => paneModel.state.turnPhase,
        getRootWidth: () => rootRef?.offsetWidth,
    });

    // Ghost-text next-prompt suggestion. AgentFooter supplies the composer's emptiness once it mounts; until
    // then it counts as empty, and no suggestion exists before a completed turn anyway.
    let composerIsEmptyFn: (() => boolean) | null = null;
    useNextPromptSuggestion({
        blockId: model.blockId,
        turnPhase: () => paneModel.state.turnPhase,
        turnJustEndedAtom,
        isComposerEmpty: () => composerIsEmptyFn?.() ?? true,
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
    // Handed to ActivityDock for background tasks the transcript has no record of
    // (REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md, Tier 1).
    const backgroundTasksAtom = useAgentStream({
        blockId: model.blockId,
        transcriptSettle,
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
        onTurnStartFromQueue: () => scrollToBottomFn?.("queued-turn"),
        // A commit point for the one-way scroll path: the pending panel leaving, even mid-turn.
        onPendingLeft: () => scrollToBottomFn?.("accepted"),
    });

    // The list's scrollToBottom, assigned once AgentDocumentView mounts: the composer's onTyping and
    // useAgentCommands' onSent call it.
    let scrollToBottomFn: ((reason?: string) => void) | null = null;

    // The wall-clock instant a running Bash call is promoted to the dock, for
    // everything that depends on promotion (activity/promotion-clock.ts).
    const promotionTick = createPromotionClock(paneModel.document);

    // The busy predicate and its renderings (hooks/useWorkingIndicator.ts).
    const { paneBusy, workingRowVisible } = useWorkingIndicator({
        paneModel,
        showingLaunchActivity,
    });
    const workingRowLoading = paneBusy;

    // Attached-task axis dispatch (activity/useAttachedTaskAxis.ts).
    useAttachedTaskAxis({ blockId: model.blockId, paneModel, promotionTick });

    // Sending a message, the /login and /clear intercepts, and back-to-picker (hooks/useAgentCommands.ts).
    const commands = useAgentCommands({
        // The same whole predicate this view renders as `paneBusyForInput`, so the eager flush can't disagree.
        showingLaunchActivity,
        blockId: model.blockId,
        model: paneModel,
        block,
        provider,
        documentNodes: paneModel.document,
        log,
        setAuthUrl: status.setAuthUrl,
        canRetry: status.canRetry,
        loginWaiting: status.loginWaiting,
        setAuthNotice: status.setAuthNotice,
        notifyControllerHealthy: status.notifyControllerHealthy,
        forceControllerRefresh: status.forceControllerRefresh,
        beginRecoveryFlow: status.beginRecoveryFlow,
        endRecoveryFlow: status.endRecoveryFlow,
        isCancelled: status.isCancelled,
        resetCancelled: status.resetCancelled,
        // The last confirmed backend `turn_active`, from live controllerstatus events only.
        isBackendTurnActive: turnConfirmation.isBackendTurnActive,
        // Not `!isBackendTurnActive()`: that treats "never confirmed" as idle, and the deferred controller
        // restart is destructive, so it needs positive confirmation of idle.
        isBackendTurnConfirmedIdle: turnConfirmation.isBackendTurnConfirmedIdle,
        backToPicker: () => model.backToPicker(),
        quickFork: () => quickForkAgent(model), // /fork, like the context menu's Quick Fork
        askSideQuestion: (question: string) => askSideQuestion(model.blockId, question, paneModel.document()), // /btw
        onSent: () => scrollToBottomFn?.("sent"),
        pendingMessages,
    });

    // The failure row's Retry: re-send the last user message with its attachments. With none (the agent failed
    // on its first launch) relaunch the agent, rather than dismissing the row silently (spec §5.1).
    const retryLastTurn = () => {
        const last = lastUserMessage(getDocument());
        if (last) {
            void commands.sendWithState(last.text, last.attachments);
        } else {
            log("agent", "Retry — no prior message to re-send; relaunching the agent");
            void status.startLaunchFlow();
        }
    };
    // Input held while the model API is unreachable; a network-failed turn retried when it's back.
    const offline = useOfflineInput({
        blockId: model.blockId, paneModel, provider, agentId: () => agentId, send: commands.sendWithState, stop: commands.stopAgent,
    });
    // Pre-launch auth failure -> the "Not signed in" failure row (failure/useAuthHealth.ts).
    useSyntheticAuthRow({ canRetry: status.canRetry, paneModel });

    // Bind/switch account for this agent (failure/useAccountBinding.ts).
    const account = useAgentAccountBinding({
        blockId: model.blockId, providerId: () => provider()?.id, bindExistingAccount: status.bindExistingAccount,
        signedOut: () => loginStatus() === "unauthenticated", onLogin: () => void status.relogin({ retryAfterLogin: paneModel.state.failure?.turnAttempted ?? false }),
    });

    // Declaring auth healthy, and the auto-unblock after a bind from anywhere (failure/useAuthHealth.ts).
    const { declareAuthHealthy } = useAuthHealth({
        blockId: model.blockId,
        agentDefinitionId: () => getBlockMetaKeyAtom(model.blockId, "agentId")() as string | undefined,
        paneModel,
        status,
        refreshLinkedAccountId: account.refreshLinkedAccountId,
    });

    // The failure row and its actions: per-class buttons and a bounded auto-retry ladder for transient
    // throttling (SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16).
    const failureUI = useFailureRow({ model, paneModel, status, account, offline, retryLastTurn, send: commands.sendWithState, log });

    // Held "send now" messages go out at the next tool call or turn end
    // (hooks/useTurnReconciliation.ts).
    useHeldMessageDelivery({
        paneModel,
        hasHeldMessages: commands.hasHeldMessages,
        flushHeldMessages: commands.flushHeldMessages,
        flushPendingControllerRefresh: commands.flushPendingControllerRefresh,
    });

    // Jump command as a signal: AgentDocumentView reacts and scrolls inside its own container (hooks/useScrollToNode.ts).
    const scroll = useScrollToNode();

    // In-session search over the loaded document slice: matches, navigation, highlight.
    const search = useInSessionSearch({
        document: paneModel.document,
        jumpTo: scroll.jumpTo,
    });

    // Pane-scoped Ctrl+F: a second press closes the bar and clears the state (hooks/useAgentKeyboard.ts).
    useAgentKeyboard({
        blockId: model.blockId,
        onToggleSearch: () => (search.visible() ? search.close() : search.setVisible(true)),
        onCycleView: viewControls.cycle,
        onFollowDefaultView: viewControls.followDefault,
    });

    // Per-pane zoom: `term:zoom` in block meta, the key terminal panes use; the level just asked for while its
    // write is on its way, else the stored one. The universal zoom framework owns the input (keymodel.ts,
    // app.tsx) and the persistence, so this file has no zoom handlers of its own.
    const zoomFactor = createMemo(() => paneZoom(model.blockId, block()?.meta));

    // File drops: this pane's hook for the window-level controller (tray, or copy + `@filename`).
    useAgentDropAttach({
        blockId: model.blockId,
        rootRef: () => rootRef,
    });

    // Pane width -> PTY cols, so tools in the agent CLI wrap to the pane instead of the opening `cols: 80`
    // (hooks/usePtyWidth.ts). Lines already captured keep their wrap.
    usePtyWidth({
        blockId: model.blockId,
        elementRef: () => rootRef,
        log,
    });

    // A context-menu region under the click (the Shell drawer) owns its own menu. This handler runs first, being
    // a descendant of blockframe's, so without yielding a transcript selection would show a Copy for the wrong
    // text inside the terminal.
    const handleContextMenu = (e: MouseEvent) => {
        if (resolveContextMenuRegion(e.target, e.currentTarget as Element)) return;
        const sel = window.getSelection()?.toString();
        if (!sel) return;
        e.preventDefault();
        showCopyContextMenu([{ label: "Copy", value: sel }], e);
    };

    return (
        // Dormancy is provided at the subtree root, not threaded as a prop: its expensive consumer (MarkdownBlock)
        // sits four layers down, behind tuned virtualization code (agent-dormancy.tsx). `renderDormant` also
        // covers a hidden window and gates only rendering; the timers read `hidden`. The providers carry the cwd
        // for inline media too.
        <AgentPaneProviders dormant={renderDormant} block={block} agent={currentAgent} loadToolResult={liveFeed.loadToolResult}>
            {/* The pane-scope ModalLayer is in AgentBlockContent, this component's parent: it covers this view and the picker. */}
        <div
            ref={rootRef}
            class="agent-view agent-view--presentation"
            // `zoom` lives on `.agent-view-zoomed` below, not here: the Shell drawer must render unscaled. xterm
            // measures cells in visual px and reclaims width in layout px, and CSS `zoom` makes them disagree
            // (fractional cells, a misplaced link layer; xtermjs/xterm.js #2584). `--agent-pane-zoom` is published
            // here for other consumers (SPEC_AGENT_SHELL_DRAWER_ZOOM_COORDINATE_SPACE_2026_09_20.md).
            style={{ "--agent-pane-zoom": String(zoomFactor()) }}
            onContextMenu={handleContextMenu}
            tabIndex={-1}
        >
            {/* Covers the pane from mount until the initial history load resolves. A direct child of `.agent-view`,
                outside `.agent-view-zoomed`: it is `position: absolute; inset: 0`, so inside the zoomed wrapper it
                would not cover the Shell drawer (a sibling), and the pane zoom would scale it
                (REPORT_AGENT_PANE_LOADING_UI_2026_09_20.md §F). */}
            <PaneLoadingCover phase={readiness.phase} />
            {/* Shutdown log while the pane closes in place (SPEC_AGENT_SELF_QUIT_2026_09_24.md §5.5). */}
            <ShutdownOverlay blockId={model.blockId} agentName={agentName()} />
            <div class="agent-view-zoomed" style={{ zoom: zoomFactor() }}>
            {/* Portaled above the tab strip; see components/AgentProgressBar.tsx. */}
            <AgentProgressBar
                mount={progressBarMount}
                active={paneBusy()}
                stopping={isStopping(paneModel.state.turnPhase)}
            />
            {/* /btw side-question overlay: ephemeral, floats over the whole pane (styles/_btw.scss), not gated on
                the turn state, so it stays usable while a turn streams. */}
            <Show when={commands.btwOverlay()}>
                {(state) => (
                    <BtwOverlay
                        blockId={model.blockId}
                        askId={state().askId}
                        question={state().question}
                        requestId={state().requestId}
                        error={state().error}
                        onClose={commands.closeBtw}
                    />
                )}
            </Show>
            <AgentSearchBar
                visible={search.visible}
                onSearch={search.performSearch}
                onNext={search.next}
                onPrev={search.prev}
                onClose={search.close}
                matchIndex={search.currentIndex}
                matchCount={search.matchCount}
            />

            {/* The scroll region: `.agent-document` inside AgentDocumentView is absolutely positioned to fill it. The
                working row is a sibling below the dock, not an overlay on this box
                (SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md). */}
            <div class="agent-document-scroll-region">
                <HistoryLoadingRow readiness={readiness} />
                <AgentDocumentView
                    documentNodes={shownDocument}
                    documentStateAtom={agentAtoms().documentStateAtom}
                    onOpenHistory={() => void openOrFocusHistoryTab({ currentBlockId: model.blockId, agentId })}
                    onAgentErrorLogin={() => {
                        // The button says "Login Again", so it forces a fresh OAuth for every provider, like the failure
                        // row's. Seeding from the personal credential here would reuse a possibly equally stale token
                        // (retro-agent-auth-relogin-noop-2026-07-01).
                        log("auth", "Login Again (inline error node) — forcing a fresh provider login");
                        void status.relogin();
                    }}
                    onLoadOlder={history.loadOlder}
                    loadingOlder={history.loadingOlder}
                    // The one paging gate: the list pages only while this is true.
                    hasOlderHistory={() => liveFeed.canPageOlder() && history.historyOffset() > 0}
                    followingRef={(f) => liveFeed.setFollowingBottom(f)}
                    scrollCommand={scroll.command}
                    scrollToBottomRef={(fn) => {
                        scrollToBottomFn = fn;
                    }}
                    highlightNodeId={search.highlightId}
                    registerHistoryReadyCallback={(fn) => {
                        historyReadyFn = fn;
                    }}
                    zoomFactor={zoomFactor}
                    blockId={model.blockId}
                    layoutView={layoutView}
                />
            </div>

            {/* Login, decisions, questions, queue, recovery banners, activity dock,
                working row and session notices (components/AgentBottomPanels.tsx). */}
            <AgentBottomPanels
                blockId={model.blockId}
                agentId={agentId}
                agentName={agentName()}
                authProviderId={provider()?.id ?? providerKey()}
                providerId={provider()?.id ?? ""}
                block={block}
                paneModel={paneModel}
                status={status}
                log={log}
                pendingDecisions={pendingDecisions}
                onDecide={handleDecide}
                pendingQuestions={pendingQuestions}
                onAnswer={handleAnswer}
                onCancel={handleCancel}
                pendingMessages={pendingMessages}
                failureRow={failureUI.row}
                backgroundTasksAtom={backgroundTasksAtom}
                workingRowVisible={workingRowVisible}
                workingRowLoading={workingRowLoading}
                onStop={commands.stopAgent}
                offline={offline}
            />

            {/* A container agent's container starting in the background (preparing.ts). */}
            <AgentPreparingRow nodes={displayDocument} enabled={meta.isContainer} />

            {/* The composer status strip: the activity ticker and the Log button (`detailsOpen` is reducer-owned). */}
            <AgentComposerStrip
                sessionTotals={paneModel.state.sessionTotals}
                loading={paneBusy()}
                logOpen={paneModel.state.detailsOpen}
                onToggleLog={withShellFocus(() => paneModel.dispatchPane({ type: "DetailsToggle" }, "user"))}
                contextTokens={contextReading()?.tokens ?? null}
                contextWindow={contextReading()?.window ?? undefined}
                contextNote={contextNote()}
                autoCompact={autoCompact()}
                lastReplyModel={paneModel.state.lastContextModel}
                authStatus={loginStatus()}
                authEmail={account.authEmail()}
                signInCheckedLater={block()?.meta?.["agentMode"] === "container"}
                canSwitchAccount={account.canSwitchAccount()}
                onSwitchAccount={account.onSwitchAccount}
                pendingAccountLabel={account.pendingAccountLabel()}
                blockId={model.blockId}
                blockAtom={block}
                providerId={provider()?.id ?? ""}
                agentMode={meta.agentMode()}
                compacting={paneModel.state.compacting}
                // "/compact" is deliberately not a registered SlashCommand (see AgentComposerStrip); it goes through
                // sendWithState for the state it captures before TurnStart.
                onCompact={() => void commands.sendWithState("/compact")}
                viewChip={viewChipLabel(viewControls.resolved())}
                viewOpen={viewControls.popoverOpen()}
                onOpenView={() => viewControls.setPopoverOpen(!viewControls.popoverOpen())}
            />
            {/* Hangs from the chip, or from the strip while there is none (opened from the menu or the palette). Keyed by
                which: the panel reads its anchor once, and a change of view can remove the chip under it. */}
            <Show when={viewControls.popoverOpen() ? (viewChipLabel(viewControls.resolved()) ? "chip" : "strip") : undefined} keyed>
                {(anchor) => (
                    <AgentViewPopover
                        controls={viewControls}
                        anchor={rootRef?.querySelector(anchor === "chip" ? "[data-agent-view-chip]" : ".agent-composer-strip") ?? undefined}
                    />
                )}
            </Show>

            <div class="agent-composer-region" data-toast-avoid>
                <Show when={commands.helpVisible()}>
                    <SlashHelpPanel
                        commands={commands.availableCommands()}
                        onInvoke={(cmd) => {
                            commands.closeHelp();
                            // Through sendWithState, not sendMessage: a bare sendMessage assumes no turn is running, so
                            // /login from this panel mid-turn would restart, and kill, that turn.
                            void commands.sendWithState(`/${cmd.name}`);
                        }}
                        onClose={commands.closeHelp}
                    />
                </Show>
                <Show when={commands.pickerSpec()}>
                    {(spec) => (
                        <SlashCommandPicker
                            spec={spec()}
                            onSelect={commands.resolvePicker}
                            onDismiss={commands.dismissPicker}
                        />
                    )}
                </Show>
                <AgentFooter
                    agentName={agentName()}
                    onSendMessage={offline.submit}
                    onTyping={() => scrollToBottomFn?.("typing")}
                    onStopAgent={commands.escapeOnEmptyComposer}
                    onRecallLatestQueued={commands.recallLatestHeld}
                    getCompletions={commands.completions}
                    viewModel={model}
                    isComposerEmptyRef={(fn) => (composerIsEmptyFn = fn)}
                />
            </div>
            </div>
            {/* The shell drawer: outside `.agent-view-zoomed`, a flex child of `.agent-view`. */}
            <AgentShellDrawer
                open={paneModel.state.detailsOpen}
                blockId={model.blockId}
                shellSubBlockId={meta.shellSubBlockId()}
                cwd={meta.cwd() || undefined}
                persistedHeight={meta.shellHeight()}
                onTermReady={handleShellTermReady}
                onTermDispose={handleShellTermDispose}
                onShellExited={handleShellExited}
            />
        </div>
        </AgentPaneProviders>
    );
};

AgentPresentationView.displayName = "AgentPresentationView";
