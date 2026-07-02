// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { getProvider } from "./providers";
import { createAgentAtoms } from "./state";
import {
    dispatchIfRegistered as dispatchDocIfRegistered,
} from "@/app/store/agent-document-store";
import {
    dispatch as dispatchPane,
    dispatchIfRegistered as dispatchPaneIfRegistered,
    snapshot as paneSnapshot,
} from "@/app/store/agent-pane-state-store";
import {
    registerPane as registerAgentPane,
    unregisterPane as unregisterAgentPane,
    type AgentPaneModel,
} from "@/app/store/agent-pane-registration";
import {
    registerActivity as registerAgentActivity,
    unregisterActivity as unregisterAgentActivity,
} from "@/app/store/agentActivity";
import {
    registerPane as registerLayoutPane,
    snapshot as layoutSnapshot,
    unregisterPane as unregisterLayoutPane,
    type LayoutView,
} from "@/app/store/agent-pane-layout-store";
import { isInterruptibleTurn, workingFromPhase } from "@/app/store/agent-pane-state/types";
import type { SubagentLinkNode } from "./types";
import { openSubagentPane, isSubagentPaneOpen } from "@/app/store/subagent-pane-manager";
import { getRecentDispatches } from "@/app/store/command-source";
import { getTrail } from "@/log/render-trail";
import { useAgentStream } from "./useAgentStream";
import { useActivityLog } from "./hooks/useActivityLog";
import { useHistoryPagination } from "./hooks/useHistoryPagination";
import { useAgentControllerStatus } from "./hooks/useAgentControllerStatus";
import { useInSessionSearch } from "./hooks/useInSessionSearch";
import { useScrollToNode } from "./hooks/useScrollToNode";
import { useAgentKeyboard } from "./hooks/useAgentKeyboard";
import { useProcessCount } from "./hooks/useProcessCount";
import { usePtyWidth, computeTermSizeFromEl } from "./hooks/usePtyWidth";
import { useSubagentEvents } from "./hooks/useSubagentEvents";
import { useControllerStatusEvents } from "./hooks/useControllerStatusEvents";
import { useBlockActivity } from "./hooks/useBlockActivity";
import { useAgentActivitySummary } from "./hooks/useAgentActivitySummary";
import { useAgentCommands } from "./hooks/useAgentCommands";
import { useAgentFailure } from "./hooks/useAgentFailure";
import { PaneRow } from "./components/PaneRow";
import { useAgentDropAttach } from "./hooks/useAgentDropAttach";
import { useSnapshotPersistence } from "./hooks/useSnapshotPersistence";
import { useAgentDecisions } from "./hooks/useAgentDecisions";
import { useAgentQuestions } from "./hooks/useAgentQuestions";
import { useAgentCloseConfirm } from "./hooks/useAgentCloseConfirm";
import { handleAgentIdChange } from "@/app/view/term/termagent";
import { DragOverlay } from "@/app/element/dragoverlay";
import { AgentControlBar } from "./components/AgentControlBar";
import { ActivityDock } from "./components/ActivityDock";
import { ActivityLogPanel } from "./components/ActivityLogPanel";
import { AgentDecisionPanel } from "./components/AgentDecisionPanel";
import { AgentQuestionPanel } from "./components/AgentQuestionPanel";
import { AgentDisconnectedBanner } from "./components/AgentDisconnectedBanner";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter, AgentWorkingRow } from "./components/AgentFooter";
import { AgentComposerStrip } from "./components/AgentComposerStrip";
import { PendingMessagesPanel } from "./components/PendingMessagesPanel";
import { AgentPicker, useAgentDefinitions } from "./components/AgentPicker";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { SlashCommandPicker } from "./components/SlashCommandPicker";
import { SlashHelpPanel } from "./components/SlashHelpPanel";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createBlock, getApi, openOrFocusPaneByView, WOS } from "@/app/store/global";
import { ConfirmModal } from "@/element/modal";
import { ModalLayer } from "@/element/ModalLayer";
import { useModalLayer } from "@/element/modal-layer";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { parseAgentAccounts, loadAccounts } from "@/app/view/identity/identity-model";
import { buildStartupPayload, resolveAccounts } from "./startup/buildStartupPayload";
import "./agent-view.scss";

/**
 * Top-level wrapper — switches between agent picker and presentation view.
 *
 * The pane-scope `<ModalLayer>` wrap lives HERE (not inside
 * AgentPresentationView) because the launch picker — which uses
 * `useModalLayer()` to open the launch / install / new-bundle /
 * create-from-template modals — runs in the fallback branch BEFORE
 * an agentId exists. If the layer wrapped only the presentation
 * view, every first-launch / template-launch call site would resolve
 * `useModalLayer()` to the outer tab-scope layer and the modal would
 * inert the whole tab instead of just this pane. Wrapping at the
 * wrapper covers both the pre-launch picker AND the post-launch
 * presentation view, so the pane-scope lock holds across the entire
 * pane lifecycle. SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md.
 */
