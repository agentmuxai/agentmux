// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AttachmentRef } from "@/types/rpc/AttachmentRef";
import {
    snapshot as layoutSnapshot,
    registerPane as registerLayoutPane,
    unregisterPane as unregisterLayoutPane,
    type LayoutView,
} from "@/app/store/agent-pane-layout-store";
import {
    registerPane as registerAgentPane,
    unregisterPane as unregisterAgentPane,
    type AgentPaneModel,
} from "@/app/store/agent-pane-registration";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import { isAuthFailure, isStopping, workingFromPhase } from "@/app/store/agent-pane-state/types";
import {
    registerActivity as registerAgentActivity,
    unregisterActivity as unregisterAgentActivity,
} from "@/app/store/agentActivity";
import { AgentPaneProviders } from "./agent-media";
import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { getRecentDispatches } from "@/app/store/command-source";
import { resolveContextMenuRegion } from "@/app/block/context-menu-region";
import { ContextMenuModel } from "@/app/store/contextmenu";
import {
    getApi,
    getBlockMetaKeyAtom,
    openOrFocusPaneByView,
    MOS,
} from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { readZoom } from "@/app/store/zoom-factor";
import { PaneLoadingCover } from "@/app/element/PaneLoadingCover";
import {
    accountLabel,
    loadAccounts,
} from "@/app/view/identity/identity-model";
import { handleAgentIdChange } from "@/app/view/term/termagent";
import { makeWindowFocusSignal } from "@/app/window/window-focus";
import { ErrorBoundary } from "@/element/errorboundary";
import { getTrail } from "@/log/render-trail";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import {
    createEffect,
    createMemo,
    createSignal,
    on,
    onCleanup,
    onMount,
    Show,
    type Accessor,
    type JSX,
} from "solid-js";
import { createPromotionClock } from "./activity/promotion-clock";
import { useAttachedTaskAxis } from "./activity/useAttachedTaskAxis";
import { AgentProgressBar } from "./components/AgentProgressBar";
import { useWorkingIndicator } from "./hooks/useWorkingIndicator";
import { quickForkAgent } from "./quick-fork";
import { isBangCommand } from "./bang-command";
import { askSideQuestion } from "./btw";
import type { AgentViewModel } from "./agent-model";
import "./agent-view.scss";
import { AgentBottomPanels } from "./components/AgentBottomPanels";
import { AgentComposerStrip } from "./components/AgentComposerStrip";
import { AgentShellDrawer } from "./components/AgentShellDrawer";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter } from "./components/AgentFooter";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { collapseDrawerOnShellExit } from "./shell-exit-collapse";
import { AgentStashDrawer } from "./components/AgentStashDrawer";
import { BtwOverlay } from "./components/BtwOverlay";
import { SlashCommandPicker } from "./components/SlashCommandPicker";
import { SlashHelpPanel } from "./components/SlashHelpPanel";
import { usePaneReveal } from "./hooks/usePaneReveal";
import { useLiveFeedRollOff } from "./hooks/useLiveFeedRollOff";
import { useShellLogBridge } from "./hooks/useShellLogBridge";
import { useFocusRepoll, useHeldMessageDelivery } from "./hooks/useTurnReconciliation";
import { useAmbientNarration } from "./hooks/useAmbientNarration";
import { useAgentActivitySummary } from "./hooks/useAgentActivitySummary";
import { useAgentCommands } from "./hooks/useAgentCommands";
import { useAgentControllerStatus } from "./hooks/useAgentControllerStatus";
import { useAgentDecisions } from "./hooks/useAgentDecisions";
import { useAgentDropAttach } from "./hooks/useAgentDropAttach";
import { useAgentFailure } from "./hooks/useAgentFailure";
import { useAccountBinding } from "./failure/useAccountBinding";
import { useAuthHealth, useSyntheticAuthRow } from "./failure/useAuthHealth";
import { requestAgentTakeover } from "./failure/takeover";
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
import { sendStartupSequence } from "./startup/sendStartupSequence";
import { createAgentAtoms } from "./state";
import type { DocumentNode } from "./types";
import { ShutdownOverlay } from "./shutdown/ShutdownOverlay";
import { useAgentStream } from "./useAgentStream";
import { setBlockMeta } from "@/app/store/block-meta";

// Launch flow lives in `flows/launch-flow.ts` — Step 2 of
// docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.

