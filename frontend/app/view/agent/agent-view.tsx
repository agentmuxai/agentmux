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
import { isAuthFailure, workingFromPhase } from "@/app/store/agent-pane-state/types";
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
    getSettingsKeyAtom,
    openOrFocusPaneByView,
    MOS,
} from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { BlockService } from "@/app/store/services";
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
    batch,
    createEffect,
    createMemo,
    createSignal,
    on,
    onCleanup,
    onMount,
    Show,
    untrack,
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
import { ActivityDock } from "./components/ActivityDock";
import { AgentComposerStrip } from "./components/AgentComposerStrip";
import { AgentSessionNotices } from "./components/AgentSessionNotices";
import { AgentShellDrawer } from "./components/AgentShellDrawer";
import { AgentCredentialsRevokedChip } from "./components/AgentCredentialsRevokedChip";
import { ShutdownPendingBanner } from "./shutdown/ShutdownPendingBanner";
import { AgentDecisionPanel } from "./components/AgentDecisionPanel";
import { AgentDisconnectedBanner } from "./components/AgentDisconnectedBanner";
import { AgentAuthPanel, AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter, AgentWorkingRow } from "./components/AgentFooter";
import { AgentQuestionPanel } from "./components/AgentQuestionPanel";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { collapseDrawerOnShellExit } from "./shell-exit-collapse";
import { ForkProviderFallbackBanner } from "./components/ForkProviderFallbackBanner";
import { PaneRow } from "./components/PaneRow";
import { PendingMessagesPanel } from "./components/PendingMessagesPanel";
import { AgentStashDrawer } from "./components/AgentStashDrawer";
import { BtwOverlay } from "./components/BtwOverlay";
import { SlashCommandPicker } from "./components/SlashCommandPicker";
import { SlashHelpPanel } from "./components/SlashHelpPanel";
import { usePaneReveal } from "./hooks/usePaneReveal";
import { useShellLogBridge } from "./hooks/useShellLogBridge";
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
import { didTurnJustEnd, useControllerStatusEvents } from "./hooks/useControllerStatusEvents";
import { useHistoryPagination } from "./hooks/useHistoryPagination";
import { createTranscriptSettleLatch } from "./transcript-cursor";
import { useInSessionSearch } from "./hooks/useInSessionSearch";
import { useNextPromptSuggestion } from "./hooks/useNextPromptSuggestion";
import { computeTermSizeFromEl, usePtyWidth } from "./hooks/usePtyWidth";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { useScrollToNode } from "./hooks/useScrollToNode";
import { useSnapshotPersistence } from "./hooks/useSnapshotPersistence";
import { injectGapRows, injectHistoryLink } from "./inject-history-link";
import { liveFeedSupported, resolveLiveFeedTurns, visibleIdsOf } from "./live-feed";
import { userIsInteracting } from "./stream-scheduler";
import { buildResumePreflightNode, injectResumePreflight } from "./inject-resume-preflight";
import { useResumePreflight } from "./hooks/useResumePreflight";
import { openOrFocusHistoryTab } from "./open-history-tab";
import { getProvider } from "./providers";
import { sendStartupSequence } from "./startup/sendStartupSequence";
import { createAgentAtoms } from "./state";
import type { DocumentNode } from "./types";
import { ShutdownOverlay } from "./shutdown/ShutdownOverlay";
import { useAgentStream } from "./useAgentStream";

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
            void RpcApi.SetMetaCommand(TabRpcClient, {
                oref: MOS.makeORef("block", model.blockId),
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
        // What the live feed keeps: its K finished turns plus the one in
        // flight. Only Claude's transcript is split into turns on the
        // backend. Read at restore time — after this component's body, so
        // the live-feed consts declared below are set.
        restoreTurns: () =>
            liveFeedOn() && outputFormat() === "claude-stream-json" ? liveFeedTurns + 1 : undefined,
        onHistoryReady: () => {
            historyReadyFn?.();
            // A pane opens with K turns, not the load window's worth (§6.9).
            scheduleRollOff();
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

    // True when content older than the working session exists out of view:
    // set by the restore/pagination clamp paths (scopeClamped) OR derived
    // from a live clamp — after the reducer's StreamFlush trim, the fresh
    // session_outcome divider is always the first document node.
    // ---- The live feed (SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.9) ----
    // The pane keeps the turn in flight plus the last K finished turns;
    // older ones roll off into History, which follows the transcript. Read
    // once at mount, like agent:turnscopedtail. Providers whose transcript
    // lacks the user's messages keep today's behaviour (`liveFeedSupported`).
    const liveFeedSetting = untrack(() => getSettingsKeyAtom("agent:livefeed")()) !== false;
    const liveFeedTurns = resolveLiveFeedTurns(untrack(() => getSettingsKeyAtom("agent:livefeedturns")()));
    const liveFeedOn = (): boolean =>
        liveFeedSetting && liveFeedSupported(outputFormat(), block()?.meta?.["controller"] as string | undefined);
    // Whether the reader follows the bottom — handed over by the document view.
    let followingBottom: Accessor<boolean> = () => true;
    // Turns rolled off the front since mount, and the gap rows between kept turns.
    const [rolledOffTurns, setRolledOffTurns] = createSignal(0);
    const [gapsBefore, setGapsBefore] = createSignal<ReadonlySet<string>>(new Set());
    let rollOffDisposed = false;
    onCleanup(() => {
        rollOffDisposed = true;
    });

    /** One roll-off pass: a single reducer command, planned on its current nodes. */
    const runRollOff = (): void => {
        if (!liveFeedOn() || rollOffDisposed) return;
        const [docState, setDocState] = agentAtoms().documentStateAtom;
        const pinnedIds = untrack(docState).pinnedNodes;
        const events = paneModel.dispatchDoc({
            type: "RollOff",
            keepTurns: liveFeedTurns,
            visibleIds: visibleIdsOf(layoutSnapshot(model.blockId)),
            keepIds: pinnedIds,
            pinned: untrack(followingBottom),
        });
        const ev = events.find((e) => e.type === "turns-rolled-off");
        if (!ev || ev.type !== "turns-rolled-off") return;
        const present = new Set(untrack(paneModel.document).map((n) => n.id));
        const prune = (set: Set<string>): Set<string> => {
            let changed = false;
            const next = new Set<string>();
            for (const id of set) {
                if (present.has(id)) next.add(id);
                else changed = true;
            }
            return changed ? next : set;
        };
        batch(() => {
            setRolledOffTurns((n) => n + ev.prefixTurns);
            setGapsBefore((prev) => {
                const next = new Set<string>();
                for (const id of prev) if (present.has(id)) next.add(id);
                for (const id of ev.gapsBefore) next.add(id);
                return next;
            });
            // The view's own id sets must not keep ids that are gone.
            setDocState((prev) => {
                const collapsedNodes = prune(prev.collapsedNodes);
                const expandedTools = prune(prev.expandedTools);
                const pinnedNodes = prune(prev.pinnedNodes);
                return collapsedNodes === prev.collapsedNodes &&
                    expandedTools === prev.expandedTools &&
                    pinnedNodes === prev.pinnedNodes
                    ? prev
                    : { ...prev, collapsedNodes, expandedTools, pinnedNodes };
            });
        });
        if (ev.blockedTurns > 0) {
            console.debug(`[live-feed] ${model.blockId}: ${ev.blockedTurns} older turn(s) kept (not in the transcript)`);
        }
    };

    /**
     * Schedule a pass off the input path: when the browser is idle, stepping
     * aside while the user types, but never later than ROLL_OFF_DEADLINE_MS —
     * a deferred pass is re-queued, not dropped (§6.9).
     */
    const ROLL_OFF_DEADLINE_MS = 1_000;
    let rollOffQueued = false;
    const whenIdle = (cb: () => void, timeoutMs: number): void => {
        const ric = (globalThis as { requestIdleCallback?: (cb: () => void, o: { timeout: number }) => number })
            .requestIdleCallback;
        if (ric) ric(cb, { timeout: Math.max(1, timeoutMs) });
        else setTimeout(cb, Math.min(50, Math.max(0, timeoutMs)));
    };
    function scheduleRollOff(): void {
        if (!liveFeedOn() || rollOffQueued) return;
        rollOffQueued = true;
        const deadline = performance.now() + ROLL_OFF_DEADLINE_MS;
        const attempt = (): void => {
            if (rollOffDisposed) return;
            const left = deadline - performance.now();
            if (left > 0 && userIsInteracting()) return whenIdle(attempt, left);
            rollOffQueued = false;
            runRollOff();
        };
        whenIdle(attempt, ROLL_OFF_DEADLINE_MS);
    }

    // Backstop for paths that add many turns at once without a turn end or a
    // send (a restore, a large history load): a pass whenever the feed first
    // holds clearly more turns than it keeps. A memo, so it fires on the
    // transition, not on every flush.
    const feedOverBudget = createMemo(() => {
        if (!liveFeedOn()) return false;
        let turns = 0;
        for (const n of paneModel.document()) if (n.type === "user_message") turns++;
        return turns > liveFeedTurns + 3;
    });
    createEffect(
        on(feedOverBudget, (over) => {
            if (over) scheduleRollOff();
        }),
    );

    // Roll-off points besides the history load and turn end (below): the next
    // send, and the pane going out of view.
    createEffect(
        on(
            () => {
                const doc = paneModel.document();
                const last = doc[doc.length - 1];
                return last?.type === "user_message" ? last.id : null;
            },
            (id) => {
                if (id) scheduleRollOff();
            },
            { defer: true },
        ),
    );
    createEffect(
        on(
            hidden,
            (hidden) => {
                if (hidden) scheduleRollOff();
            },
            { defer: true },
        ),
    );

    const earlierHistoryAvailable = createMemo(() => {
        if (history.scopeClamped()) return true;
        if (liveFeedOn() && (rolledOffTurns() > 0 || history.historyOffset() > 0)) return true;
        const first = paneModel.document()[0];
        return first?.type === "session_outcome" && first.outcome === "fresh";
    });
    // "N earlier turns" only when N is the whole story: everything before the
    // feed was loaded from line 0 and rolled off here.
    const earlierTurnsKnown = (): number | undefined =>
        liveFeedOn() && rolledOffTurns() > 0 && history.historyOffset() === 0 && !history.scopeClamped()
            ? rolledOffTurns()
            : undefined;

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

    // Bumped exactly once per genuine, backend-confirmed turn completion —
    // the `turn_active: true -> false` edge, fed ONLY by live controllerstatus
    // events (see trackTurnJustEnded below, and NOT reconcileTurnActive — the
    // mount-time one-shot deliberately does not participate; reagent P1 on
    // PR #2241). This is the trigger useAgentActivitySummary/
    // useNextPromptSuggestion use instead of TurnPhase.kind === "Done" (which
    // over-triggers — see
    // docs/specs/REPORT_AMBIENT_SUMMARY_OVERTRIGGER_2026_07_20.md).
    // `wasTurnActive` is plain (non-reactive) — it only exists to detect the
    // edge, not to be read anywhere.
    let wasTurnActive: boolean | undefined;
    const [turnJustEndedAtom, setTurnJustEndedAtom] = createSignal(0);

    // Dispatches ReconcileTurnActive to the pane reducer so TurnPhase follows
    // the backend's live turn state — used by BOTH the mount-time one-shot
    // (useAgentControllerStatus's Phase 3 GetControllerStatus) and every live
    // controllerstatus event. Does NOT touch turnJustEndedAtom — see
    // trackTurnJustEnded for why that's kept separate.
    function reconcileTurnActive(active: boolean): void {
        paneModel.dispatchPane({ type: "ReconcileTurnActive", at: Date.now(), active }, "system");
    }

    // Feeds the turnJustEndedAtom edge-detector. Deliberately called ONLY
    // from the live useControllerStatusEvents subscription (up from onMount,
    // always current), never from the mount-time GetControllerStatus
    // one-shot. That one-shot can resolve up to ~300s late — after Phase 1/2's
    // auth wait — by which point the live subscription may have already
    // tracked a real turn starting AND ending. Letting the stale snapshot
    // also drive wasTurnActive could clobber the correct live-tracked state
    // back to a value that no longer reflects reality, making the next live
    // event compute a spurious edge and re-fire the Haiku RPC for a turn that
    // isn't actually ending — reintroducing the over-trigger bug this fix
    // closes (reagent P1 on PR #2241).
    function trackTurnJustEnded(active: boolean): void {
        const turnJustEnded = didTurnJustEnd(wasTurnActive, active);
        // Update BEFORE calling flushPendingControllerRefresh below, not
        // after: that call synchronously checks isBackendTurnConfirmedIdle()
        // (backed by this same wasTurnActive) at call time, before any
        // await — the OLD ordering left it reading the STALE (pre-update)
        // value on exactly the genuine turn-end edge this call exists to
        // react to, so the deferred refresh's own safety gate saw the
        // turn as still "active" and refused to run — stranding it
        // forever on this trigger (the reactive turnIdle effect could
        // still rescue it asynchronously, but only if it happened to fire
        // separately). Codex P1 on PR #2338 (twenty-first re-review).
        wasTurnActive = active;
        if (turnJustEnded) {
            setTurnJustEndedAtom((n) => n + 1);
            // Run any controller refresh /login deferred because this exact
            // turn was still active when it succeeded — see
            // SlashCommandContext.deferControllerRefreshUntilIdle's doc
            // comment. No-ops if nothing is pending. `commands` is defined
            // further down this component body, but this function is only
            // ever invoked from async event callbacks registered after the
            // full component setup (including `commands`) has run. Codex
            // P1 on PR #2338 (thirteenth re-review).
            void commands.flushPendingControllerRefresh();
        }
    }

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
            scheduleRollOff();
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

    // Focus/visibility-triggered re-poll — the mount-time GetControllerStatus
    // (onControllerStatus above) is one-shot, and the live useControllerStatusEvents
    // subscription only self-heals a missed turn-end if a LATER live event
    // arrives. If the single turn-end push is missed (backgrounded window, a
    // MPS reconnect gap, a pane remount that doesn't re-trigger the MPS
    // persisted-event replay — see REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md
    // §3/§4 item 5) nothing else corrects it until the *next* turn starts.
    // Re-poll on every background→foreground transition to drive the two
    // effects below (turnJustEnded edge-tracking, deferred controller-refresh
    // recovery), independent of event-bus replay semantics. Skips the
    // initial `true` at mount (already covered by the one-shot above) via
    // `{ defer: true }`.
    //
    // Deliberately does NOT call reconcileTurnActive from this snapshot
    // (removed per direct user request — "Working" state must not depend on
    // window focus at all). `turn_active` isn't a clean boolean: it reads
    // transiently false during the gap between one CLI round's session_end
    // and the next round's start (the same phenomenon StreamFlushObserved's
    // Done->Streaming re-promotion exists to paper over on a different
    // path), and this poll fires on the single most common user action —
    // clicking/refocusing a pane to check on it — making that race far more
    // visible than it needs to be. The live useControllerStatusEvents
    // subscription below still reconciles TurnPhase from the backend's
    // periodic status heartbeat (persistent.rs's spawn_status_heartbeat,
    // every 20s while a turn is active) independent of focus, so the
    // original stuck-Working-forever gap this mechanism was built for is
    // still bounded — just by that heartbeat's cadence instead of an
    // instant refocus, not left uncovered entirely.
    const windowFocused = makeWindowFocusSignal();
    createEffect(
        on(
            windowFocused,
            (focused) => {
                if (!focused) return;
                void BlockService.GetControllerStatus(model.blockId)
                    .then((rts) => {
                        if (!rts) return;
                        const active = !!rts.turn_active;
                        // Mirror the live useControllerStatusEvents handler below —
                        // reagent P2: a turn-end detected ONLY via this focus poll
                        // (the missed-live-push case this mechanism exists for)
                        // must still bump turnJustEndedAtom, or
                        // useAgentActivitySummary/useNextPromptSuggestion silently
                        // never fire for that turn's completion.
                        trackTurnJustEnded(active);
                        // Independent of the turnJustEnded edge above: this RPC
                        // response is itself a fresh, authoritative confirmation of
                        // idleness whenever active is false — attempt the deferred
                        // refresh unconditionally on that, not only when
                        // trackTurnJustEnded's edge detector fires. didTurnJustEnd
                        // requires prev===true (a CONFIRMED active state to
                        // transition FROM); a pane whose backend state was never
                        // confirmed either way before this poll (wasTurnActive
                        // undefined — e.g. the live confirming controllerstatus
                        // push was itself missed, the exact gap this poll exists to
                        // self-heal) computes turnJustEnded=false here even though
                        // this is the FIRST time idleness has been confirmed. The
                        // reactive turnPhaseAtom effect (below) can't rescue this
                        // either: ReconcileTurnActive no-ops (same state reference)
                        // once local turnPhase already reads idle/Done, so it never
                        // re-fires off this same confirmation. Without this call,
                        // a /login deferred mid-turn — where the turn then ends via
                        // session_end while the live idle controllerstatus push is
                        // lost — would leave the refresh (and any held messages)
                        // stuck until the user happens to send another message.
                        // codex P1 on PR #2338 (twenty-eighth re-review).
                        if (!active) {
                            void commands.flushPendingControllerRefresh();
                        }
                    })
                    .catch(() => {
                        // Best-effort — the live subscription and next mount remain
                        // as fallbacks; nothing user-visible to report on failure.
                    });
            },
            { defer: true }
        )
    );

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
        isBackendTurnActive: () => wasTurnActive === true,
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
        isBackendTurnConfirmedIdle: () => wasTurnActive === false,
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

    // Deliver queued-while-busy ("send now") messages at the next tool-call
    // boundary — the agent finishes its current step and then picks them up
    // (the CLI consumes a stdin message at its next inference, after the
    // in-flight tool's result). Falls back to turn end (Idle/Done) so a
    // tool-less turn still delivers. Holding until here is what lets ArrowUp
    // recall an un-sent message first.
    let prevTool: string | null = null;
    createEffect(() => {
        const tool = paneModel.state.currentTool;
        const phaseKind = paneModel.state.turnPhase.kind;
        const newToolCall = tool !== null && tool !== prevTool;
        prevTool = tool;
        const turnIdle = phaseKind === "Idle" || phaseKind === "Done";
        if ((newToolCall || turnIdle) && commands.hasHeldMessages()) {
            void commands.flushHeldMessages();
        }
        // Independent of the above: run any controller refresh /login
        // deferred because a turn was active when it succeeded, the moment
        // this pane's OWN turnPhase reflects idle — regardless of whether
        // there are any held messages to otherwise trigger it. Deliberately
        // reacts to turnPhaseAtom directly rather than relying solely on
        // trackTurnJustEnded's live-controllerstatus-event edge detector:
        // (1) a turn also ends via the independent session_end -> TurnEnd
        // stream path (useTurnLifecycle.ts's finalizeTurn), which is not
        // synchronized with the controllerstatus event stream reagent P1
        // found flushHeldMessages/trackTurnJustEnded alone don't cover; (2)
        // a pane that mounts onto an ALREADY-active turn never initializes
        // trackTurnJustEnded's wasTurnActive (deliberately, to avoid a
        // false busy->idle edge on the very first live event — see its own
        // doc comment), so if /login succeeds during that pre-existing turn
        // and it ends before any OTHER live event arrives,
        // didTurnJustEnd(undefined, false) never fires and — with no held
        // messages either — nothing would ever run the deferred refresh at
        // all. Codex P1 on PR #2338 (seventeenth re-review, both points).
        // No-ops when nothing is pending.
        if (turnIdle) {
            void commands.flushPendingControllerRefresh();
        }
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
    const zoomFactor = createMemo(() => {
        const meta = block()?.meta;
        const z = meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });

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
        <AgentPaneProviders dormant={hidden} block={block} agent={currentAgent}>
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
                stopping={paneModel.state.turnPhase.kind === "Interrupting"}
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
                    onLoadOlder={liveFeedOn() ? undefined : history.loadOlder}
                    loadingOlder={history.loadingOlder}
                    hasOlderHistory={() => !liveFeedOn() && history.historyOffset() > 0}
                    followingRef={(f) => {
                        followingBottom = f;
                    }}
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

            {/* Login UI — bottom-docked like AgentDecisionPanel/
                AgentQuestionPanel below, not inside the scrollable document.
                See #2429 follow-up: it used to render inside
                AgentDocumentView's header slot, which pinned it to the top
                of the scroll area. */}
            <AgentAuthPanel
                authUrl={status.authUrl}
                authNotice={status.authNotice}
                onDismissAuthNotice={() => status.setAuthNotice(null)}
                onCancelLogin={status.cancelLogin}
                onUseTerminal={status.useTerminalInstead}
                authProviderId={provider()?.id ?? providerKey()}
                launchPhase={status.launchPhase}
            />

            {/* The separate blue "Log in" bar that used to render here on
                `status.canRetry()` is GONE — it was a second CTA for the
                identical action (relogin()) as the failure row below, and the
                two could be on screen simultaneously (a pane reopened after an
                auth failure seeds the row from persisted block meta while the
                mount-time launch flow independently sets canRetry). That case
                now raises a synthetic `turnAttempted: false` auth failure
                instead (see the createEffect above), so it renders through the
                one shared row, labelled "Log in" and still passing
                `retryAfterLogin: false`.

                `status.canRetry()` itself is DELIBERATELY still live — it is
                not only a display gate: useAgentCommands reads it to fast-fail
                sends into an unauthenticated agent, and /login reads it too.
                Deleting the signal along with this bar would silently re-open
                that hole. See
                docs/specs/PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md. */}

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
                tool_result over the persistent controller's stdin. Cancel
                (button / Escape) is a REAL protocol-level decline via
                `handleCancel`, not a UI-only dismiss — replaces the old
                "Answer later" minimize, which never told the agent anything.
                Spec: docs/specs/SPEC_ASK_USER_QUESTION_2026_06_15.md,
                docs/specs/SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md. */}
            <AgentQuestionPanel
                pending={pendingQuestions}
                onAnswer={handleAnswer}
                onCancel={handleCancel}
                isDormant={hidden}
            />

            {/* Queue sits directly below the feed so the user's newly-
                typed message lands next to the live conversation it's
                queued against. Previously lived below the activity log;
                repositioning per SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.
                No "Send now" affordance — Esc on an empty composer delivers
                a queued message immediately instead (mirrors Claude Code
                CLI). See SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06.md. */}
            <PendingMessagesPanel pendingMessages={pendingMessages} />

            {/* PR F — Disconnected banner. Visible when the stream
                tore down while a turn was in flight (kind=Disconnected).
                Sits above the status line so the working spinner (which
                is already suppressed because `isWorking(Disconnected) =
                false`) doesn't overlay the disconnect message. The
                Reconnect button re-subscribes; the reducer's
                `StreamSubscribe` arm clears the phase to Idle. Spec
                docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md
                §6.4. */}
            {/* Credentials-revoked disclosure chip — appears when an identity
                account this agent was linked to is deleted (or unlinked)
                while the pane is live. Honest wording: the running process
                still holds working tokens until restarted; enforcement lands
                at the next spawn (layer 3).
                SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md §3. */}
            <AgentCredentialsRevokedChip agentId={agentId} />
            {/* Something other than the user asked to shut this agent down:
                15 s to keep it (SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5). */}
            <ShutdownPendingBanner blockId={model.blockId} agentId={agentId} agentName={agentName()} />
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
                phase={(() => paneModel.state.turnPhase)}
                onReconnect={() => {
                    // Standard stream-reconnect path: dispatch
                    // `StreamSubscribe` against the live pane. If the
                    // backend has auto-reconnected between render and
                    // click, the second subscribe is harmless — the
                    // reducer's Disconnected→Idle transition is the
                    // same regardless of who calls it.
                    paneModel.dispatchPane({ type: "StreamSubscribe", at: Date.now() }, "user");
                }}
            />
            {/* Non-Claude quick-fork fallback note — SPEC_AGENT_QUICK_FORK_NEW_TAB_2026_08_21.md
                §4.4. Set once on the new block's meta right after a fork
                lands with no `--fork-session` support; stays for the pane's
                lifetime, no dismiss button (quick-fork.ts). */}
            <ForkProviderFallbackBanner meta={() => block()?.meta} />

            {/* Pinned activity dock — long-running shells (and later crons /
                subagents) sit just above the composer so task status is adjacent
                to where the user's attention already is. Moved from the top per
                SPEC_ACTIVITY_DOCK_BOTTOM_MOVE_2026_06_20. */}
            <ActivityDock
                documentNodes={paneModel.document}
                blockId={model.blockId}
                backgroundTasksAtom={backgroundTasksAtom}
            />

            {/* Working indicator — the turn's own status, so it sits directly
                above the composer, with the dock's long-running tasks stacked
                above it. Reads bottom-up as narrowing scope: what's running in
                the background (dock) → what this turn is doing right now
                (here) → where you type.

                Normal-flow row, not the overlay it used to be — see the
                .agent-document-scroll-region comment above for what that
                change removed. When it appears or disappears it changes the
                scroll region's clientHeight, which
                AgentDocumentVirtualList's clientHeight ResizeObserver already
                re-pins on; that observer was written for exactly this family
                of normal-flow siblings (the retry bar, decision/question
                panels, PendingMessagesPanel), so the row simply joins them
                rather than needing its own tracked height signal.

                Shows spinner + elapsed while loading, "✓ Worked · Ns" on
                completion. Acts as a visual turn delimiter; stays until the
                next message is sent.
                See SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md §2 and
                SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md. */}
            <div class="agent-working-row-anchor">
                <Show when={workingRowVisible()}>
                    <AgentWorkingRow
                        loading={workingRowLoading()}
                        stopping={paneModel.state.turnPhase.kind === "Interrupting"}
                        currentTool={paneModel.state.currentTool}
                        currentToolArg={paneModel.state.currentToolArg}
                        toolPromoted={hasPromotedTool()}
                        sessionStats={paneModel.state.sessionStats}
                        turnTokens={paneModel.state.turnTokens}
                        launchPhase={status.launchPhase()}
                        onCancelLogin={status.cancelLogin}
                        hasAuthUrl={!!status.authUrl()}
                        waitingReason={(() => {
                            const phase = paneModel.state.turnPhase;
                            return phase.kind === "Streaming" ? (phase.waitingReason ?? null) : null;
                        })()}
                        retryAfterMs={(() => {
                            const phase = paneModel.state.turnPhase;
                            return phase.kind === "Streaming" ? (phase.retryAfterMs ?? null) : null;
                        })()}
                        compacting={paneModel.state.compacting}
                        reconnecting={paneModel.state.reconnecting}
                    />
                </Show>
            </div>

            {/* Session banners (interrupted / resume-failed / large /
                archived). Above the strip, NOT inside the Shell drawer where
                they used to live: a conversation-level disclosure can't be
                gated behind a terminal toggle — see
                SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §3. */}
            <AgentSessionNotices
                blockId={model.blockId}
                blockAtom={block}
                providerId={provider()?.id ?? ""}
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