export const AgentViewWrapper = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const block = model.blockAtom;
    const agentId = () => block()?.meta?.["agentId"];

    return (
        <ModalLayer scope="pane">
            <Show
                when={agentId()}
                fallback={<AgentPicker model={model} />}
            >
                <AgentPresentationView model={model} agentId={agentId()} />
            </Show>
        </ModalLayer>
    );
};

AgentViewWrapper.displayName = "AgentViewWrapper";

// Launch flow lives in `flows/launch-flow.ts` — Step 2 of
// specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.

const AgentPresentationView = ({ model, agentId }: { model: AgentViewModel; agentId: string }): JSX.Element => {
    const block = model.blockAtom;
    const providerKey = (): string => block()?.meta?.["agentProvider"] ?? agentId;
    const provider = () => getProvider(providerKey());
    const outputFormat = (): string => block()?.meta?.["agentOutputFormat"] ?? "claude-stream-json";
    // Human-readable display name for the composer placeholder. Matches
    // the same fallback chain used by the onMount log() on line 380 —
    // single source of truth for "what to call this agent in the UI."
    // Reactive: the textarea placeholder updates if the user renames
    // the agent without remounting the pane.
    const agentName = (): string => block()?.meta?.["agentName"] ?? agentId;

    // Reactive agent-definition list — used to resolve the current AgentDefinition object
    // for identity/memory modal requests.
    const agentDefinitions = useAgentDefinitions();
    const currentAgent = createMemo(() => agentDefinitions().find((a) => a.id === agentId));

    // Wire pane-scoped modal callbacks into the model so the title-bar icon
    // buttons (brain / id-card) can open modals without holding a SolidJS
    // context in the model. Mirrors the former _setOverlayTab pattern.
    const modalLayer = useModalLayer();
    // Drive model._agentDefLoaded via a createEffect so endIconButtons
    // has a proper SolidJS reactive dependency on the async definition
    // list. A plain function-mutation (_hasAgentDef = () => ...) would
    // not create a tracking subscription — BlockFrame evaluates
    // endIconButtons once with the default () => false and the id-card
    // button never appears without a reactive signal atom.
    createEffect(() => {
        model._agentDefLoaded._set(currentAgent() != null);
    });
    onMount(() => {
        model._openIdentityModal = () => {
            const agent = currentAgent();
            if (!agent) return;
            modalLayer.open({ kind: "agent-identity", agent, blockId: model.blockId });
        };
        model._openMemoryModal = () => {
            // Prefer cmd:cwd (actual launch cwd, set by launchAgentDefinition)
            // over AgentDefinition.working_directory, which is often empty or a
            // stale default for template-launched and continuation agents.
            const block = model.blockAtom();
            const workingDirectory =
                (block?.meta?.["cmd:cwd"] as string) ||
                currentAgent()?.working_directory ||
                "";
            modalLayer.open({
                kind: "agent-memory",
                agentId,
                agentName: agentName(),
                workingDirectory,
            });
        };
    });
    onCleanup(() => {
        model._openIdentityModal = null;
        model._openMemoryModal = null;
        model._agentDefLoaded._set(false);
    });

    const agentAtoms = createMemo(() => createAgentAtoms(model.blockId));

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
    let paneModel: AgentPaneModel;
    {
        const a = agentAtoms();
        paneModel = registerAgentPane(model.blockId, {
            agentId,
            documentSetter: a.documentAtom[1],
            projections: {
                streaming: a.streamingStateAtom[1],
                sessionStats: a.sessionStatsAtom[1],
                currentTool: a.currentToolAtom[1],
                turnTokens: a.turnTokensAtom[1],
                contextTokens: a.contextTokensAtom[1],
                contextWindow: a.contextWindowAtom[1],
                pending: a.pendingMessagesAtom[1],
                initPhase: a.initPhaseAtom[1],
                turnPhase: a.turnPhaseAtom[1],
                detailsOpen: a.detailsOpenAtom[1],
                currentToolArg: a.currentToolArgAtom[1],
            },
        });
        registerAgentActivity(model.blockId, a.turnPhaseAtom[0]);

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
            const phase = a.turnPhaseAtom[0]();
            const midTurn = workingFromPhase(phase);
            if (midTurn) {
                console.warn(
                    `[agent-view] DISPOSE UNEXPECTED(mid-turn) blockId=${model.blockId.slice(0, 7)} turnPhase=${JSON.stringify(phase)} stack=${new Error().stack}`,
                );
                try {
                    console.warn(`[agent-view] DISPOSE mid-turn render_trail=${JSON.stringify(getTrail())}`);
                    console.warn(
                        `[agent-view] DISPOSE mid-turn recent_dispatches=${JSON.stringify(getRecentDispatches(40))}`,
                    );
                } catch { /* best-effort diagnostic */ }
            }
            unregisterAgentPane(model.blockId);
            unregisterAgentActivity(model.blockId);
            handleAgentIdChange(model.blockId, undefined);
        });

        // Mirror context token count to block meta so the Swarm view can read
        // it without needing access to per-pane in-memory signals. Fires at
        // most once per turn (TokensIn at message_start).
        createEffect(() => {
            const tokens = a.contextTokensAtom[0]();
            void RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", model.blockId),
                meta: { "term:ctx-tokens": tokens ?? null } as any,
            });
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
        (window as unknown as { __agentLayout?: () => unknown }).__agentLayout = () =>
            layoutSnapshot(model.blockId);
    }

    // Activity log — collects per-session diagnostic entries from launch
    // flow, subprocess lifecycle, slash commands, errors, etc. Rendered
    // in the collapsible `<ActivityLogPanel>` above the composer.
    // `log` is passed down to every hook whose signature takes a `LogFn`.
    const { lines: logLines, append: log } = useActivityLog();

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
    // a non-empty string is guaranteed at this point by the `Show
    // when={agentId()}` gate in AgentViewWrapper above.
    // Bridged callback: AgentDocumentView registers viewState.markHistoryReady
    // here on mount (before any async history work starts), so
    // useHistoryPagination can signal "history done" into the viewState
    // that lives inside AgentDocumentView. This drives the enter-animation
    // gate on the streaming buffer.
    let historyReadyFn: (() => void) | undefined;
    const history = useHistoryPagination({
        blockId: model.blockId,
        model: paneModel,
        outputFormat,
        definitionId: agentId,
        onHistoryReady: () => historyReadyFn?.(),
        // Schema v2: apply DocumentState + pane overlay after NDJSON replay.
        onSnapshotOverlay: ({ documentState, detailsOpen }) => {
            const [, setDocState] = agentAtoms().documentStateAtom;
            setDocState((prev) => ({ ...prev, ...documentState }));
            if (typeof detailsOpen === "boolean") {
                const [, setDetailsOpen] = agentAtoms().detailsOpenAtom;
                setDetailsOpen(detailsOpen);
            }
        },
        log,
    });

    // Auth + launch flow state and the onCleanup that kills the CLI
    // if the pane closes mid-login.
    // `getDocument` is read-only; for writes we MUST dispatch through
    // agent-document-store so slot.state stays in sync.
    const [getDocument] = agentAtoms().documentAtom;

    // Agent-pane state-persistence (RFC #857 + spec
    // SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md). See
    // hooks/useSnapshotPersistence.ts.
    useSnapshotPersistence({
        blockId: model.blockId,
        definitionId: agentId,
        getAtoms: agentAtoms,
        getDocument,
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
    const { pendingQuestions, handleAnswer } = useAgentQuestions({
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

    const status = useAgentControllerStatus({
        blockId: model.blockId,
        provider,
        log,
        onLoginSuccess: (email) => {
            const display = email ? `Logged in as **${email}**` : "Login successful";
            const node: import("./types").MarkdownNode = {
                type: "markdown",
                id: `login_success_${Date.now()}`,
                content: `\u2713 ${display}`,
            } as import("./types").MarkdownNode;
            dispatchDocIfRegistered(model.blockId, {
                type: "StreamFlush",
                newNodes: [node],
                updatedNodes: [],
            });
        },
        onReady: () => onReadyFn?.(),
        getInitialTermSize: () => computeTermSizeFromEl(rootRef),
    });

    onMount(() => {
        const name = block()?.meta?.["agentName"] ?? agentId;
        const provName = provider()?.displayName ?? providerKey();
        const cwd = block()?.meta?.["cmd:cwd"] ?? "";
        log("agent", `${name} selected (provider: ${provName})`);
        if (cwd) log("env", `working directory: ${cwd}`);
        status.startLaunchFlow();
    });

    // Log controllerstatus events as they stream in.
    useControllerStatusEvents({ blockId: model.blockId, log });

    // Subscribe to Claude Code OSC window-title extractions and write them
    // to term:activity block metadata for the tab label.
    useBlockActivity({ blockId: model.blockId });

    // Haiku-powered live mini-summary: generates a fresh phrase in the pane header
    // on every completed agent turn, replacing the non-functional OSC path.
    useAgentActivitySummary({
        blockId: model.blockId,
        turnPhase: agentAtoms().turnPhaseAtom[0],
        getRootWidth: () => rootRef?.offsetWidth,
    });

    // Subagent event subscriptions. See hooks/useSubagentEvents.ts.
    useSubagentEvents({
        blockId: model.blockId,
        documentAtom: agentAtoms().documentAtom,
        log,
    });

    // Count of OS processes currently tracked for this block — drives
    // the `⚙ N` badge on the status line. Silently returns 0 on
    // platforms without a real tracker. See `hooks/useProcessCount.ts`.
    const processCount = useProcessCount(model.blockId);

    // Pane-close confirm: when the user closes this pane with tracked
    // processes still alive, intercept the layout's `onClose` and raise
    // a ConfirmModal. See hooks/useAgentCloseConfirm.ts.
    const { closeConfirm, setCloseConfirm, handleCloseConfirmAccept } = useAgentCloseConfirm({
        blockId: model.blockId,
        model,
        processCount,
    });

    // Subscribe to subprocess output and parse into DocumentNodes.
    // Mutations dispatch through agent-document-store; the reducer there
    // owns dedup against in-flight history loads and the truncate-suppress
    // invariant that prevents the mid-session wipe bug.
    const pendingMessagesAtom = agentAtoms().pendingMessagesAtom;
    useAgentStream({
        blockId: model.blockId,
        // Pass the per-pane model so the hook's dispatch sites are
        // default-safe against post-unmount races — the disposed-flag
        // check is centralized in the model rather than per call site.
        model: paneModel,
        outputFormat: outputFormat(),
        documentAtom: agentAtoms().documentAtom,
        // turnPhase is the SoT for "is a stop in flight". useAgentStream
        // needs it to detect user-initiated stops and append the
        // "⏹ Interrupted by user" row when session_end lands.
        turnPhaseAtom: agentAtoms().turnPhaseAtom,
        pendingMessagesAtom,
        enabled: true,
        // Provider id (lowercase catalog key) attributes completed-turn
        // tokens to the correct row in the status-bar token-usage store.
        provider: providerKey(),
    });

    // Mutable ref to the scrollToBottom function exposed by
    // AgentDocumentView. Called by AgentFooter's onTyping when the user
    // starts composing AND by useAgentCommands.onSent after the user's
    // message has been appended to the document (SPEC_AGENT_PANE_FOLLOWUPS
    // item #1). Declared here so both useAgentCommands and the JSX below
    // can close over the same reference; assigned once AgentDocumentView
    // mounts via scrollToBottomRef.
    let scrollToBottomFn: (() => void) | null = null;

    // User-message send + /login /clear slash intercepts + back-to-picker.
    // See hooks/useAgentCommands.ts.
    const commands = useAgentCommands({
        blockId: model.blockId,
        // Per-pane model keeps dispatch sites default-safe; see useAgentStream above.
        model: paneModel,
        block,
        provider,
        documentAtom: agentAtoms().documentAtom,
        log,
        setAuthUrl: status.setAuthUrl,
        backToPicker: () => model.backToPicker(),
        // Scroll the user's own message into view after Enter. The hook
        // defers this to the next animation frame so the mounted node is
        // included in scrollHeight. See SPEC_AGENT_PANE_FOLLOWUPS item #1.
        onSent: () => scrollToBottomFn?.(),
        pendingMessagesAtom,
    });

    // Mark turn as active when the user sends a message — TurnStart
    // also clears stale sessionStats from the prior turn.
    const handleSendMessage = (message: string): Promise<void> => {
        // Bang commands (`!cmd`) output goes to the ActivityLogPanel inside the
        // details region. Auto-open the details panel so the output is immediately
        // visible — without this, the user sees no feedback if the panel is closed.
        if (message.trim().startsWith("!")) {
            agentAtoms().detailsOpenAtom[1](true);
        }
        // Capture working state BEFORE TurnStart so PendingMessageQueued can
        // mark whether this message is queued behind a running turn (true) or
        // is the message that initiated the turn (false). The panel only shows
        // messages with enqueuedWhileBusy:true, preventing the idle-send race
        // where the message flashed in the amber zone between Streaming
        // promotion and agent-message-accepted. See ANALYSIS_IDLE_SEND_RACE_2026_06_11.md.
        const wasAlreadyWorking = workingFromPhase(paneSnapshot(model.blockId)?.turnPhase ?? { kind: "Idle" });
        // Only start a NEW turn when the agent is idle. Dispatching TurnStart
        // while a turn is already running regresses Streaming → Submitting, which
        // hides the "Send now" affordance (isInterruptibleTurn is false during
        // Submitting) until the next stream event — the "panel shows up a couple
        // seconds late" bug. A queued-while-busy message rides the running turn;
        // the queue-drain (agent-message-accepted) re-enters Submitting if needed.
        if (!wasAlreadyWorking) {
            dispatchPane(model.blockId, { type: "TurnStart", at: Date.now() }, "user");
        }
        return commands.sendMessage(message, wasAlreadyWorking);
    };

    // Failure-recovery accessory row (per-error-class actions + 5s auto-retry
    // for transient throttling). SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16.
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
    const failureUI = useAgentFailure({
        blockId: model.blockId,
        onRetry: retryLastTurn,
        onTrustCenter: () => void openOrFocusPaneByView("armory"),
        canSeed: () => provider()?.id === "claude",
        // context_exceeded recovery — drop the over-full session and return to
        // the picker for a clean relaunch (resuming would only re-fail).
        onNewSession: () => {
            log("agent", "New session — clearing the over-full context and returning to the picker");
            void model.backToPicker();
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
        onLoginAgain: () => {
            log("auth", "Login Again — forcing a fresh provider login");
            void status.relogin();
        },
        // Seed-from-global recovery: copy the user's existing valid global
        // Claude login into this agent instead of a fresh OAuth — the reliable
        // path for Claude Code v2.1.x's un-scrapeable login TUI (§5.5). The
        // agent re-reads its credential per request, so the next message clears
        // this failure row with no restart.
        onUseExistingLogin: () => {
            log("auth", "Use existing login — seeding from your global Claude login");
            void status.useGlobalLogin();
        },
        // Open a real console window (CREATE_NEW_CONSOLE) so the browser OAuth
        // can launch. Polls for new credentials and seeds when they appear.
        onLoginViaTerminal: () => {
            log("auth", "Login via terminal — opening a console window for browser login");
            void status.loginViaTerminal();
        },
    });

    // Deliver queued-while-busy ("send now") messages at the next tool-call
    // boundary — the agent finishes its current step and then picks them up
    // (the CLI consumes a stdin message at its next inference, after the
    // in-flight tool's result). Falls back to turn end (Idle/Done) so a
    // tool-less turn still delivers. Holding until here is what lets ArrowUp
    // recall an un-sent message first.
    let prevTool: string | null = null;
    createEffect(() => {
        const tool = agentAtoms().currentToolAtom[0]();
        const phaseKind = agentAtoms().turnPhaseAtom[0]().kind;
        const newToolCall = tool !== null && tool !== prevTool;
        prevTool = tool;
        const turnIdle = phaseKind === "Idle" || phaseKind === "Done";
        if ((newToolCall || turnIdle) && commands.hasHeldMessages()) {
            void commands.flushHeldMessages();
        }
    });

    // On first connect (no existing session), assemble a structured startup
    // payload from agent-definition + Identity data and send it as the opening turn.
    // See docs/specs/SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md
    onReadyFn = async () => {
        // Skip if this is a resumed session
        if (block()?.meta?.["agent:sessionid"]) return;

        try {
            const agent = currentAgent();
            if (!agent) return;

            // Gather inputs in parallel where possible
            const [startupContentResult, version] = await Promise.all([
                RpcApi.GetAgentContentCommand(TabRpcClient, {
                    agent_id: agentId,
                    content_type: "startup",
                }).catch(() => null),
                Promise.resolve(getApi().getAboutModalDetails().version),
            ]);

            // Resolve assigned accounts from Identity localStorage
            const agentAccounts = parseAgentAccounts(agent);
            const accounts = resolveAccounts(agentAccounts, loadAccounts());

            const payload = buildStartupPayload({
                agent,
                providerDisplayName: provider()?.displayName ?? providerKey(),
                workDir: block()?.meta?.["cmd:cwd"] ?? "",
                version,
                accounts,
                peerAgents: agentDefinitions(),
                startupContent: startupContentResult?.content ?? null,
            });

            if (payload) {
                log("agent", "sending startup sequence");
                await handleSendMessage(payload);
            }
        } catch (err) {
            log("warn", `startup sequence failed: ${err}`, "warn");
        }
    };

    // Signal-based jump command. AgentDocumentView reacts via a
    // createEffect and scrolls inside its own container — no mutable
    // refs crossing component boundaries. See hooks/useScrollToNode.ts.
    const scroll = useScrollToNode();

    // In-session search: matches, navigation, highlight. Searches over
    // the currently-loaded document slice only.
    const search = useInSessionSearch({
        document: agentAtoms().documentAtom[0],
        jumpTo: scroll.jumpTo,
    });

    // Pane-scoped Ctrl+F listener. See hooks/useAgentKeyboard.ts.
    useAgentKeyboard({
        blockId: model.blockId,
        onToggleSearch: () => {
            // Second Ctrl+F press closes and clears state.
            if (search.visible()) {
                search.close();
            } else {
                search.setVisible(true);
            }
        },
    });

    // Per-pane zoom: read term:zoom from block meta (same key as terminal panes).
    const zoomFactor = createMemo(() => {
        const meta = block()?.meta;
        const z = meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });

    // Persistence is owned by the universal zoom framework — see the
    // note below where the inline handlers were removed.

    // File-drop attach. Drop a file onto the agent pane → copy it into the
    // agent's CWD AND splice `@filename` into the composer at the caret, so
    // the agent sees it on its next turn. Spec:
    // docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md.
    const dropAttach = useAgentDropAttach({
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
    // "agent"` is explicitly supported (see `zoom.win32.ts::getBlockZoom`).
    // The universal flow writes `term:zoom` on block meta, which we
    // read back via `zoomFactor()` and apply on the root div below.
    //
    // Earlier this file had its own Ctrl+Wheel + Ctrl+±/0 handlers
    // attached in capture phase. They fought the universal handlers
    // (different step size, both writing the same key, agent's
    // stopPropagation pre-empting the zoom indicator), which broke
    // zoom in the agent pane. Deleted.

    // Handle subagent link click — open a subagent pane
    const handleSubagentClick = (node: SubagentLinkNode) => {
        if (isSubagentPaneOpen(node.subagentId)) {
            log("subagent", `pane already open for ${node.slug || node.subagentId}`);
            return;
        }
        openSubagentPane({
            subagentId: node.subagentId,
            slug: node.slug,
            parentAgent: node.parentAgent,
            parentBlockId: model.blockId,
            sessionId: node.sessionId,
        }).then((blockId) => {
            if (blockId) {
                log("subagent", `opened pane for ${node.slug || node.subagentId}`);
            }
        });
    };

    // Context menu for copy
    const handleContextMenu = (e: MouseEvent) => {
        const sel = window.getSelection()?.toString();
        if (!sel) return; // no selection, let default behavior
        e.preventDefault();
        ContextMenuModel.showContextMenu(
            [{ label: "Copy", click: () => clipboardWriteText(sel) }],
            e,
        );
    };

    return (
        // Pane-scope `<ModalLayer>` lives in AgentViewWrapper (above)
        // so it covers BOTH this presentation view AND the picker
        // fallback. Anything in this subtree that calls
        // `useModalLayer()` resolves to that outer pane-scope layer.
        <div
            ref={rootRef}
            class="agent-view agent-view--presentation"
            style={{ zoom: zoomFactor(), "--agent-pane-zoom": String(zoomFactor()) }}
            onContextMenu={handleContextMenu}
            tabIndex={-1}
        >
            {/* Gradient progress bar — 2px, pinned position:absolute top:0.
                Animated shimmer while working, hidden at rest. Colors derived
                from --accent-color via color-mix() so it adapts to all themes.
                See SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md §4. */}
            <div
                class="agent-pane-progress-bar"
                classList={{
                    "agent-pane-progress-bar--active": status.isLoading() || workingFromPhase(agentAtoms().turnPhaseAtom[0]()),
                    "agent-pane-progress-bar--stopping": agentAtoms().turnPhaseAtom[0]().kind === "Interrupting",
                }}
                role="progressbar"
                aria-label="Agent working"
                aria-valuemin={0}
                aria-valuemax={100}
            />
            <DragOverlay message={dropAttach.dropMessage()} visible={dropAttach.isDragOver()} />
            {/* Pane title + back button now live in the block frame header,
                driven by AgentViewModel.viewName / viewIcon / endIconButtons.
                See SPEC_AGENT_PANE_FOLLOWUPS item #8. */}

            <AgentSearchBar
                visible={search.visible}
                onSearch={search.performSearch}
                onNext={search.next}
                onPrev={search.prev}
                onClose={search.close}
                matchIndex={search.currentIndex}
                matchCount={search.matchCount}
            />


            <AgentDocumentView
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
                authUrl={status.authUrl}
                authProviderId={provider()?.id ?? providerKey()}
                onSubagentClick={handleSubagentClick}
                onAgentErrorLogin={() => {
                    if (provider()?.id === "claude") {
                        log("auth", "Use existing login (inline error node) — seeding from global Claude login");
                        void status.useGlobalLogin();
                    } else {
                        log("auth", "Login Again (inline error node) — forcing a fresh provider login");
                        void status.relogin();
                    }
                }}
                onLoadOlder={history.loadOlder}
                loadingOlder={history.loadingOlder}
                scrollCommand={scroll.command}
                scrollToBottomRef={(fn) => { scrollToBottomFn = fn; }}
                highlightNodeId={search.highlightId}
                registerHistoryReadyCallback={(fn) => { historyReadyFn = fn; }}
                zoomFactor={zoomFactor}
                blockId={model.blockId}
                layoutView={layoutView}
            />

            {/* Working indicator — bottom of conversation area.
                Shows spinner + elapsed while loading, "✓ Worked · Ns" on completion.
                Acts as a visual turn delimiter; stays until next message is sent.
                See SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md §2. */}
            <Show when={
                status.isLoading()
                || workingFromPhase(agentAtoms().turnPhaseAtom[0]())
                || agentAtoms().sessionStatsAtom[0]() != null
            }>
                <AgentWorkingRow
                    loading={status.isLoading() || workingFromPhase(agentAtoms().turnPhaseAtom[0]())}
                    stopping={agentAtoms().turnPhaseAtom[0]().kind === "Interrupting"}
                    currentTool={agentAtoms().currentToolAtom[0]()}
                    currentToolArg={agentAtoms().currentToolArgAtom[0]()}
                    sessionStats={agentAtoms().sessionStatsAtom[0]()}
                    turnTokens={agentAtoms().turnTokensAtom[0]()}
                    waitingReason={
                        (() => {
                            const phase = agentAtoms().turnPhaseAtom[0]();
                            return phase.kind === "Streaming" ? (phase.waitingReason ?? null) : null;
                        })()
                    }
                    retryAfterMs={
                        (() => {
                            const phase = agentAtoms().turnPhaseAtom[0]();
                            return phase.kind === "Streaming" ? (phase.retryAfterMs ?? null) : null;
                        })()
                    }
                />
            </Show>

            <Show when={status.canRetry()}>
                <div class="agent-retry-bar">
                    <button class="agent-retry-btn" onClick={status.startLaunchFlow}>
                        Retry Login
                    </button>
                </div>
            </Show>

            {/* Permission decision panel — surfaced when one or more
                tool calls are gated by the CLI awaiting user approval.
                Sits above the queue so it can't be missed. The panel
                renders nothing when no ToolNode is in pending_approval.
                Spec: docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md §5. */}
            <AgentDecisionPanel
                pending={pendingDecisions}
                onDecide={handleDecide}
                onDefer={() => {
                    // Logging only — the panel itself manages the
                    // minimized state (per doc §7 + §4.3) so the
                    // prompt remains reachable.
                    log("agent", "Decision minimized");
                }}
            />

            {/* AskUserQuestion panel — surfaced when a tool call is in
                `awaiting_answer` (the agent asked the user a structured
                question and is blocked on the answer). Submitting delivers a
                tool_result over the persistent controller's stdin.
                Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md. */}
            <AgentQuestionPanel
                pending={pendingQuestions}
                onAnswer={handleAnswer}
                onDefer={() => log("agent", "Question minimized")}
            />

            {/* Queue sits directly below the feed so the user's newly-
                typed message lands next to the live conversation it's
                queued against. Previously lived below the activity log;
                repositioning per SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.
                "Send now" lives inside the queue header (right side)
                so it sits adjacent to the messages it accelerates. */}
            <PendingMessagesPanel
                pendingMessages={pendingMessagesAtom[0]}
                showSendNow={() =>
                    // "Send now" appears only when there is an in-flight
                    // turn that SIGINT can actually interrupt — i.e.
                    // Streaming / Interrupting. `Submitting` is excluded
                    // because the message in the queue during Submitting
                    // IS the would-be turn; there is no CLI process to
                    // stop yet. Gating on the broader `workingFromPhase`
                    // caused a brief flash on every send.
                    // Spec: docs/analysis/ANALYSIS_SEND_NOW_FLASH_2026_05_28.md.
                    isInterruptibleTurn(agentAtoms().turnPhaseAtom[0]()) &&
                    pendingMessagesAtom[0]().some((m) => m.enqueuedWhileBusy)
                }
                onSendImmediately={() => {
                    commands.stopAgent();
                }}
            />

            {/* PR F — Disconnected banner. Visible when the stream
                tore down while a turn was in flight (kind=Disconnected).
                Sits above the status line so the working spinner (which
                is already suppressed because `isWorking(Disconnected) =
                false`) doesn't overlay the disconnect message. The
                Reconnect button re-subscribes; the reducer's
                `StreamSubscribe` arm clears the phase to Idle. Spec
                docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md
                §6.4. */}
            {/* Runtime pin row — accent + title differentiate host vs container.
                agentMode defaults to "host" at launch time (agent-model.ts), so
                this row is always shown for agents launched after that default
                was introduced. The Show guard keeps legacy blocks (no agentMode
                key at all) from rendering a stale "host" row. */}
            <Show when={block()?.meta?.["agentMode"] === "host" || block()?.meta?.["agentMode"] === "container"}>
                <PaneRow
                    sigil={block()?.meta?.["agentMode"] === "container" ? "□" : "⚙"}
                    title={
                        block()?.meta?.["agentMode"] === "container"
                            ? "Container — isolated Docker sandbox"
                            : "Host — full system access"
                    }
                    accent={block()?.meta?.["agentMode"] === "container" ? "done" : "idle"}
                />
            </Show>
            {/* Failure-recovery row — per-error-class actions + auto-retry,
                rendered through the shared PaneRow accessory primitive.
                SPEC_AGENT_FAILURE_RECOVERY_UI_2026_06_16. */}
            <Show when={failureUI.row()}>
                {(row) => (
                    <PaneRow
                        sigil={row().sigil}
                        title={row().title}
                        meta={row().meta}
                        accent={row().accent}
                        actions={row().actions}
                        expanded={row().expanded}
                    >
                        <div class="agent-failure-detail">
                            <div>{row().detail}</div>
                            <Show when={row().stderrTail}>
                                <pre class="agent-failure-stderr">{row().stderrTail}</pre>
                            </Show>
                        </div>
                    </PaneRow>
                )}
            </Show>
            <AgentDisconnectedBanner
                phase={agentAtoms().turnPhaseAtom[0]}
                onReconnect={() => {
                    // Standard stream-reconnect path: dispatch
                    // `StreamSubscribe` against the live pane. If the
                    // backend has auto-reconnected between render and
                    // click, the second subscribe is harmless — the
                    // reducer's Disconnected→Idle transition is the
                    // same regardless of who calls it.
                    dispatchPane(
                        model.blockId,
                        { type: "StreamSubscribe", at: Date.now() },
                        "user",
                    );
                }}
            />

            {/* Pinned activity dock — long-running shells (and later crons /
                subagents) sit just above the composer so task status is adjacent
                to where the user's attention already is. Moved from the top per
                SPEC_ACTIVITY_DOCK_BOTTOM_MOVE_2026_06_20. */}
            <ActivityDock
                documentAtom={agentAtoms().documentAtom}
            />

            {/* Composer status strip — single 28-32px row with live
                activity ticker and Log button that toggles the log panel.
                State (detailsOpen) is reducer-owned (PR #1068). */}
            <AgentComposerStrip
                loading={
                    status.isLoading()
                    || workingFromPhase(agentAtoms().turnPhaseAtom[0]())
                }
                sessionStats={agentAtoms().sessionStatsAtom[0]()}
                turnTokens={agentAtoms().turnTokensAtom[0]()}
                processCount={processCount()}
                onProcessBadgeClick={() => {
                    createBlock({ meta: { view: "swarm" } });
                }}
                permissionMode={
                    (block()?.meta?.["agent:runtime"]?.permissionMode) as
                        import("./types").PermissionMode | undefined
                }
                logOpen={agentAtoms().detailsOpenAtom[0]()}
                onToggleLog={() =>
                    dispatchPane(model.blockId, { type: "DetailsToggle" }, "user")
                }
                contextTokens={agentAtoms().contextTokensAtom[0]()}
                contextWindow={agentAtoms().contextWindowAtom[0]() ?? provider()?.contextWindow}
                blockId={model.blockId}
                blockAtom={block}
                providerId={provider()?.id ?? ""}
            />

            <div class="agent-composer-region">
                {/* Details panel — activity log + control bar. */}
                <Show when={agentAtoms().detailsOpenAtom[0]()}>
                    <div class="agent-composer-details" id={`agent-composer-details-${model.blockId}`}>
                        <ActivityLogPanel entries={logLines} />
                        <AgentControlBar
                            blockId={model.blockId}
                            blockAtom={block}
                            providerId={provider()?.id ?? ""}
                        />
                    </div>
                </Show>
                <Show when={commands.helpVisible()}>
                    <SlashHelpPanel
                        commands={commands.availableCommands()}
                        onInvoke={(cmd) => {
                            commands.closeHelp();
                            void commands.sendMessage(`/${cmd.name}`);
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
                        scrollToBottomFn?.();
                    }}
                    onStopAgent={commands.stopAgent}
                    onRecallLatestQueued={commands.recallLatestHeld}
                    getCompletions={commands.completions}
                    viewModel={model}
                />
            </div>
            {/* AgentActionBar (Add / Import / Export) lives in the
                AgentPicker view only. Once an agent is loaded the user
                is working in the conversation; the action bar would
                just take up vertical space. */}
            <Show when={closeConfirm()}>
                {(info) => (
                    <ConfirmModal
                        open={true}
                        title="Close pane?"
                        description={
                            `This agent has ${info().count} ${
                                info().count === 1 ? "process" : "processes"
                            } still running. Close and kill them all?`
                        }
                        confirmLabel="Close and kill"
                        destructive
                        onConfirm={handleCloseConfirmAccept}
                        onCancel={() => setCloseConfirm(null)}
                    />
                )}
            </Show>
        </div>
    );
};

AgentPresentationView.displayName = "AgentPresentationView";