export const AgentPresentationView = ({
    model,
    agentId,
    agentDefinitions,
    progressBarMount,
}: {
    model: AgentViewModel;
    agentId: string;
    /** The wrapper's own reactive definition list, passed down instead of a
     *  second `useAgentDefinitions()` call here — each call issues its own
     *  ListAgentDefinitionsCommand RPC + `agents:changed` subscription, and
     *  this component is always co-mounted with the wrapper (reagent P2 on
     *  PR #2488). */
    agentDefinitions: () => AgentDefinition[];
    /** DOM node (owned by AgentPaneChrome, between the tab strip and the
     *  content) the marching-ants progress bar portals into — bridged
     *  through this AgentViewModel instance via
     *  progressBarMount/setProgressBarMount (see those fields' own doc
     *  comments in agent-model.ts) since chrome and content are separate
     *  component trees now. undefined/null before chrome has mounted and
     *  called setProgressBarMount at least once. */
    progressBarMount: () => HTMLDivElement | undefined;
}): JSX.Element => {
    const block = model.blockAtom;
    // True while the user can't see this tab: a hidden, kept-alive
    // pane-tab-strip member (SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md), or
    // its window tab isn't displayed (`usePaneTabVisibility`, Pane Tab
    // contract Phase 3). Render work pauses on it, and it's threaded into
    // AgentQuestionPanel's auto-timeout and useAgentFailure's auto-retry below
    // so neither fires invisibly while backgrounded — which, before Phase 3b,
    // covered only the pane-stack case.
    const visibility = usePaneTabVisibility(model.blockId);
    const hidden = (): boolean => visibility() !== "active";
    const providerKey = (): string => block()?.meta?.["agentProvider"] ?? agentId;
    const provider = () => getProvider(providerKey());
    const outputFormat = (): string => block()?.meta?.["agentOutputFormat"] ?? "claude-stream-json";
    // Human-readable display name for the composer placeholder. Matches
    // the same fallback chain used by the onMount log() on line 380 —
    // single source of truth for "what to call this agent in the UI."
    // Reactive: the textarea placeholder updates if the user renames
    // the agent without remounting the pane.
    const agentName = (): string => block()?.meta?.["agentName"] ?? agentId;

    // Reactive agent-definition list (from the wrapper) — used to resolve
    // the current AgentDefinition object for identity/memory modal requests.
    const currentAgent = createMemo(() => agentDefinitions().find((a) => a.id === agentId));

    // Fork tab strip + in-pane "+" tab logic lives in AgentPaneChrome
    // (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.3 follow-up,
    // 2026-08-09, moved out of this component by
    // SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md) so the strip
    // stays visible even when the pane's active member is a blank/picker
    // tab with no agentId yet — see that component's comment for the full
    // rationale.

    // Wire the Stash drawer's open/close/read callbacks into the model so
    // the single title-bar "Stash" (backpack) icon can drive it without the
    // model holding a SolidJS context.
    //
    // These used to open a MODAL (`modalLayer.open({kind:"agent-stash"})`);
    // the drawer replaced it in
    // SPEC_AGENT_STASH_PANE_MIGRATION_2026_09_22.md §3.3, which is why all
    // three collapse to one-liners now — no layer to open, no backdrop to
    // dismiss, just a reducer field. The callback NAMES keep their
    // `...StashModal` spelling only to avoid churning agent-model.ts's
    // already-shipped toggle wiring (PR #3516) in the same change; they are
    // drawer callbacks now.
    onMount(() => {
        model._openAgentStashModal = () => paneModel.dispatchPane({ type: "StashExpand" }, "user");
        model._closeAgentStashModal = () => paneModel.dispatchPane({ type: "StashCollapse" }, "user");
        // Read fresh at call time. `paneModel.state` exposes each field as
        // its own signal (agent-pane-state-store.ts's createFieldSignals),
        // and agent-model.ts's endIconButtons() reads this inside
        // blockframe.tsx's createMemo — so the icon's highlighted state
        // tracks `stashOpen` reactively, including closes that don't come
        // from the button itself.
        model._isAgentStashOpen = () => paneModel.state.stashOpen;
    });
    onCleanup(() => {
        model._openAgentStashModal = null;
        model._closeAgentStashModal = null;
        model._isAgentStashOpen = null;
    });

    const agentAtoms = createMemo(() => createAgentAtoms());

    // Register this pane with BOTH the document store and the pane-state
    // store SYNCHRONOUSLY in one atomic call, during component-body
    // execution — before any hook's onMount can dispatch. The stores throw
    // on dispatch to an unregistered slot to prevent silent reducer-command
    // drops, so both slots must exist before useAgentStream /
    // useHistoryPagination call their first dispatch from `onMount` handlers.
    //
    // Registration is unified so a dispatcher can never observe the pane
    // registered in one store but not the other — the half-registered window
    // was the structural root of a cascade-mid-dispatch failure mode. See
    // agent-pane-registration.ts.
    //
    // `turnPhase` is the single-source-of-truth working/stopping signal
    // since PR G — the legacy `turnActive` / `stopping` /
    // `streaming.active` fields and their projection setters were
    // removed. The view binds the working animation to
    // `workingFromPhase(turnPhase)` and the "Stopping…" label to
    // `turnPhase.kind === "Interrupting"`.
    // Capture the model returned by registerPane — passed into hooks/views
    // so their dispatch callsites are default-safe against post-unmount races. The disposed flag on
    // the model flips synchronously inside `unregisterAgentPane` BEFORE
    // either store unregisters, so any deferred dispatcher landing in
    // the cleanup window observes disposed=true and silently drops.
    // See agent-pane-model.ts for the rationale.
    // Last-known shell sub-block id, tracked outside Solid's reactive graph.
    // On the "silent dispose by an outer owner" path documented in the
    // onCleanup below, `block()` (model.blockAtom) is ALREADY null by the
    // time cleanup runs — the outer <Show> in block.tsx unmounts this pane
    // in response to the same blockData→null transition, and Solid tears
    // down children before/without re-reading their props at a stale value.
    // Reading `block()?.meta?.["term:shellsubblockid"]` directly inside
    // onCleanup would silently resolve to undefined on that path and the
    // sub-block's PTY would leak. This effect mirrors the id into a plain
    // variable on every change (skipping null so it keeps the last-known
    // value instead of clearing it), so cleanup always has it regardless of
    // what `block()` reads at that instant.
    let shellSubBlockIdRef: string | undefined;
    createEffect(() => {
        const id = block()?.meta?.["term:shellsubblockid"] as string | undefined;
        if (id) shellSubBlockIdRef = id;
    });

    let paneModel: AgentPaneModel;
    {
        // A6 of issue #1549: the stores own their reactive read side now
        // (`paneModel.state` / `paneModel.document`). Nothing is wired from
        // the view into them, so a new reducer field is reactive here
        // without touching this file.
        paneModel = registerAgentPane(model.blockId, { agentId });
        registerAgentActivity(model.blockId, () => paneModel.state.turnPhase);

        // Register with the reactive handler so the Swarm view sees this pane.
        // Uses the same handleAgentIdChange path as the PTY/OSC flow — handles
        // de-dup and block-to-agent bookkeeping identically.
        handleAgentIdChange(model.blockId, agentName());

        onCleanup(() => {
            // [DIAGNOSTIC] Capture WHY this pane disposed. The "pane went blank
            // mid-stream" failure is a SILENT dispose by an OUTER owner (e.g.
            // block.tsx <Show> on blockData→null from a backend block-delete): it
            // never throws, so BlockErrorBoundary never fires and no render_trail
            // is dumped — we only ever saw the aftermath (CASCADE_DETECTED). This
            // runs on EVERY dispose path; the stack distinguishes which owner tore
            // us down. If we're disposed while the turn is still WORKING, that's
            // unexpected — also dump the render-trail + recent reducer-dispatch
            // ring so the next reproduction yields a root cause.
            // See PLAN_PANE_CRASH_DIAGNOSTICS_2026-06-05.md.
            const phase = paneModel.state.turnPhase;
            const midTurn = workingFromPhase(phase);
            if (midTurn) {
                console.warn(
                    `[agent-view] DISPOSE UNEXPECTED(mid-turn) blockId=${model.blockId.slice(0, 7)} turnPhase=${JSON.stringify(phase)} stack=${new Error().stack}`
                );
                try {
                    console.warn(`[agent-view] DISPOSE mid-turn render_trail=${JSON.stringify(getTrail())}`);
                    console.warn(
                        `[agent-view] DISPOSE mid-turn recent_dispatches=${JSON.stringify(getRecentDispatches(40))}`
                    );
                } catch {
                    /* best-effort diagnostic */
                }
            }
            unregisterAgentPane(model.blockId);
            unregisterAgentActivity(model.blockId);
            handleAgentIdChange(model.blockId, undefined);

            // Phase 0 spike (SPEC_AGENT_SHELL_XTERM_TERMINAL_2026_07_03.md §7):
            // the PTY is kept alive across drawer open/close (see
            // AgentShellSubblock) but MUST die with the pane — a lingering
            // shell is exactly the leak class issue #1936 tracks. Reads the
            // plain-variable mirror (shellSubBlockIdRef above), NOT block()
            // directly — block() can already be null here on the silent
            // outer-owner dispose path, which would otherwise drop this
            // delete silently and leak the PTY.
            if (shellSubBlockIdRef) {
                void RpcApi.DeleteSubBlockCommand(TabRpcClient, { blockid: shellSubBlockIdRef });
            }
        });

        // Mirror context token count to block meta so the Swarm view can read
        // it without needing access to per-pane in-memory signals. Fires at
        // most once per turn (TokensIn at message_start).
        createEffect(() => {
            const tokens = (paneModel.state.lastContextTokens ?? null);
            void setBlockMeta(model.blockId, { "term:ctx-tokens": tokens ?? null } as any);
        });
    }

    // ── Layout slice lifecycle. The slice is FED from
    //    AgentDocumentVirtualList (Phase 3): it owns `partition()`, so it can
    //    scope `NodesChanged` to the virtualized region (the slice must model
    //    ONLY the prefix-summed rows, not the streaming buffer). Here we only
    //    register/unregister the per-pane slot.
    // Phase 3 Step 0: own the derived layout-view signal the list renders
    // from. The store recomputes computeLayoutView and calls this setter on
    // every layout-input change (deduped by viewsEqual). `zoom` stays a no-op
    // — INV-2: the single CSS `zoom` on `.agent-view` does the visual scaling.
    const [layoutView, setLayoutView] = createSignal<LayoutView | null>(null);
    registerLayoutPane(model.blockId, { layout: setLayoutView, zoom: () => {} });
    onCleanup(() => unregisterLayoutPane(model.blockId));

    // DEV-only: CDP validation hook — lets engineers run
    // `__agentLayout()` in the console to snapshot the slice state.
    if (import.meta.env.DEV) {
        (window as unknown as { __agentLayout?: () => unknown }).__agentLayout = () => layoutSnapshot(model.blockId);
    }

    // The shell drawer's log bridge: which log lines reach the shell terminal,
    // and the backlog replayed when it mounts (hooks/useShellLogBridge.ts).
    const {
        log,
        onTermReady: handleShellTermReady,
        onTermDispose: handleShellTermDispose,
        clearTermWrite,
    } = useShellLogBridge();

    /**
     * The drawer's shell process exited cleanly — the human typed `exit`.
     * Collapse the drawer around it
     * (SPEC_AGENT_PANE_SHELL_EXIT_COLLAPSES_DRAWER_2026_09_15.md §3.2).
     *
     * The body lives in `shell-exit-collapse.ts` so it can be tested — this
     * file has no render harness, and the three effects it performs are each
     * separately load-bearing (see that module's doc comment). Here we only
     * read-and-clear the local ref, so the pane-level `onCleanup` below can't
     * later try to delete a sub-block this already removed.
     */
    const handleShellExited = () => {
        const exitedId = shellSubBlockIdRef;
        shellSubBlockIdRef = undefined;
        void collapseDrawerOnShellExit({
            parentBlockId: model.blockId,
            exitedSubBlockId: exitedId,
            clearTermWrite,
            collapseDrawer: () => paneModel.dispatchPane({ type: "DetailsCollapse" }, "system"),
            setMeta: (args) =>
                RpcApi.SetMetaCommand(TabRpcClient, { oref: args.oref, meta: args.meta as any }),
            deleteSubBlock: (args) => RpcApi.DeleteSubBlockCommand(TabRpcClient, args),
            makeORef: MOS.makeORef,
        }).catch((err) => {
            // Best-effort teardown: the drawer has already collapsed (that
            // part is synchronous, above), so a failed RPC costs a leaked
            // sub-block, not a stuck UI.
            console.warn("[agent-view] shell-exit teardown failed:", err);
        });
    };

    // Startup sequence callback ref — assigned after commands + handleSendMessage
    // are defined (below), so the onReady callback can reference them.
    // onReady fires synchronously after startLaunchFlow succeeds, which is
    // always after this component body has fully run (SolidJS onMount timing).
    let onReadyFn: (() => void) | null = null;

    // History pagination: dispatches HistoryLoaded into the agent-document-store
    // for the trailing 200 lines on mount + each user-triggered loadOlder.
    //
    // Pass the agent definition id so the snapshot fast-path reads from the
    // agent-anchored zone (`agent:<defId>:current`) rather than the
    // per-block zone. `agentId` here is the AgentDefinition slug/UUID —
    // a non-empty string is guaranteed at this point by AgentBlockContent's
    // own `Show when={agentId()}` gate around this component.
    // Bridged callback: AgentDocumentView registers viewState.markHistoryReady
    // here on mount (before any async history work starts), so
    // useHistoryPagination can signal "history done" into the viewState
    // that lives inside AgentDocumentView. This drives the enter-animation
    // gate on the streaming buffer.
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
        // Jekt direction detection during replay: FROM == this agent →
        // outgoing bubble (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2).
        agentName,
        definitionId: agentId,
        // Only with a turn cap set (by default the feed is bounded by size):
        // K + the turn in flight; Claude only. Read after this body runs.
        restoreTurns: () =>
            liveFeed.liveFeedOn() && outputFormat() === "claude-stream-json" && Number.isFinite(liveFeed.liveFeedTurns) ? liveFeed.liveFeedTurns + 1 : undefined,
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

    // Will the next spawn continue the conversation this pane is displaying, or
    // start a new one? Asked once on mount, before the user can type — the
    // whole point is to beat the first send, since every other continuity
    // signal is retrospective. See `hooks/useResumePreflight.ts`.
    const resumePreflight = useResumePreflight(model.blockId);

    // The live feed: the turn in flight plus the last K finished turns; older
    // ones roll off into History (hooks/useLiveFeedRollOff.ts). Created here,
    // after the history hook; the callbacks above that call into it
    // (onHistoryReady, restoreTurns) run only once this body has finished.
    const liveFeed = useLiveFeedRollOff({
        blockId: model.blockId,
        paneModel,
        outputFormat,
        block,
        agentAtoms,
        hidden,
        history,
    });
    const { liveFeedOn, gapsBefore, earlierHistoryAvailable, earlierTurnsKnown } = liveFeed;

    // Read-side view of the document with the "Open Agent History" link row
    // injected as a normal, scrolling document node (§3.2 of
    // SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md) —
    // replaces the old pinned-above-the-scroll-region PaneRow. The real
    // read-only view: AgentDocumentView / createAgentViewState only ever
    // read it. Writes go through `paneModel.dispatchDoc`.
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

    // Auth + launch flow state and the onCleanup that kills the CLI
    // if the pane closes mid-login.
    // `getDocument` is read-only; for writes we MUST dispatch through
    // agent-document-store so slot.state stays in sync.
    const getDocument = paneModel.document;

    // Agent-pane state-persistence (RFC #857 + spec
    // SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md). See
    // hooks/useSnapshotPersistence.ts.
    useSnapshotPersistence({
        blockId: model.blockId,
        definitionId: agentId,
        getAtoms: agentAtoms,
        getDocument,
        getDetailsOpen: () => paneModel.state.detailsOpen,
        snapshotIsForeignBlock: () => history.snapshotIsForeignBlock(),
        log,
    });

    // Permission decision queue + decide handler. See hooks/useAgentDecisions.ts.
    const { pendingDecisions, handleDecide } = useAgentDecisions({
        blockId: model.blockId,
        getDocument,
        log,
    });

    // AskUserQuestion queue, waiting-ambient tone, and answer handler.
    // See hooks/useAgentQuestions.ts. `sendMessage` is passed as a thunk so
    // the non-persistent follow-up fallback (invoked only inside the async
    // catch) can delegate to the handleSendMessage defined below.
    const { pendingQuestions, handleAnswer, handleCancel } = useAgentQuestions({
        blockId: model.blockId,
        getDocument,
        sendMessage: (message: string) => handleSendMessage(message),
        log,
    });

    // Root element ref — declared before useAgentControllerStatus so its
    // getInitialTermSize closure can read the laid-out pane width when the
    // launch flow's Phase-3 resync runs (the ref is assigned during render,
    // before onMount fires). Also consumed by dropAttach + usePtyWidth below.
    let rootRef: HTMLDivElement | undefined;

    // Whether the backend's turn is confirmed active / idle, and the
    // turn-end edge (hooks/turn-confirmation.ts). onTurnEnded reads `commands`,
    // defined further down this body; it only ever runs from async event
    // callbacks registered after the full setup has run (Codex P1 on #2338).
    const turnConfirmation = createTurnConfirmation({
        reconcile: (active) =>
            paneModel.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active }, "system"),
        onTurnEnded: () => void commands.flushPendingControllerRefresh(),
    });
    const { turnJustEndedAtom, reconcileTurnActive, trackTurnJustEnded } = turnConfirmation;

    // Turn-end ghost-tool scrub (user report 2026-08-10: a ~1s `git status`
    // call stuck as a "running \u00b7 45m" dock row for the rest of the session).
    // A foreground tool call cannot outlive its turn \u2014 it blocks it \u2014 and a
    // backgrounded harness call resolves its ToolNode immediately, so a tool
    // node still `running` shortly AFTER TurnEnd is provably an orphan
    // (rejected call / dropped tool_result). scrubOrphanedInProgress
    // otherwise only runs at session boundaries (SessionEnd/HistoryLoaded),
    // which is why the ghost survived all session. The 2s delay absorbs any
    // tail flush still in flight; the working guard skips the pass when a
    // new turn already started (its running tools are legit). tools-only
    // scope: thinking markdown, shells (turn-independent), and questions
    // keep their session-bounded lifecycles.
    createEffect(
        on(turnJustEndedAtom, (n) => {
            if (n === 0) return;
            liveFeed.scheduleRollOff();
            const timer = setTimeout(() => {
                if (workingFromPhase(paneModel.state.turnPhase)) return;
                paneModel.dispatchDoc({
                    type: "ScrubOrphanedInProgress",
                    at: Date.now(),
                    scope: "tools-only",
                });
            }, 2_000);
            onCleanup(() => clearTimeout(timer));
        })
    );

    // Posts a permanent, visible line into the pane's own conversation \u2014
    // distinct from `log()`, which routes to the hidden activity-log/shell-
    // terminal channel (see docs/specs/SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md
    // \u00a71). "success"/"warning" get a symbol prefix; "info" doesn't (used for
    // neutral narration like "Signing in..." or "Ready...").
    const postSystemNotification = (text: string, style: "info" | "warning" | "success" = "info"): void => {
        const prefix = style === "success" ? "\u2713 " : style === "warning" ? "\u26a0 " : "";
        const node: import("./types").MarkdownNode = {
            type: "markdown",
            id: `system_notification_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`,
            content: `${prefix}${text}`,
        } as import("./types").MarkdownNode;
        paneModel.dispatchDoc({
            type: "StreamFlush",
            newNodes: [node],
            updatedNodes: [],
        });
    };

    const status = useAgentControllerStatus({
        blockId: model.blockId,
        provider,
        log,
        onLoginSuccess: (email) => {
            const display = email ? `Logged in as **${email}**` : "Login successful";
            postSystemNotification(display, "success");
        },
        onNotify: (text, style) => postSystemNotification(text, style),
        onReady: () => onReadyFn?.(),
        // A successful recovery (relogin / terminal login) refreshed
        // the credential — retry the failed turn so the agent recovers in one
        // click. Lazy arrow: retryLastTurn is defined below but only invoked at
        // runtime (post-click), by which point it's initialized.
        onRecovered: () => {
            // If a DIFFERENT, overlapping recovery flow is still running,
            // leave the failure banner and loginWaiting() both untouched
            // instead of clearing-and-retrying now. THIS flow's own
            // credential is confirmed good, but relogin()/
            // loginViaTerminal() never check whether a turn is active
            // before calling forceControllerRefresh (only /login's
            // slash-command path does) — clearing+retrying here used to let
            // this resend start a new turn that the sibling's later restart
            // would then kill (Codex P1, fourteenth re-review), and even
            // after gating the SEND on loginWaiting() (so it correctly got
            // rejected instead), clearing the banner unconditionally still
            // discarded the user's only path back if that sibling
            // ultimately FAILS: a failed flow only decrements the counter
            // and never calls onRecovered, so nothing retries automatically,
            // and the banner this comment used to clear first was the
            // user's manual "Retry"/"Login Again" affordance too. Codex P2
            // on PR #2338 (sixteenth re-review). Bailing out here instead
            // leaves that banner up — the user can retry manually once the
            // sibling settles, and if the sibling instead SUCCEEDS, ITS OWN
            // onRecovered fires this same check with loginWaiting() now
            // false, and completes the clear+retry then.
            if (status.loginWaiting()) return;
            // Recovery succeeded and no sibling is in flight — explicitly
            // resolve the failure this banner was showing rather than
            // waiting for retryLastTurn's own TurnStart to clear it as a
            // side effect. handleSendMessage captures the live
            // "auth"-classified failure state BEFORE dispatching TurnStart
            // (so a user's own fresh keystroke send gets fast-failed
            // against a still-showing auth failure); without clearing here
            // first, that same capture would see the stale failure on THIS
            // auto-retry too and wrongly reject the very resend recovery
            // just enabled. Codex P1 on PR #2338.
            // Unconditional again, and deliberately so.
            //
            // Two earlier attempts on this PR tried to make the retry-vs-startup
            // decision HERE, from pane state, because loginViaTerminal (unlike
            // relogin) called this without saying which it wanted. Retrying
            // unconditionally resent a stale message on a never-launched agent
            // (codex); returning early instead dropped the startup sequence
            // (reagent); and reading the failure to decide raced the login's own
            // setCanRetry(false), which synchronously flushes the effect that
            // clears that very failure (reagent + manoz, independently).
            //
            // The fix was to remove the inference, not to time it better:
            // loginViaTerminal now takes an explicit retryAfterLogin like
            // relogin always did, decided at click time from the row's own
            // turnAttempted, and calls onReady() instead of this on its
            // no-retry path. So every caller that reaches here wants a retry.
            paneModel.dispatchPane({ type: "FailureCleared" }, "system");
            retryLastTurn();
        },
        getInitialTermSize: () => computeTermSizeFromEl(rootRef),
        // Mount-time TurnPhase reconciliation — see
        // docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
        // Finding 1. Dispatched via paneModel (soft; never throws) because
        // this can resolve before registerPane() has run for a pane still
        // mid-mount.
        onControllerStatus: (rts) => {
            reconcileTurnActive(!!rts.turn_active);
        },
    });

    // Hold the reveal until the launch flow is past the phases that can pop
    // the auth panel in; then fade (hooks/usePaneReveal.ts).
    reveal.connectLaunchStatus(status);

    // status.isLoading() is `flowRunning() || !agentReady()` — it never
    // becomes true during relogin()/loginViaTerminal(),
    // since the agent is already ready by the time those recovery flows
    // run. Without launchPhase() in this gate too, the working row (and
    // its phase label + Cancel button), the top progress bar, and the
    // composer status strip all stay invisible for the entire up-to-5-
    // minute recovery poll — exactly the flows this launchPhase work was
    // meant to make visible. reagent P1 on PR #2300.
    const showingLaunchActivity = () => status.isLoading() || status.launchPhase() != null;

    // Composer strip's logged-in/out tag. `status.authStatus()` is the
    // durable signal (set at mount and on every successful login/relogin),
    // but a mid-turn 401 (credential went stale while the agent was already
    // marked authenticated) surfaces first as an "auth"-classified failure
    // row, not a fresh authStatus transition — so a live auth failure
    // overrides the tag to "unauthenticated" the instant it appears, instead
    // of waiting for the user to click "Login Again" first.
    const loginStatus = createMemo((): "authenticated" | "unauthenticated" | "unknown" => {
        if (isAuthFailure(paneModel.state.failure)) return "unauthenticated";
        return status.authStatus();
    });

    onMount(() => {
        const name = block()?.meta?.["agentName"] ?? agentId;
        const provName = provider()?.displayName ?? providerKey();
        const cwd = block()?.meta?.["cmd:cwd"] ?? "";
        log("agent", `${name} selected (provider: ${provName})`);
        if (cwd) log("env", `working directory: ${cwd}`);
        status.startLaunchFlow();
    });

    // Log controllerstatus events as they stream in, and reconcile the pane's
    // TurnPhase from the backend's live `turn_active` in both directions — the
    // mount-time GetControllerStatus (onControllerStatus above) is one-shot, so
    // without this a turn that ends while the pane is mounted but whose
    // session_end the frontend missed would leave the phase stuck at Streaming
    // (Agent1 stuck-"Working" / Agent2 stuck-"Queued"). Same dispatch as the
    // mount reconcile, just fed by every live controllerstatus event.
    useControllerStatusEvents({
        blockId: model.blockId,
        log,
        onTurnActive: (active) => {
            reconcileTurnActive(active);
            trackTurnJustEnded(active);
        },
        onActiveTurnConfirmed: () => {
            // A controllerstatus event with an ACTIVE turn is independent
            // proof the CLI is alive and running turns — clear any stale
            // "Retry Login" / auth notice left over from the mount-time
            // gated launch flow's auth_failed classification. Otherwise
            // the button can outlive the failure it was reporting: an
            // agent recovers and starts answering messages through this
            // same event stream, but nothing ever told useAgentControllerStatus
            // its earlier canRetry=true was stale. Reported live 2026-07-18.
            // Gated on an ACTIVE turn specifically (not any controllerstatus
            // event) — codex P1 on PR #2338 (eighth re-review): an idle
            // heartbeat from a controller left alive from before a
            // just-FAILED recovery attempt carries no proof the credential
            // is valid, and would otherwise silently clear that recovery's
            // own canRetry=true, letting the next message bypass the
            // fast-fail guard and reach the still-known-bad process.
            //
            // Also clears a stale live "auth"-classified state.failure —
            // unlike the OTHER two places in this PR that declare a
            // controller healthy (login.ts's finalizeLoginSuccess,
            // useAgentCommands.ts's flushPendingControllerRefresh success
            // path), this call site only ever cleared canRetry via
            // notifyControllerHealthy, never the separate state.failure
            // checkAuthGuard's liveAuthFailure check reads
            // (paneSnapshot(...).failure?.data.code === "auth"). Without
            // this, a stale failure row survives even this independent,
            // stronger proof of health (a live controllerstatus event
            // showing a turn genuinely streaming), permanently
            // fast-failing every subsequent send. reagentx P1 on PR #2338
            // (thirty-second re-review).
            //
            // Gated on the failure actually being "auth" — FailureCleared
            // has no payload and unconditionally clears state.failure
            // REGARDLESS of code (reducer.ts's FailureCleared case), so
            // dispatching it unconditionally here would ALSO silently wipe
            // an unrelated concurrent failure (rate_limited, overloaded,
            // context_exceeded, etc.) that happens to be showing the moment
            // a turn-active event arrives, even though that unrelated
            // problem was never actually resolved. reagentx P1 on PR #2338
            // (thirty-fifth re-review).
            //
            // Extracted into `declareAuthHealthy` (defined below, in scope
            // via closure) so the SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md
            // §2 bind-event listener can reuse the identical logic instead of
            // duplicating this exact gating a second time.
            declareAuthHealthy();
        },
    });

    // Re-poll turn state on refocus, to recover a missed turn-end push
    // (hooks/useTurnReconciliation.ts).
    useFocusRepoll({
        blockId: model.blockId,
        windowFocused: makeWindowFocusSignal(),
        trackTurnJustEnded,
        // `commands` is declared further down; only read once a poll resolves.
        flushPendingControllerRefresh: () => commands.flushPendingControllerRefresh(),
    });

    // Subscribe to Claude Code OSC window-title extractions and write them
    // to term:osc_title block metadata (free fallback signal — see
    // readActivitySummary()'s precedence in agent-model.ts).
    useBlockActivity({ blockId: model.blockId });

    // Haiku-powered session-goal title: maintains a stable PR-title-style
    // phrase in term:ambient_summary, re-evaluated (and usually reaffirmed
    // unchanged) each time the user submits a new message — NOT on turn
    // completion; see useAgentActivitySummary.ts's module doc comment.
    // Preferred over the OSC title above when both are present. Routed
    // through the backend's Ambient Model Call gateway — see
    // docs/specs/SPEC_AMBIENT_MODEL_CALLS_FRAMEWORK_2026_07_03.md.
    useAgentActivitySummary({
        blockId: model.blockId,
        turnPhase: (() => paneModel.state.turnPhase),
        getRootWidth: () => rootRef?.offsetWidth,
    });

    // Ghost-text next-prompt suggestion (composer). Populated by AgentFooter
    // via its isComposerEmptyRef prop below. Defaults to "empty" if the
    // footer hasn't mounted yet — matches the common case (no suggestion
    // exists yet either, since one only appears after a completed turn).
    let composerIsEmptyFn: (() => boolean) | null = null;
    useNextPromptSuggestion({
        blockId: model.blockId,
        turnPhase: (() => paneModel.state.turnPhase),
        turnJustEndedAtom,
        isComposerEmpty: () => composerIsEmptyFn?.() ?? true,
    });

    // Subscribe to subprocess output and parse into DocumentNodes.
    // Mutations dispatch through agent-document-store; the reducer there
    // owns dedup against in-flight history loads and the truncate-suppress
    // invariant that prevents the mid-session wipe bug.
    const pendingMessages = () => paneModel.state.pending;
    // Short lines from AgentMux about its own actions (first consumer: a tool
    // call the harness detached), inserted in-flow as `ambient_narration` nodes.
    // Dispatched straight to the document store like `postSystemNotification`,
    // NOT through the stream-flush queue: that queue's flush also reports the
    // batch to the pane as turn activity, and AgentMux talking to itself is not
    // evidence the model is doing anything. Best-effort — nothing here can fail
    // in a way that affects the pane, and an absent narration is simply silence.
    useAmbientNarration(model.blockId, (node) => {
        paneModel.dispatchDoc({ type: "StreamFlush", newNodes: [node], updatedNodes: [] });
    });
    // Forwarded to ActivityDock so it can render registry-known background
    // tasks the transcript itself has no record of (Tier 1 of
    // docs/reports/REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md).
    const backgroundTasksAtom = useAgentStream({
        blockId: model.blockId,
        transcriptSettle,
        // Pass the per-pane model so the hook's dispatch sites are
        // default-safe against post-unmount races — the disposed-flag
        // check is centralized in the model rather than per call site.
        model: paneModel,
        outputFormat: outputFormat(),
        documentNodes: paneModel.document,
        // turnPhase is the SoT for "is a stop in flight". useAgentStream
        // needs it to detect user-initiated stops and append the
        // "⏹ Interrupted by user" row when session_end lands.
        turnPhase: () => paneModel.state.turnPhase,
        // See useAgentStream.ts's UseAgentStreamOpts doc comment: watched by
        // useCompactionStream to push the "Compacting conversation…"
        // transcript node whenever `compacting` transitions to set,
        // regardless of which dispatch caused it (SPEC_COMPACTION_STARTED_
        // RECONCILIATION_RACE_2026_09_02.md).
        compacting: () => paneModel.state.compacting,
        pendingMessages,
        enabled: true,
        // Provider id (lowercase catalog key) attributes completed-turn
        // tokens to the correct row in the status-bar token-usage store.
        provider: providerKey(),
        // Jekt direction detection on the live stream: FROM == this agent
        // → outgoing bubble (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2).
        agentName: agentName(),
        // Re-engage message-list auto-scroll for a turn that starts from
        // the queue-drain path — see usePendingMessageAcceptance's
        // `onTurnStartFromQueue` doc comment. `scrollToBottomFn` is declared
        // below (assigned once AgentDocumentView mounts); referencing it in
        // this closure is safe regardless of declaration order since the
        // closure only runs later, on a live `agent-message-accepted` event.
        onTurnStartFromQueue: () => scrollToBottomFn?.("queued-turn"),
    });

    // Mutable ref to the scrollToBottom function exposed by
    // AgentDocumentView. Called by AgentFooter's onTyping when the user
    // starts composing AND by useAgentCommands.onSent after the user's
    // message has been appended to the document (SPEC_AGENT_PANE_FOLLOWUPS
    // item #1). Declared here so both useAgentCommands and the JSX below
    // can close over the same reference; assigned once AgentDocumentView
    // mounts via scrollToBottomRef.
    let scrollToBottomFn: ((reason?: string) => void) | null = null;

    // The wall-clock instant a running Bash call is promoted to the dock, for
    // everything that depends on promotion (activity/promotion-clock.ts).
    const promotionTick = createPromotionClock(paneModel.document);

    // The busy predicate and its renderings (hooks/useWorkingIndicator.ts).
    const { paneBusy, workingRowVisible, hasPromotedTool } = useWorkingIndicator({
        paneModel,
        showingLaunchActivity,
        promotionTick,
    });
    const workingRowLoading = paneBusy;

    // Attached-task axis dispatch (activity/useAttachedTaskAxis.ts).
    useAttachedTaskAxis({ blockId: model.blockId, paneModel, promotionTick });

    // User-message send + /login /clear slash intercepts + back-to-picker.
    // See hooks/useAgentCommands.ts.
    const commands = useAgentCommands({
        // Threaded in so the hook's §2.3a eager-flush evaluates the same
        // whole-predicate `paneBusyForInput` this view renders (ReAgent P1 on
        // PR #3340), rather than a subset that could disagree with it.
        showingLaunchActivity,
        blockId: model.blockId,
        // Per-pane model keeps dispatch sites default-safe; see useAgentStream above.
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
        // The last CONFIRMED backend turn_active reading (see
        // UseAgentCommandsOptions.isBackendTurnActive's doc comment) —
        // `wasTurnActive` is the same state trackTurnJustEnded's edge
        // detector uses, tracked from live controllerstatus events only
        // (never the mount-time GetControllerStatus one-shot — see
        // trackTurnJustEnded's own doc comment for why). Codex P1 on
        // PR #2338 (nineteenth re-review).
        isBackendTurnActive: turnConfirmation.isBackendTurnActive,
        // Deliberately NOT `!isBackendTurnActive()` (which would treat
        // `undefined` — never confirmed either way, e.g. a pane that
        // mounts mid-turn before its first live controllerstatus event
        // arrives — the SAME as confirmed idle). flushPendingControllerRefresh
        // force-restarts the controller when it proceeds, so it must
        // require POSITIVE confirmation of idle before doing something
        // destructive — "we don't know yet" must lean toward "don't
        // flush," not "safe to flush." A pane that mounts onto an
        // already-active turn and never receives a live event before a
        // premature per-round session_end demotes turnPhase would
        // otherwise have a deferred /login refresh flushed prematurely,
        // killing that still-active (just never locally confirmed) turn.
        // reagent P1 on PR #2338 (twenty-first re-review).
        isBackendTurnConfirmedIdle: turnConfirmation.isBackendTurnConfirmedIdle,
        backToPicker: () => model.backToPicker(),
        // /fork — same fork-to-sibling-tab action as the pane's right-click
        // "Quick Fork" context-menu item (agent-model.ts's
        // getBodyContextMenuItems, which passes `this` — this same `model`
        // instance already satisfies QuickForkModel).
        quickFork: () => quickForkAgent(model),
        // /btw — fires the side-question backend request (AskSideQuestionCommand);
        // useAgentCommands itself owns opening/updating the overlay signal
        // around this call.
        askSideQuestion: (question: string) => askSideQuestion(model.blockId, question, paneModel.document()),
        // Scroll the user's own message into view after Enter. The hook
        // defers this to the next animation frame so the mounted node is
        // included in scrollHeight. See SPEC_AGENT_PANE_FOLLOWUPS item #1.
        onSent: () => scrollToBottomFn?.("sent"),
        pendingMessages,
    });

    // Mark turn as active when the user sends a message — TurnStart
    // also clears stale sessionStats from the prior turn.
    const handleSendMessage = (message: string, attachments: AttachmentRef[] = []): Promise<void> => {
        // Bang commands (`!cmd`) output writes into the shell terminal (see
        // `log`/`handleShellTermReady` above). Auto-open the details drawer so
        // the shell — and thus the output — is immediately visible; without
        // this the user sees no feedback if the drawer is closed.
        if (isBangCommand(message)) {
            paneModel.dispatchPane({ type: "DetailsExpand" }, "user");
        }
        // Capture working state BEFORE TurnStart so PendingMessageQueued can
        // mark whether this message is queued behind a running turn (true) or
        // is the message that initiated the turn (false). The panel only shows
        // messages with enqueuedWhileBusy:true, preventing the idle-send race
        // where the message flashed in the amber zone between Streaming
        // promotion and agent-message-accepted. See ANALYSIS_IDLE_SEND_RACE_2026_06_11 (never committed to this repo).
        const wasAlreadyWorking = workingFromPhase(paneSnapshot(model.blockId)?.turnPhase ?? { kind: "Idle" });
        // Captured BEFORE TurnStart for the same reason wasAlreadyWorking is:
        // TurnStart unconditionally clears state.failure (reducer.ts), so a
        // read taken any later (e.g. inside deliverToBackend's guard) would
        // always see it already gone, whether this send is a user's own
        // fresh keystroke while a live "auth"-classified failure is still
        // showing, or a legitimate auto-retry after successful recovery.
        // onRecovered (above) explicitly dispatches FailureCleared before
        // calling retryLastTurn precisely so this capture reads null for
        // that case — for a real live auth failure the user hasn't
        // acknowledged, nothing has cleared it yet, so this reads the actual
        // failure. Codex P1 on PR #2338; captures the failure DATA (not just
        // a boolean) so a rejected send can re-dispatch it and restore the
        // banner instead of leaving it cleared with no recovery affordance
        // (Codex P1, third re-review).
        const liveFailure = paneModel.state.failure;
        // Carry the WHOLE PaneFailure, not just `.data`: `turnAttempted` lives
        // on the wrapper, and capturing only the inner AgentFailure discarded
        // it structurally — so the guard's re-dispatch below rebuilt the
        // failure with the reducer's `?? true` default and silently flipped a
        // pre-launch "Log in" row into "Login Again" (+ retryAfterLogin true,
        // i.e. an old message resent on an agent that never ran a turn).
        // Found independently by codex and manoz on PR #2951.
        const authFailureToPreserve = isAuthFailure(liveFailure) ? liveFailure : null;
        // Only start a NEW turn when the agent is idle. Dispatching TurnStart
        // while a turn is already running regresses Streaming → Submitting,
        // which would flicker the busy indicator back to its "Submitting"
        // look for no reason. A queued-while-busy message rides the running
        // turn; the queue-drain (agent-message-accepted) re-enters Submitting
        // if needed.
        if (!wasAlreadyWorking) {
            paneModel.dispatchPane({ type: "TurnStart", at: Date.now(), content: message }, "user");
        }
        return commands.sendMessage(message, wasAlreadyWorking, authFailureToPreserve, attachments);
    };

    // Esc on an empty composer. Mirrors Claude Code CLI: if a message is
    // already queued behind a running turn, deliver it to the live agent
    // right now instead of waiting for the next tool-boundary/idle
    // auto-flush — "stop and consider this now." Must NOT also call
    // stopAgent(): killing the process here would destroy the very live
    // session the delivery just wrote into (persistent controllers only
    // steer while the process stays alive). Only fall back to stopAgent()
    // (SIGINT) when there's nothing queued to steer with.
    // See SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06.md.
    const handleEscapeOnEmptyComposer = (): void => {
        if (commands.hasHeldMessages()) {
            void commands.flushHeldMessages();
            return;
        }
        commands.stopAgent();
    };

    // Failure-recovery accessory row (per-error-class actions + a bounded
    // auto-retry ladder for transient throttling — see AUTO_RETRY_BACKOFF_S in
    // useAgentFailure.ts). SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.
    const retryLastTurn = () => {
        const last = [...getDocument()].reverse().find((n) => n.type === "user_message");
        const msg = last && "message" in last ? (last as { message?: string }).message : undefined;
        if (msg) {
            void handleSendMessage(msg);
        } else {
            // No prior user message (e.g. the agent failed on its first
            // launch/spawn) — fall back to respawning the agent rather than
            // silently dismissing the row. Spec §5.1.
            log("agent", "Retry — no prior message to re-send; relaunching the agent");
            void status.startLaunchFlow();
        }
    };
    // Pre-launch auth failure → the "Not signed in" failure row
    // (failure/useAuthHealth.ts).
    useSyntheticAuthRow({ canRetry: status.canRetry, paneModel });

    // Bind/switch account for this agent (failure/useAccountBinding.ts).
    const { authEmail, bindCandidates, onBindAccount, onSwitchAccount, refreshLinkedAccountId } = useAccountBinding({
        agentDefinitionId: () => getBlockMetaKeyAtom(model.blockId, "agentId")() as string | undefined,
        providerId: () => provider()?.id,
        bindExistingAccount: status.bindExistingAccount,
    });

    // Declaring auth healthy, and the auto-unblock after a bind from anywhere
    // (failure/useAuthHealth.ts).
    const { declareAuthHealthy } = useAuthHealth({
        blockId: model.blockId,
        agentDefinitionId: () => getBlockMetaKeyAtom(model.blockId, "agentId")() as string | undefined,
        paneModel,
        status,
        refreshLinkedAccountId,
    });

    const failureUI = useAgentFailure({
        blockId: model.blockId,
        // Per-pane model keeps dispatch sites default-safe; see useAgentStream above.
        model: paneModel,
        isDormant: hidden,
        failure: (() => paneModel.state.failure),
        onRetry: retryLastTurn,
        // live_elsewhere — take the agent over from the other AgentMux
        // instance running it, then re-run the refused turn if there was one
        // (SPEC_AGENT_SINGLE_LIVE_INSTANCE_2026_09_24.md §4.6).
        onTakeOver: async (turnAttempted: boolean) => {
            log("agent", "Take over — asking the other AgentMux instance to hand this agent over");
            const r = await requestAgentTakeover(model.blockId);
            log("agent", r.released ? `Take over — released by ${r.fromChannel ?? "the other instance"}` : "Take over — nobody else was running it");
            if (turnAttempted) retryLastTurn();
        },
        onOpenArmory: () => void openOrFocusPaneByView("armory"),
        // context_exceeded recovery — archive the over-full session and return
        // to the picker for a clean relaunch (resuming would only re-fail).
        onNewSession: () => {
            log("agent", "New session — archiving the over-full context and returning to the picker");
            void model.startNewSession();
        },
        // P2 — real re-auth. An auth failure is a *CLI-provider* login lapse
        // (e.g. claude's subscription OAuth expired), not an Armory
        // service account. We must FORCE the login rather than re-run the gated
        // launch flow: a 401 means the token is bad, but `CheckCliAuth` still
        // reports it present (expired-but-present false positive), so the gated
        // flow would trust the check, skip login, and do nothing — the exact bug
        // this fixes. `relogin()` bypasses the check and always opens the OAuth
        // (SPEC_REAUTH_FROM_AUTH_ERROR §11). The running persistent agent
        // re-reads its credential per request, so the next message uses the new
        // token and clears this failure row.
        onLoginAgain: (turnAttempted: boolean) => {
            // `retryAfterLogin` mirrors whether a turn actually ran before this
            // failure. The pre-launch case (turnAttempted false — what used to
            // render its own blue "Log in" bar) must pass false: no turn was
            // ever sent, and re-running "the failed turn" would re-send this
            // agent's last OLD message as a new one on a successful login.
            // See docs/specs/PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md.
            log(
                "auth",
                turnAttempted
                    ? "Login Again — forcing a fresh provider login"
                    : "Log in — starting the provider login (no turn to retry)",
            );
            void status.relogin({ retryAfterLogin: turnAttempted });
        },
        // Open a real console window (CREATE_NEW_CONSOLE) so the browser OAuth
        // can launch. Polls for new credentials and seeds when they appear.
        onLoginViaTerminal: (turnAttempted: boolean) => {
            log("auth", "Login via terminal — opening a console window for browser login");
            void status.loginViaTerminal({ retryAfterLogin: turnAttempted });
        },
        // Labelled for the failure row's "Bind: <account>" (email, else name).
        bindCandidates: () => bindCandidates().map((a) => ({ id: a.id, name: accountLabel(a) })),
        onBindAccount,
    });

    // Held "send now" messages go out at the next tool call or turn end
    // (hooks/useTurnReconciliation.ts).
    useHeldMessageDelivery({
        paneModel,
        hasHeldMessages: commands.hasHeldMessages,
        flushHeldMessages: commands.flushHeldMessages,
        flushPendingControllerRefresh: commands.flushPendingControllerRefresh,
    });

    // On first connect (no existing session), send the startup sequence as the
    // opening turn (startup/sendStartupSequence.ts).
    onReadyFn = () =>
        sendStartupSequence({
            sessionId: () => block()?.meta?.["agent:sessionid"],
            agent: currentAgent,
            providerDisplayName: () => provider()?.displayName ?? providerKey(),
            workDir: () => block()?.meta?.["cmd:cwd"] ?? "",
            version: () => getApi().getAboutModalDetails().version,
            peerAgents: agentDefinitions,
            getAgentContent: (contentType) =>
                RpcApi.GetAgentContentCommand(TabRpcClient, { agent_id: agentId, content_type: contentType }),
            getBundle: (id) => RpcApi.GetBundleCommand(TabRpcClient, { id }),
            listIdentities: () => RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agentId }),
            loadAccounts,
            send: handleSendMessage,
            log,
        });

    // Signal-based jump command. AgentDocumentView reacts via a
    // createEffect and scrolls inside its own container — no mutable
    // refs crossing component boundaries. See hooks/useScrollToNode.ts.
    const scroll = useScrollToNode();

    // In-session search: matches, navigation, highlight. Searches over
    // the currently-loaded document slice only.
    const search = useInSessionSearch({
        document: paneModel.document,
        jumpTo: scroll.jumpTo,
    });

    // Pane-scoped Ctrl+F listener. See hooks/useAgentKeyboard.ts.
    useAgentKeyboard({
        blockId: model.blockId,
        onToggleSearch: () => {
            // This component only ever represents the live view now — Agent
            // History is a separate pane tab (AgentHistoryTabView), a
            // separate block/component instance entirely, so there's no
            // "history mode" of THIS component to guard against anymore.
            // Second Ctrl+F press closes and clears state.
            if (search.visible()) {
                search.close();
            } else {
                search.setVisible(true);
            }
        },
    });

    // Per-pane zoom: read term:zoom from block meta (same key as terminal panes).
    const zoomFactor = createMemo(() => readZoom(block()?.meta));

    // Persistence is owned by the universal zoom framework — see the
    // note below where the inline handlers were removed.

    // File drops: this pane's hook for the window-level file-drop controller
    // (tray, or copy + `@filename`). SPEC_DRAG_AND_DROP_CONSOLIDATION §5.3.
    useAgentDropAttach({
        blockId: model.blockId,
        rootRef: () => rootRef,
    });

    // Track pane width → PTY cols so tools running inside the agent CLI
    // (git, ls, claude, …) wrap their output to match the pane instead of
    // the hard-coded `cols: 80` the PTY was opened with. ResizeObserver
    // on the root element; debounced 150 ms; one ControllerInputCommand
    // per net size change. See docs/analysis/AGENT_PANE_PTY_WRAP_2026_05_23.md.
    // Caveat: already-captured live-log lines stay at their original wrap.
    usePtyWidth({
        blockId: model.blockId,
        elementRef: () => rootRef,
        log,
    });

    // Zoom input is handled by the universal framework — `keymodel.ts`
    // intercepts Ctrl+/-/0 and dispatches to `zoomIn/Out/Reset`, and
    // `app.tsx` routes Ctrl+Wheel to `zoomBlockIn/Out`. Both call
    // paths probe the focused block's `viewType`, and `viewType ===
    // "agent"` is explicitly supported (see `zoom.ts::getBlockZoom`).
    // The universal flow writes `term:zoom` on block meta, which we
    // read back via `zoomFactor()` and apply on the root div below.
    //
    // Earlier this file had its own Ctrl+Wheel + Ctrl+±/0 handlers
    // attached in capture phase. They fought the universal handlers
    // (different step size, both writing the same key, agent's
    // stopPropagation pre-empting the zoom indicator), which broke
    // zoom in the agent pane. Deleted.

    // Context menu for copy
    const handleContextMenu = (e: MouseEvent) => {
        // A registered context-menu region under the click (the Shell drawer)
        // owns its own menu — this handler runs first (it is a descendant of
        // blockframe's), so without yielding a transcript selection would win
        // and show a Copy for the wrong text inside the terminal.
        if (resolveContextMenuRegion(e.target, e.currentTarget as Element)) return;
        const sel = window.getSelection()?.toString();
        if (!sel) return; // no selection, let default behavior
        e.preventDefault();
        ContextMenuModel.showContextMenu([{ label: "Copy", click: () => clipboardWriteText(sel) }], e);
    };

    return (
        // Dormancy is provided at the SUBTREE root, not threaded as a prop:
        // the expensive consumer (MarkdownBlock) sits four layers down, behind
        // virtualization code that is performance-critical and deliberately
        // tuned, and should not grow another prop it would only forward.
        // See `agent-dormancy.tsx` for why rendering (not data) is what gets
        // gated.
        // `hidden` covers a dormant pane-stack member and a hidden window tab
        // alike; this provider gates only rendering (the timers read `hidden`
        // directly). The same providers carry the cwd for inline media.
        <AgentPaneProviders dormant={hidden} block={block} agent={currentAgent} loadToolResult={liveFeed.loadToolResult}>
            {/* Pane-scope `<ModalLayer>` lives in AgentBlockContent (this
                component's own parent) so it covers BOTH this presentation view
                AND the picker fallback. Anything in this subtree that calls
                `useModalLayer()` resolves to that pane-scope layer. */}
        <div
            ref={rootRef}
            class="agent-view agent-view--presentation"
            // NOTE: `zoom` lives on `.agent-view-zoomed` below, NOT here. The Shell
            // drawer must render in an UNSCALED coordinate space: xterm measures cells
            // via getBoundingClientRect (visual px) but customFit reclaims width from
            // clientWidth (layout px), and CSS `zoom` makes those two disagree — which
            // produced fractional cell sizes (clipped top row, since the grid is
            // bottom-anchored) and a link layer drawn into a 445x210 buffer but
            // displayed at 307x145 (misplaced hover underlines). xterm.js does not
            // support being rendered inside a CSS-scaled subtree (xtermjs/xterm.js
            // #2584, #3242). `--agent-pane-zoom` is still published here for other
            // consumers. See SPEC_AGENT_SHELL_DRAWER_ZOOM_COORDINATE_SPACE_2026_09_20.md.
            style={{ "--agent-pane-zoom": String(zoomFactor()) }}
            onContextMenu={handleContextMenu}
            tabIndex={-1}
        >
            {/* Everything that SHOULD scale with pane zoom lives in here. The Shell
                drawer is deliberately a sibling of this element, below. */}
            {/* Loading overlay — covers the pane from mount until the initial
                history load resolves, so a content-heavy pane never sits
                blank while it replays. See
                docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md.

                Deliberately a DIRECT child of `.agent-view`, OUTSIDE
                `.agent-view-zoomed`. It is `position: absolute; inset: 0`
                (PaneLoadingCover.scss), so it covers its nearest POSITIONED
                ancestor — and the zoomed wrapper is `position: relative`. Nested
                inside it, the overlay stopped covering the Shell drawer (a sibling
                of the wrapper), leaving an open drawer visible and uncovered for
                the whole load. Keeping it out here also keeps it unscaled, so the
                cover can't be 69%-sized by the pane zoom.
                See REPORT_AGENT_PANE_LOADING_UI_2026_09_20.md §F. */}
            <PaneLoadingCover phase={readiness.phase} />
            {/* Shutdown log while the pane closes in place (SPEC_AGENT_SELF_QUIT_2026_09_24.md §5.5). */}
            <ShutdownOverlay blockId={model.blockId} agentName={agentName()} />
            {/* Stash drawer: a DIRECT flex child of `.agent-view`, outside
                `.agent-view-zoomed` (components/AgentStashDrawer.tsx). */}
            <AgentStashDrawer
                open={paneModel.state.stashOpen}
                blockId={model.blockId}
                persistedHeight={block()?.meta?.["agent:stashheight"] as number | undefined}
                agentId={agentId}
                agentName={agentName()}
                workingDirectory={(block()?.meta?.["cmd:cwd"] as string) || currentAgent()?.working_directory || ""}
                hasDefinition={currentAgent() != null}
            />
            <div class="agent-view-zoomed" style={{ zoom: zoomFactor() }}>
            {/* Portaled above the tab strip; see components/AgentProgressBar.tsx. */}
            <AgentProgressBar
                mount={progressBarMount}
                active={paneBusy()}
                stopping={isStopping(paneModel.state.turnPhase)}
            />
            {/* /btw side-question overlay — ephemeral, floats over the whole
                pane (position: absolute against .agent-view, styles/_btw.scss),
                NOT part of the persisted layout tree and NOT gated on the
                pane's own turn state: it must stay usable, and non-blocking,
                whether or not a real agent turn is streaming. State is owned
                by useAgentCommands (mirrors helpVisible/pickerSpec below). */}
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
            {/* Pane title + back button now live in the block frame header,
                driven by AgentViewModel.viewName / viewIcon / endIconButtons.
                See SPEC_AGENT_PANE_FOLLOWUPS item #8. */}

            {/* Tab strip now lives in AgentPaneChrome — see its comment. */}

            {/* This component only ever represents the live view now —
                Agent History is a separate pane tab (AgentHistoryTabView),
                not a swap-in-place body of this one. See
                SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.1. */}
            <AgentSearchBar
                visible={search.visible}
                onSearch={search.performSearch}
                onNext={search.next}
                onPrev={search.prev}
                onClose={search.close}
                matchIndex={search.currentIndex}
                matchCount={search.matchCount}
            />

            {/* The "Earlier conversations / Open Agent History" link used to
                render here as a PaneRow pinned above the scroll region —
                now it's a `history_link` synthetic DOCUMENT NODE, injected
                by injectHistoryLink into displayDocument below, so it
                scrolls with the transcript instead of staying fixed in
                place. See SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.2. */}
            {/* Scroll region wrapper — .agent-document (inside AgentDocumentView)
                is absolutely positioned to fill this box.

                AgentWorkingRow used to float over this box's bottom edge
                (SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md
                §3.2) so the message list's scrollbar could run the full height
                of the region. As of
                SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md the row is
                a normal-flow sibling below the dock instead — which reaches
                the same goal more simply, since a row that is no longer
                *inside* this box cannot cover the scrollbar or the last
                message in the first place. That removed the overlay's whole
                support apparatus: the height-measuring ResizeObserver, the
                --agent-working-row-height custom property, .agent-document's
                matching padding-bottom reservation, and the full-width
                backdrop that existed only to color the scrollbar gutter the
                overlay had to stay inset from
                (SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md). */}
            <div class="agent-document-scroll-region">
                <AgentDocumentView
                    documentNodes={displayDocument}
                    documentStateAtom={agentAtoms().documentStateAtom}
                    onOpenHistory={() => void openOrFocusHistoryTab({ currentBlockId: model.blockId, agentId })}
                    onAgentErrorLogin={() => {
                        // Must match onLoginAgain above: the button is labeled "Login
                        // Again", so it has to force a fresh OAuth regardless of
                        // provider. A prior version special-cased Claude into
                        // a seed-from-global path instead — silently reusing the
                        // (possibly equally-stale) personal credential under a
                        // "Login Again" label, exactly the kind of no-op this
                        // button exists to avoid
                        // (retro-agent-auth-relogin-noop-2026-07-01). That path
                        // was removed outright 2026-08-31 (per-channel auth
                        // enforcement), so the trap is now structural, not just
                        // a convention to uphold here.
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

            {/* Login, decisions, questions, queue, recovery banners, activity
                dock, working row and session notices
                (components/AgentBottomPanels.tsx). */}
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
                hidden={hidden}
                pendingMessages={pendingMessages}
                failureRow={failureUI.row}
                backgroundTasksAtom={backgroundTasksAtom}
                workingRowVisible={workingRowVisible}
                workingRowLoading={workingRowLoading}
                hasPromotedTool={hasPromotedTool}
            />

            {/* Composer status strip — single 28-32px row with live
                activity ticker and Log button that toggles the log panel.
                State (detailsOpen) is reducer-owned (PR #1068). */}
            <AgentComposerStrip
                sessionTotals={paneModel.state.sessionTotals}
                loading={paneBusy()}
                logOpen={paneModel.state.detailsOpen}
                onToggleLog={() => paneModel.dispatchPane({ type: "DetailsToggle" }, "user")}
                contextTokens={(paneModel.state.lastContextTokens ?? null)}
                contextWindow={(paneModel.state.lastContextWindow ?? null) ?? provider()?.contextWindow}
                lastReplyModel={paneModel.state.lastContextModel}
                authStatus={loginStatus()}
                authEmail={authEmail()}
                canSwitchAccount={bindCandidates().length > 0}
                onSwitchAccount={onSwitchAccount}
                blockId={model.blockId}
                blockAtom={block}
                providerId={provider()?.id ?? ""}
                agentMode={block()?.meta?.["agentMode"] as string | undefined}
                compacting={paneModel.state.compacting}
                // Route through handleSendMessage — same pattern as the
                // SlashHelpPanel's onInvoke above, and for the same reason:
                // this needs the same pre-TurnStart wasAlreadyWorking
                // snapshot the composer path computes, not a bare
                // commands.sendMessage() call (which would default
                // wasAlreadyWorking to false regardless of the pane's real
                // turn state). "/compact" is deliberately NOT a registered
                // SlashCommand — see AgentComposerStrip's onCompact doc
                // comment for why that would break instead of help.
                onCompact={() => {
                    void handleSendMessage("/compact");
                }}
            />

            <div class="agent-composer-region">
                <Show when={commands.helpVisible()}>
                    <SlashHelpPanel
                        commands={commands.availableCommands()}
                        onInvoke={(cmd) => {
                            commands.closeHelp();
                            // Route through handleSendMessage — NOT
                            // commands.sendMessage directly — so this gets
                            // the same pre-TurnStart wasAlreadyWorking
                            // snapshot the composer path computes. Codex P1
                            // on PR #2338 (twelfth re-review): calling
                            // sendMessage() bare defaults wasAlreadyWorking
                            // to false regardless of whether a turn is
                            // actually active, so invoking /login from this
                            // panel during an active turn made
                            // isTurnActive() lie and finalizeLoginSuccess()
                            // force-restart (killing) that turn.
                            void handleSendMessage(`/${cmd.name}`);
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
                    onSendMessage={handleSendMessage}
                    onTyping={() => {
                        scrollToBottomFn?.("typing");
                    }}
                    onStopAgent={handleEscapeOnEmptyComposer}
                    onRecallLatestQueued={commands.recallLatestHeld}
                    getCompletions={commands.completions}
                    viewModel={model}
                    isComposerEmptyRef={(fn) => {
                        composerIsEmptyFn = fn;
                    }}
                />
            </div>
            </div>
            {/* Shell drawer: outside `.agent-view-zoomed`, a flex child of
                `.agent-view` (components/AgentShellDrawer.tsx). */}
            <AgentShellDrawer
                open={paneModel.state.detailsOpen}
                blockId={model.blockId}
                shellSubBlockId={block()?.meta?.["term:shellsubblockid"] as string | undefined}
                cwd={block()?.meta?.["cmd:cwd"] as string | undefined}
                persistedHeight={block()?.meta?.["term:shellheight"] as number | undefined}
                onTermReady={handleShellTermReady}
                onTermDispose={handleShellTermDispose}
                onShellExited={handleShellExited}
            />
        </div>
        </AgentPaneProviders>
    );
};

AgentPresentationView.displayName = "AgentPresentationView";
