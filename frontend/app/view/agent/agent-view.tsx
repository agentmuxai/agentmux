// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { getProvider } from "./providers";
import { createAgentAtoms } from "./state";
import {
    dispatch as dispatchDoc,
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
import { useSessionDigest } from "./hooks/useSessionDigest";
import { useHistoryPagination, SNAPSHOT_SCHEMA_VERSION } from "./hooks/useHistoryPagination";
// SNAPSHOT_SCHEMA_VERSION re-exported from useHistoryPagination; imported here for the write path.
import { useAgentControllerStatus } from "./hooks/useAgentControllerStatus";
import { useInSessionSearch } from "./hooks/useInSessionSearch";
import { useScrollToNode } from "./hooks/useScrollToNode";
import { useAgentKeyboard } from "./hooks/useAgentKeyboard";
import { useProcessCount } from "./hooks/useProcessCount";
import { usePtyWidth, computeTermSizeFromEl } from "./hooks/usePtyWidth";
import { useSubagentEvents } from "./hooks/useSubagentEvents";
import { useControllerStatusEvents } from "./hooks/useControllerStatusEvents";
import { useAgentCommands } from "./hooks/useAgentCommands";
import { useAgentFailure } from "./hooks/useAgentFailure";
import { PaneRow } from "./components/PaneRow";
import { openBundleManager } from "@/app/modals/bundle-manager-modal";
import { useAgentDropAttach } from "./hooks/useAgentDropAttach";
import { DragOverlay } from "@/app/element/dragoverlay";
import { AgentControlBar } from "./components/AgentControlBar";
import { ActivityDock } from "./components/ActivityDock";
import { ActivityLogPanel } from "./components/ActivityLogPanel";
import { AgentDecisionPanel } from "./components/AgentDecisionPanel";
import { AgentQuestionPanel, type AnswerOutcome } from "./components/AgentQuestionPanel";
import { AgentDisconnectedBanner } from "./components/AgentDisconnectedBanner";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter, AgentWorkingRow, AgentAuxInfoBar } from "./components/AgentFooter";
import { AgentComposerStrip } from "./components/AgentComposerStrip";
import { PendingMessagesPanel } from "./components/PendingMessagesPanel";
import { AgentPicker, useAgentDefinitions } from "./components/AgentPicker";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { AgentFocusedPanel } from "./components/AgentFocusedPanel";
import { SlashCommandPicker } from "./components/SlashCommandPicker";
import { SlashHelpPanel } from "./components/SlashHelpPanel";
import { SessionDigestBanner } from "./components/SessionDigestBanner";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createBlock, getApi, WOS } from "@/app/store/global";
import { ConfirmModal } from "@/element/modal";
import { ModalLayer } from "@/element/ModalLayer";
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
 * inert the whole tab instead of just this pane (codex P1 on PR
 * #1034). Wrapping at the wrapper covers both the pre-launch picker
 * AND the post-launch presentation view, so the pane-scope lock
 * holds across the entire pane lifecycle.
 * SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md.
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

// ── Presentation View ───────────────────────────────────────────────────────────

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

    // Overlay tab signal — lives in the component so SolidJS can track it.
    // The model's _setOverlayTab callback is wired on mount and cleaned up on unmount.
    const [showOverlayTab, setShowOverlayTab] = createSignal<import("./agent-model").OverlayTab | null>(null);
    onMount(() => {
        model._setOverlayTab = setShowOverlayTab;
    });
    onCleanup(() => {
        model._setOverlayTab = null;
    });

    // Reactive agent-definition list — used to resolve the current AgentDefinition object
    // so the overlay can pass it to AgentCardSettingsPanel / rename input.
    const agentDefinitions = useAgentDefinitions();
    const currentAgent = createMemo(() => agentDefinitions().find((a) => a.id === agentId));

    const agentAtoms = createMemo(() => createAgentAtoms(model.blockId));

    // Register this pane with BOTH the document store and the pane-state
    // store SYNCHRONOUSLY in one atomic call, during component-body
    // execution — before any hook's onMount can dispatch
    // (codex P1 PR #681 round 1). The stores throw on dispatch to an
    // unregistered slot to prevent silent reducer-command drops, so both
    // slots must exist before useAgentStream / useHistoryPagination call
    // their first dispatch from `onMount` handlers.
    //
    // PR-3 of the cascade follow-up sequence: registration is unified so
    // a dispatcher can never observe the pane registered in one store
    // but not the other. The half-registered window was the structural
    // root of the cascade-mid-dispatch failure mode PR #878 detected and
    // PR #989 migrated three call sites away from
    // (docs/analysis/LIFECYCLE_DISPATCH_LEAK_2026_05_15.md + the
    // 2026-05-23 replaceChild retro). See agent-pane-registration.ts.
    //
    // `turnPhase` is the single-source-of-truth working/stopping signal
    // since PR G — the legacy `turnActive` / `stopping` /
    // `streaming.active` fields and their projection setters were
    // removed. The view binds the working animation to
    // `workingFromPhase(turnPhase)` and the "Stopping…" label to
    // `turnPhase.kind === "Interrupting"` (see AgentStatusLine below).
    // Capture the model returned by registerPane (PR-4 of the cascade
    // follow-up) — passed into hooks/views so their dispatch callsites
    // are default-safe against post-unmount races. The disposed flag on
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
                pending: a.pendingMessagesAtom[1],
                initPhase: a.initPhaseAtom[1],
                turnPhase: a.turnPhaseAtom[1],
                detailsOpen: a.detailsOpenAtom[1],
                composerUnreadCount: a.composerUnreadCountAtom[1],
            },
        });
        registerAgentActivity(model.blockId, a.turnPhaseAtom[0]);
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

    // Cross-slice projection: dispatch `LogEntryArrived` to the pane
    // reducer on each new log line, so it can increment
    // `composerUnreadCount` while the composer details panel is closed.
    // Tracked via prev-length to detect strict growth (`clear()` shrinks
    // the array; we don't want to count that). Reducer-side gating means
    // entries arriving while the panel is open are no-ops, so this is
    // safe to dispatch unconditionally on growth.
    // SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md §5.4.
    createEffect((prevLen: number) => {
        const cur = logLines().length;
        if (cur > prevLen) {
            const grown = cur - prevLen;
            for (let i = 0; i < grown; i++) {
                dispatchPaneIfRegistered(model.blockId, { type: "LogEntryArrived" }, "system");
            }
        }
        return cur;
    }, 0);

    // Startup sequence callback ref — assigned after commands + handleSendMessage
    // are defined (below), so the onReady callback can reference them.
    // onReady fires synchronously after startLaunchFlow succeeds, which is
    // always after this component body has fully run (SolidJS onMount timing).
    let onReadyFn: (() => void) | null = null;

    // History pagination: dispatches HistoryLoaded into the agent-document-store
    // for the trailing 200 lines on mount + each user-triggered loadOlder.
    //
    // Option E (PR #1007 backend, this PR frontend): pass the agent
    // definition id so the snapshot fast-path reads from the
    // agent-anchored zone (`agent:<defId>:current`) rather than the
    // per-block zone. `agentId` here is the AgentDefinition slug/UUID —
    // a non-empty string is guaranteed at this point by the `Show
    // when={agentId()}` gate in AgentViewWrapper above.
    // Bridged callback: AgentDocumentView registers viewState.markHistoryReady
    // here on mount (before any async history work starts), so
    // useHistoryPagination can signal "history done" into the viewState
    // that lives inside AgentDocumentView. This drives the enter-animation
    // gate on the streaming buffer. See PR #1212.
    let historyReadyFn: (() => void) | undefined;
    const history = useHistoryPagination({
        blockId: model.blockId,
        // PR-4 — see useAgentStream above; same rationale.
        model: paneModel,
        outputFormat,
        definitionId: agentId,
        // Option E: snapshot read returns the modts of the previous
        // owner's last write; project into the model so `viewText`
        // renders a "· continued Xm ago" title-bar chip when the gap
        // is >30s. Setter is a no-op if the pane unmounts before
        // restore.
        onContinuationModts: (ms) => model.continuedFromMsAtom._set(ms),
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

    // Session digest banner state + auto-trigger.
    const digest = useSessionDigest({ blockId: model.blockId, block, log });

    // Auth + launch flow state and the onCleanup that kills the CLI
    // if the pane closes mid-login.
    // `getDocument` is read-only; for writes we MUST dispatch through
    // agent-document-store so slot.state stays in sync (codex P1 PR #681).
    const [getDocument] = agentAtoms().documentAtom;

    // Agent-pane state-persistence (RFC #857 + spec
    // SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md):
    // (a) on pane close, write a snapshot of `nodes[]` so the next reopen
    //     restores the full conversation via HistoryRestored rather than
    //     the lossy 200-line NDJSON replay;
    // (b) during the pane lifetime, write a snapshot every 30s if the
    //     document changed since the last save. Bounds crash-loss to ~30s.
    // Serialize concurrent writes through a single promise chain
    // (codex P2 on #877 round 5): the 30s interval and the on-close
    // cleanup can both call writeSnapshotNow() with their own captured
    // `nodes` snapshot, then race through an async line-count RPC and
    // write. Without ordering, the older interval write can resolve
    // LAST and overwrite the close-time snapshot, losing recent nodes.
    let inFlightSnapshot: Promise<void> = Promise.resolve();
    const writeSnapshotNow = () => {
        // Don't let a cross-block continuation pane (one that mounted against a
        // snapshot whose sourceBlockId names another block) overwrite the
        // agent-anchored snapshot. It holds no durable history of its own, so a
        // write would repoint the agent's snapshot at this near-empty block and
        // make the original block's conversation unrestorable. See spec §15 / #1397.
        if (history.snapshotIsForeignBlock()) {
            return;
        }
        // Schema v2: capture the lightweight overlay state (DocumentState +
        // pane flags) synchronously before the async RPC chain so we snapshot
        // the values at trigger time, not after a potential 3 s round-trip.
        // nodes[] is NOT included — the NDJSON output log is the source of
        // truth and is replayed on restore. This keeps the payload under 1 KB
        // regardless of conversation length, eliminating the renderer OOM.
        // See docs/specs/SPEC_WRITE_STATE_NDJSON_RESTORE_2026_06_12.md.
        const [docState] = agentAtoms().documentStateAtom;
        const [detailsOpen] = agentAtoms().detailsOpenAtom;
        const capturedDocState = docState();
        const capturedDetailsOpen = detailsOpen();

        inFlightSnapshot = inFlightSnapshot.then(async () => {
            let highWaterMark = 0;
            try {
                const countResp = await RpcApi.BlockfileLineCountCommand(TabRpcClient, {
                    block_id: model.blockId,
                    filename: "output",
                }, { timeout: 3000 });
                highWaterMark = countResp?.count ?? 0;
            } catch {
                // Soft fail — snapshot still ships without the mark.
            }
            // Note: no historyOffset field — v2 restore derives the render window
            // from highWaterMark (windowStart = hwm - RESTORE_WINDOW_LINES), so a
            // persisted offset would be dead/misleading.
            //
            // sourceBlockId records which block's per-block NDJSON `output` the
            // highWaterMark counted. The snapshot itself is agent-anchored
            // (definition_id zone) and survives across blocks, but the NDJSON it
            // references is per-block — so restore must read history from this
            // block, not from a fresh continuation pane's empty block.
            const snapshot = {
                schemaVersion: SNAPSHOT_SCHEMA_VERSION,
                savedAt: new Date().toISOString(),
                highWaterMark,
                sourceBlockId: model.blockId,
                documentState: {
                    collapsedNodeIds: capturedDocState ? [...capturedDocState.collapsedNodes] : [],
                    pinnedNodeIds: capturedDocState ? [...capturedDocState.pinnedNodes] : [],
                    scrollPosition: capturedDocState?.scrollPosition ?? 0,
                    filter: capturedDocState?.filter ?? {
                        showThinking: false,
                        showSuccessfulTools: true,
                        showFailedTools: true,
                        showIncoming: true,
                        showOutgoing: true,
                    },
                },
                paneState: {
                    detailsOpen: capturedDetailsOpen ?? false,
                },
            };
            await RpcApi.AgentSessionWriteStateCommand(TabRpcClient, {
                definition_id: agentId,
                content: JSON.stringify(snapshot),
            }, { timeout: 10000 });
        }).catch((e) => {
            log("history", `snapshot write failed: ${e?.message ?? e}`, "warn");
        });
    };
    // Dirty-flag interval: avoids resetting a debounce timer on every
    // token chunk during streaming (would block all crash-time saves)
    // and avoids dispatching a save on every reactive change. A change
    // sets `dirty`; the 30s tick flushes if dirty and resets.
    let dirty = false;
    let lastNodes = getDocument();
    createEffect(() => {
        const next = getDocument();
        if (next !== lastNodes) {
            dirty = true;
            lastNodes = next;
        }
    });
    const SNAPSHOT_INTERVAL_MS = 30_000;
    const snapshotInterval = setInterval(() => {
        if (!dirty) return;
        dirty = false;
        writeSnapshotNow();
    }, SNAPSHOT_INTERVAL_MS);
    onCleanup(() => clearInterval(snapshotInterval));
    onCleanup(() => {
        if (!dirty) return;
        writeSnapshotNow();
    });

    // Pending decision queue — every ToolNode whose
    // `status === "pending_approval"`, oldest first. The decision
    // panel renders the head; Allow / Deny clears the node by
    // transitioning its status. Defer is HANDLED INSIDE THE PANEL
    // (it minimizes locally) — per
    // docs/specs/SPEC_DECISION_PROMPT_DESIGN_2026_04_25.md §7,
    // the parent must NOT filter pending. The actual
    // `tool:decision` IPC + sidecar stdin write lands in PR-3.
    const pendingDecisions = (): import("./types").ToolNode[] => {
        const docs = getDocument();
        const out: import("./types").ToolNode[] = [];
        for (const n of docs) {
            if (n.type === "tool" && n.status === "pending_approval") out.push(n);
        }
        return out;
    };

    const handleDecide = (decision: import("./components/AgentDecisionPanel").DecisionOutcome) => {
        // Optimistic UI update — flip the ToolNode out of
        // pending_approval immediately so the panel disappears (or
        // advances to the next pending request). The backend write
        // happens in parallel; if it fails we log but don't try to
        // roll back the visual transition.
        // Dispatch through the reducer (StreamFlush.updatedNodes) so
        // slot.state stays in sync. Find the matching pending tool node
        // by request_id, then build the updated node.
        const updated: import("./types").ToolNode[] = [];
        for (const n of getDocument()) {
            if (n.type !== "tool" || n.status !== "pending_approval") continue;
            if (n.pendingPermission?.request_id !== decision.request_id) continue;
            updated.push({
                ...n,
                status: decision.outcome === "allow" ? "running" : "denied",
                pendingPermission: undefined,
            });
        }
        if (updated.length > 0) {
            dispatchDoc(
                model.blockId,
                { type: "StreamFlush", newNodes: [], updatedNodes: updated },
                "user",
            );
        }
        // Send the decision to the sidecar so it can record + audit
        // it (PR-3a). Actual delivery to the agent CLI — rules
        // persistence (path 1) or interactive subprocess stdin (path
        // 2) — is deferred to PR-3b/PR-4 per
        // SPEC_DECISION_PROMPT_2026_04_24.md §9.1.
        void RpcApi.ToolDecisionCommand(TabRpcClient, {
            blockid: model.blockId,
            request_id: decision.request_id,
            outcome: decision.outcome,
            scope: decision.scope,
            feedback: decision.feedback,
        }).catch((err: unknown) => {
            log("error", `tool:decision failed: ${String(err)}`);
        });
    };

    // Pending AskUserQuestion queue — every ToolNode in `awaiting_answer`,
    // oldest first. The question panel renders the head; Submit transitions
    // the node and delivers the answer to the agent CLI as a tool_result.
    // Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md.
    const pendingQuestions = (): import("./types").ToolNode[] => {
        const docs = getDocument();
        const out: import("./types").ToolNode[] = [];
        for (const n of docs) {
            if (n.type === "tool" && n.status === "awaiting_answer" && n.question) out.push(n);
        }
        return out;
    };

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
    // a ConfirmModal. Accept → `agent.kill-tree` RPC then proceed with
    // close. Cancel → abort, pane stays open. Zero tracked processes
    // → original close path, no prompt.
    //
    // We wrap `nodeModel.onClose` in place rather than adding a new
    // ViewModel hook — ViewModel has no `beforeClose` / `canClose`
    // surface today, and threading one through would touch the layout
    // + block-frame + pane-actions tree. A local wrapper is enough for
    // v1 of this feature.
    const [closeConfirm, setCloseConfirm] = createSignal<{
        count: number;
        originalClose: () => void;
    } | null>(null);

    onMount(() => {
        const original = model.nodeModel.onClose;
        const wrapped = () => {
            const count = processCount();
            if (count <= 0) {
                original?.();
                return;
            }
            // Stash the original close so the modal can invoke it on
            // confirm. Not calling original() here keeps the pane open
            // until the user decides.
            setCloseConfirm({ count, originalClose: () => original?.() });
        };
        model.nodeModel.onClose = wrapped;
        onCleanup(() => {
            // Only restore if we're still the wrapper — avoids
            // clobbering a later wrapper set by someone else.
            if (model.nodeModel.onClose === wrapped) {
                model.nodeModel.onClose = original;
            }
        });
    });

    const handleCloseConfirmAccept = async () => {
        const info = closeConfirm();
        if (!info) return;
        try {
            // Kill first, then proceed with layout close. The tracker's
            // Drop impl in `delete_controller` will nuke what survived
            // if the RPC errors — we've already committed to closing.
            await RpcApi.AgentKillTreeCommand(TabRpcClient, {
                block_id: model.blockId,
            });
        } catch {
            // swallow — close proceeds regardless
        } finally {
            setCloseConfirm(null);
            info.originalClose();
        }
    };

    // Subscribe to subprocess output and parse into DocumentNodes.
    // Mutations dispatch through agent-document-store; the reducer there
    // owns dedup against in-flight history loads and the truncate-suppress
    // invariant that prevents the mid-session wipe bug.
    const pendingMessagesAtom = agentAtoms().pendingMessagesAtom;
    useAgentStream({
        blockId: model.blockId,
        // PR-4 — pass the per-pane model so the hook's dispatch sites
        // are default-safe against post-unmount races. Without the
        // model, the hook had to import / remember the soft variant
        // (`dispatchIfRegistered`) per call site; with the model the
        // disposed-flag check is centralized.
        model: paneModel,
        outputFormat: outputFormat(),
        documentAtom: agentAtoms().documentAtom,
        // turnPhase is the SoT for "is a stop in flight" — replaces the
        // legacy `stoppingAtom` dropped in PR G. useAgentStream needs
        // it to detect user-initiated stops and append the
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
        // PR-4 — see useAgentStream above; same rationale.
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
        // Capture working state BEFORE TurnStart so PendingMessageQueued can
        // mark whether this message is queued behind a running turn (true) or
        // is the message that initiated the turn (false). The panel only shows
        // messages with enqueuedWhileBusy:true, preventing the idle-send race
        // where the message flashed in the amber zone between Streaming
        // promotion and agent-message-accepted. See ANALYSIS_IDLE_SEND_RACE_2026_06_11.md.
        const wasAlreadyWorking = workingFromPhase(paneSnapshot(model.blockId)?.turnPhase ?? { kind: "Idle" });
        dispatchPane(model.blockId, { type: "TurnStart", at: Date.now() }, "user");
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
        onTrustCenter: () => openBundleManager(),
        // context_exceeded recovery — drop the over-full session and return to
        // the picker for a clean relaunch (resuming would only re-fail).
        onNewSession: () => {
            log("agent", "New session — clearing the over-full context and returning to the picker");
            void model.backToPicker();
        },
        // P2 — real re-auth. An auth failure is a *CLI-provider* login lapse
        // (e.g. claude's subscription OAuth expired), not a Trust Center
        // service account. `startLaunchFlow` is the canonical path that
        // re-runs that login (surfacing the OAuth URL via the auth panel) and
        // relaunches the agent; the relaunched process then reports
        // `controllerstatus: running`, which clears this failure row. This is
        // the same flow the legacy AgentRetryBar's button drives.
        onLoginAgain: () => {
            log("auth", "Login Again — re-running the provider login flow");
            void status.startLaunchFlow();
        },
    });

    // AskUserQuestion answer handler. Defined after handleSendMessage because
    // the non-persistent fallback below delegates to it.
    const handleAnswer = (outcome: AnswerOutcome) => {
        // Optimistic transition: flip the node out of awaiting_answer so the
        // panel dismisses immediately. We snapshot the original node(s) so a
        // failed delivery can roll the transition back.
        const originals: import("./types").ToolNode[] = [];
        const updated: import("./types").ToolNode[] = [];
        for (const n of getDocument()) {
            if (n.type !== "tool" || n.status !== "awaiting_answer") continue;
            if (n.question?.tool_use_id !== outcome.tool_use_id) continue;
            originals.push(n);
            updated.push({
                ...n,
                status: "success",
                question: undefined,
                summary: `❓ Answered — ${outcome.answer_text.replace(/\n/g, "; ")}`,
            });
        }
        const applyDoc = (nodes: import("./types").ToolNode[]) => {
            if (nodes.length > 0) {
                dispatchDoc(model.blockId, { type: "StreamFlush", newNodes: [], updatedNodes: nodes }, "user");
            }
        };
        applyDoc(updated);

        // Phase 1 path: persistent (host) agents speak the control protocol, so
        // the answer is delivered as a control_response (updatedInput.answers)
        // that resumes the turn the CLI parked on the can_use_tool request.
        void RpcApi.AgentAnswerCommand(TabRpcClient, {
            blockid: model.blockId,
            tool_use_id: outcome.tool_use_id,
            answers: outcome.answers_map,
        }).catch((err: unknown) => {
            const msg = String(err);
            // Phase 2 path: one-shot / container agents have no live stdin, and
            // the CLI abandons the AskUserQuestion tool_use when the turn ends —
            // a tool_result can no longer reach it (validated empirically:
            // SPEC §10.1). Deliver the answer as a normal follow-up turn
            // instead; the agent resumes the session and continues from the
            // question with the answer as context. Keep the optimistic success.
            if (msg.includes("UNSUPPORTED_CONTROLLER")) {
                log("agent", "Delivering AskUserQuestion answer as a follow-up message (non-persistent agent)");
                void handleSendMessage(outcome.answer_text).catch((e: unknown) => {
                    log("error", `answer follow-up failed: ${String(e)}`);
                    applyDoc(originals);
                });
                return;
            }
            // Any other failure: roll the node back so the panel re-surfaces
            // rather than falsely showing "answered" while the agent is blocked.
            log("error", `agent.answer failed: ${msg}`);
            applyDoc(originals);
        });
    };

    // ── Startup sequence ────────────────────────────────────────────────────────
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

    // ── Jump-to-node ───────────────────────────────────────────────────────────

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
            style={{ zoom: zoomFactor() }}
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

            <SessionDigestBanner
                accessory={digest.accessory}
                onDismiss={digest.dismiss}
                onRegenerate={() => digest.fetch(true)}
            />

            {/* Title-bar action overlay: ⚙ Agent / 👤 Identity.
                Gated only on the tab being open — NOT on currentAgent()
                resolving. The pane's `agentId` is a db_agent_definitions
                id only for definition-launched panes; provider quick-launch
                writes a provider id, and the definition list may also be
                mid-load or have failed to fetch. Requiring currentAgent()
                here made the gear silently no-op in all those cases. The
                panel handles an undefined agent (create-mode + the Identity
                tab's "save first" fallback). */}
            <Show when={showOverlayTab() != null}>
                <AgentFocusedPanel
                    blockId={model.blockId}
                    nodeModel={model.nodeModel}
                    agent={currentAgent()}
                    initialTab={showOverlayTab()!}
                    onClose={() => setShowOverlayTab(null)}
                    onTabChange={(tab) => { model._lastOverlayTab = tab; }}
                />
            </Show>

            {/* Pinned activity dock — long-running shells (and later crons /
                subagents) stay glanceable at the top while the conversation
                scrolls under it. SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15. */}
            <ActivityDock
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
            />

            <AgentDocumentView
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
                authUrl={status.authUrl}
                authProviderId={provider()?.id ?? providerKey()}
                onSubagentClick={handleSubagentClick}
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
                    sessionStats={agentAtoms().sessionStatsAtom[0]()}
                    turnTokens={agentAtoms().turnTokensAtom[0]()}
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
                v1 PR-2 wires the UI; PR-3 adds the IPC + sidecar stdin
                write so decisions actually reach the subprocess.
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

            {/* Slim composer status strip — single 28-32px row replacing
                the prior AgentStatusLine. Surfaces the latest activity-
                log entry as a live ticker on the left, tokens/elapsed/
                process-count on the right, and a chevron with unread
                badge that toggles the details panel. State is reducer-
                owned (`AgentPaneState.detailsOpen` /
                `composerUnreadCount`, added in #1068).
                SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md. */}
            <AgentComposerStrip
                detailsPanelId={`agent-composer-details-${model.blockId}`}
                loading={
                    status.isLoading()
                    || workingFromPhase(agentAtoms().turnPhaseAtom[0]())
                }
                stopping={agentAtoms().turnPhaseAtom[0]().kind === "Interrupting"}
                currentTool={agentAtoms().currentToolAtom[0]()}
                sessionStats={agentAtoms().sessionStatsAtom[0]()}
                turnTokens={agentAtoms().turnTokensAtom[0]()}
                processCount={processCount()}
                onProcessBadgeClick={() => {
                    createBlock({ meta: { view: "swarm" } });
                }}
                latestLogLine={logLines()[logLines().length - 1]?.text}
                permissionMode={
                    (block()?.meta?.["agent:runtime"]?.permissionMode) as
                        import("./types").PermissionMode | undefined
                }
                expanded={agentAtoms().detailsOpenAtom[0]()}
                unreadCount={agentAtoms().composerUnreadCountAtom[0]()}
                onToggleExpanded={() =>
                    dispatchPane(model.blockId, { type: "DetailsToggle" }, "user")
                }
                contextTokens={agentAtoms().contextTokensAtom[0]()}
                contextWindow={provider()?.contextWindow}
            />

            <div class="agent-composer-region">
                {/* Details panel — when the user expands the strip, show
                    the activity log + control bar inside this section.
                    Phase 1 of the redesign keeps the activity log and
                    control bar mostly intact (just gated on
                    `detailsOpen`); a follow-up will consolidate them
                    into a single AgentComposerDetails component.
                    SPEC_AGENT_COMPOSER_SLIM_STATUS_2026_05_26.md §4. */}
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
                    onTyping={() => scrollToBottomFn?.()}
                    onStopAgent={commands.stopAgent}
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
