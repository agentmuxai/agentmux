// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BrainSpinner } from "@/app/element/BrainSpinner";
import { DragOverlay } from "@/app/element/dragoverlay";
import { PaneTabRenameInput } from "@/app/element/PaneTabRenameInput";
import { PaneTabStrip } from "@/app/element/PaneTabStrip";
import { dispatchIfRegistered as dispatchDocIfRegistered } from "@/app/store/agent-document-store";
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
import {
    dispatch as dispatchPane,
    dispatchIfRegistered as dispatchPaneIfRegistered,
    snapshot as paneSnapshot,
} from "@/app/store/agent-pane-state-store";
import { workingFromPhase } from "@/app/store/agent-pane-state/types";
import {
    registerActivity as registerAgentActivity,
    unregisterActivity as unregisterAgentActivity,
} from "@/app/store/agentActivity";
import { getRecentDispatches } from "@/app/store/command-source";
import { ContextMenuModel } from "@/app/store/contextmenu";
import {
    atoms,
    createBlock,
    getApi,
    openOrFocusPaneByView,
    pushNotification,
    refocusNode,
    WOS,
} from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { BlockService, ObjectService } from "@/app/store/services";
import { scheduleOnSettle } from "@/app/util/settle-detector";
import { loadAccounts, type AgentAccounts } from "@/app/view/identity/identity-model";
import { handleAgentIdChange } from "@/app/view/term/termagent";
import { makeWindowFocusSignal } from "@/app/window/window-focus";
import { ConfirmModal } from "@/element/modal";
import { useModalLayer } from "@/element/modal-layer";
import { ModalLayer } from "@/element/ModalLayer";
import {
    closeBlockInStack,
    getLayoutModelForStaticTab,
    pushBlockOntoStack,
    setActiveBlockInStack,
} from "@/layout/index";
import { holdLeafRevealGate, scheduleLeafRevealLift } from "@/app/store/tab-reveal";
import { getTrail } from "@/log/render-trail";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { earliestLiveAttachedStartMs } from "./activity/attached-task";
import { allSubagentsAtom } from "./activity/subagent-source";
import { hasRunningPromotedTool, nextToolPromotionAt } from "./activity/tool-adapter";
import type { AgentViewModel } from "./agent-model";
import "./agent-view.scss";
import { ActivityDock } from "./components/ActivityDock";
import { AgentComposerStrip } from "./components/AgentComposerStrip";
import { AgentControlBar } from "./components/AgentControlBar";
import { AgentCredentialsRevokedChip } from "./components/AgentCredentialsRevokedChip";
import { AgentDecisionPanel } from "./components/AgentDecisionPanel";
import { AgentDisconnectedBanner } from "./components/AgentDisconnectedBanner";
import { AgentAuthPanel, AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter, AgentWorkingRow } from "./components/AgentFooter";
import { AgentPicker, useAgentDefinitions, useOpenDefinitionMap } from "./components/AgentPicker";
import { AgentQuestionPanel } from "./components/AgentQuestionPanel";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { AgentShellSubblock } from "./components/AgentShellSubblock";
import { ForkProviderFallbackBanner } from "./components/ForkProviderFallbackBanner";
import { PaneRow } from "./components/PaneRow";
import { PendingMessagesPanel } from "./components/PendingMessagesPanel";
import { ResizableDetailsDrawer } from "./components/ResizableDetailsDrawer";
import { SlashCommandPicker } from "./components/SlashCommandPicker";
import { SlashHelpPanel } from "./components/SlashHelpPanel";
import { useForkSet } from "./fork/useForkSet";
import { AgentHistoryTabView } from "./history/AgentHistoryTabView";
import { useActivityLog } from "./hooks/useActivityLog";
import { useAgentActivitySummary } from "./hooks/useAgentActivitySummary";
import { useAgentCloseConfirm } from "./hooks/useAgentCloseConfirm";
import { useAgentCommands } from "./hooks/useAgentCommands";
import { useAgentControllerStatus } from "./hooks/useAgentControllerStatus";
import { useAgentDecisions } from "./hooks/useAgentDecisions";
import { useAgentDropAttach } from "./hooks/useAgentDropAttach";
import { useAgentFailure } from "./hooks/useAgentFailure";
import { useAgentKeyboard } from "./hooks/useAgentKeyboard";
import { useAgentQuestions } from "./hooks/useAgentQuestions";
import { useBlockActivity } from "./hooks/useBlockActivity";
import { didTurnJustEnd, useControllerStatusEvents } from "./hooks/useControllerStatusEvents";
import { useHistoryPagination } from "./hooks/useHistoryPagination";
import { useInSessionSearch } from "./hooks/useInSessionSearch";
import { useNextPromptSuggestion } from "./hooks/useNextPromptSuggestion";
import { useProcessCount } from "./hooks/useProcessCount";
import { computeTermSizeFromEl, usePtyWidth } from "./hooks/usePtyWidth";
import { useScrollToNode } from "./hooks/useScrollToNode";
import { useSnapshotPersistence } from "./hooks/useSnapshotPersistence";
import { injectHistoryLink } from "./inject-history-link";
import { buildResumePreflightNode, injectResumePreflight } from "./inject-resume-preflight";
import { useResumePreflight } from "./hooks/useResumePreflight";
import { HISTORY_TAB_FOR_META_KEY, openOrFocusHistoryTab } from "./open-history-tab";
import { getProvider } from "./providers";
import { buildStartupPayload, resolveAccounts } from "./startup/buildStartupPayload";
import type { SignalPair } from "./state";
import { createAgentAtoms } from "./state";
import type { DocumentNode } from "./types";
import { useAgentStream } from "./useAgentStream";

// Matches a CSI or OSC ANSI escape sequence (the standard sindresorhus/ansi-regex
// pattern). Used by sanitizeLogTextForTerminal below to strip escape sequences
// out of arbitrary text (e.g. a bang command's subprocess stdout/stderr) before
// it's wrapped in formatLogLine's own SGR color codes and written into the live
// shell Terminal — otherwise embedded sequences in that text could move the
// cursor, recolor arbitrary regions, or otherwise corrupt the shared terminal's
// rendered state (this text is not our own trusted output; it's shell-command
// output the user chose to run).
// Matches BrainSpinner.scss's own `.is-fading` opacity transition duration —
// the AgentPicker->AgentPresentationView cross-fade (AgentViewWrapper, below)
// reuses the same visual timing so the two fades feel like one brand moment
// rather than two differently-tuned animations back to back.
const PICKER_FADE_OUT_MS = 200;

const ANSI_SEQUENCE_RE = new RegExp(
    "[\\u001B\\u009B][[\\]()#;?]*(?:(?:(?:(?:;[-a-zA-Z\\d/#&.:=?%@~_]+)*|" +
        "[a-zA-Z\\d]+(?:;[-a-zA-Z\\d/#&.:=?%@~_]*)*)?\\u0007)|" +
        "(?:(?:\\d{1,4}(?:;\\d{0,4})*)?[\\dA-PR-TZcf-ntqry=><~]))",
    "g"
);

/**
 * Strips ANSI escape sequences and other terminal control bytes from `text`,
 * then converts bare `\n` to `\r\n` so multi-line text renders as separate
 * lines instead of a cursor staircase (xterm.js, like a real terminal,
 * treats `\n` as line-feed-only — it doesn't imply carriage return).
 */
const sanitizeLogTextForTerminal = (text: string): string => {
    const withoutAnsi = text
        .replace(ANSI_SEQUENCE_RE, "")
        // Any stray control byte not part of a matched sequence above
        // (malformed/truncated escapes, bare ESC, BEL, CR, etc.) — \t and \n
        // are kept; \n is converted to \r\n next.
        .replace(/[\x00-\x08\x0b-\x1f\x7f]/g, "");
    return withoutAnsi.replace(/\n/g, "\r\n");
};

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
    // A block opened as a read-only history reader (openOrFocusHistoryTab)
    // — takes priority over the live/picker gate below, and never toggles
    // back: closing this reading posture is closing the tab, not swapping
    // content in place. See SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.1.
    const isHistoryTab = () => !!block()?.meta?.[HISTORY_TAB_FOR_META_KEY];

    // Cross-fade AgentPicker -> AgentPresentationView instead of an instant
    // hard cut when this SAME block gains an agentId in place (launching an
    // agent from a blank "+" tab's picker — no block-stack mutation, no
    // node remount, so PR #2761's leaf reveal gate never covers this
    // transition at all). SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md
    // §2.3/§4 Option B.
    //
    // Same stuck-visible race as block.tsx's ready()-gate (see
    // docs/retro/retro-block-ready-gate-spinner-stuck-visible-race-2026-08-23.md):
    // seeding `pickerVisible` from `!agentId()` read once at construction,
    // then relying on `on(agentId, ..., {defer: true})`'s first (swallowed)
    // run to treat "agentId already set" as "nothing to do," is two
    // different reads of `agentId()` taken at two different times. If
    // `agentId()` resolves in the gap between them, the seed is never
    // corrected. Fixed the same way: the first observation and the seed
    // are now the same read, inside the same effect.
    const [pickerVisible, setPickerVisible] = createSignal(true);
    const [pickerFadingOut, setPickerFadingOut] = createSignal(false);
    let pickerFadeRaf: number | undefined;
    let pickerFadeTimeout: ReturnType<typeof setTimeout> | undefined;
    let pickerGateInitialized = false;
    onCleanup(() => {
        if (pickerFadeRaf !== undefined) cancelAnimationFrame(pickerFadeRaf);
        clearTimeout(pickerFadeTimeout);
    });
    createEffect(() => {
        const id = agentId();
        if (pickerFadeRaf !== undefined) cancelAnimationFrame(pickerFadeRaf);
        clearTimeout(pickerFadeTimeout);
        if (!pickerGateInitialized) {
            // First observation of `agentId()` for this mount: reflect it
            // directly, no fade — there's nothing painted yet to fade from
            // either way.
            pickerGateInitialized = true;
            setPickerVisible(!id);
            setPickerFadingOut(false);
            return;
        }
        if (id) {
            if (!pickerVisible()) return; // already past the transition
            // One rAF so the picker paints at full opacity at least once
            // before the fade starts — flipping straight to the
            // "is-fading" class in this same tick would apply opacity:0
            // on the very first paint, with nothing to visibly transition
            // from.
            pickerFadeRaf = requestAnimationFrame(() => setPickerFadingOut(true));
            pickerFadeTimeout = setTimeout(() => {
                setPickerVisible(false);
                setPickerFadingOut(false);
            }, PICKER_FADE_OUT_MS);
        } else {
            // Lost the agentId (not a normal path, but stay correct) —
            // show the picker again immediately, no fade needed going
            // this direction.
            setPickerFadingOut(false);
            setPickerVisible(true);
        }
    });

    // Portal target for the marching-ants progress bar (below) — the bar's
    // own working-state/turn-phase reads live inside AgentPresentationView
    // (deep in .agent-pane-stack-content, BELOW the tab strip in DOM order),
    // but it needs to render visually in its own row between the tab strip
    // and the content, which no CSS trick can reach across that boundary
    // (.agent-pane-stack-content's containing-block ancestors all clip
    // overflow before it could escape upward). A signal (not a plain ref
    // variable) so <Portal>'s mount prop — read reactively by
    // AgentPresentationView on its own first render — never races the ref
    // callback that sets it just above in JSX order.
    const [progressBarSlot, setProgressBarSlot] = createSignal<HTMLDivElement>();

    // In-pane tabs — rendered here (not inside AgentPresentationView) so the
    // strip stays visible whether the active member is a launched
    // conversation OR a blank/picker tab (AgentPicker, no agentId yet).
    // Previously the strip lived only in AgentPresentationView and was
    // driven solely by fork lineage, so clicking "+" (which pushes a blank,
    // agentId-less block onto this pane's stack) swapped the ENTIRE pane to
    // AgentPicker with no strip at all — the just-open tab (e.g. "Camper")
    // had no visible pill to switch back to. Report: 2026-08-09.
    //
    // Two independent sources are merged into one tab list:
    //  - "stack" tabs: every block in this pane's own blockStack (Phase 2 of
    //    SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.3) — always
    //    present, agentId or not. This is the primary source and mirrors
    //    term.tsx's termTabs.
    //  - "fork" tabs: conversations sharing this one's `parent_id` lineage
    //    that are open in ANOTHER top-level pane (Phase 3/4) — kept as
    //    additional pills, deduped against the stack by blockId, so
    //    cross-pane fork-switching keeps working.
    const layoutModel = getLayoutModelForStaticTab();
    const [agentDefinitions] = useAgentDefinitions();
    const [openDefinitions] = useOpenDefinitionMap();
    const forks = useForkSet({
        definitions: agentDefinitions,
        openBlockByDef: openDefinitions,
        activeDefinitionId: () => agentId() ?? "",
    });
    // Same "switchable" filter AgentPresentationView used to apply: a fork
    // with no open blockId anywhere can't be jumped to, so it isn't offered.
    const switchableForks = createMemo(() => forks().filter((f) => f.isActive || !!f.blockId));

    interface PaneTab {
        blockId: string;
        label: string;
        /** AgentDefinition id, when this tab has launched an agent — stays
         *  undefined for a still-blank picker tab (nothing to rename). */
        definitionId?: string;
        /** A read-only history-reader tab (openOrFocusHistoryTab) — never
         *  double-click-renamable (that flow renames the shared
         *  AgentDefinition, which a history tab must never touch) and
         *  always labeled distinctly from its live sibling tab. */
        isHistoryTab?: boolean;
    }
    // Rename overrides, keyed by blockId — set synchronously by
    // handleTabRenameConfirm below so a just-renamed tab (including a
    // dormant, non-active member whose block meta isn't reactively tracked
    // here) reflects its new label immediately, without waiting on the
    // SetMetaCommand round-trip (term.tsx's titleOverrides precedent).
    const [titleOverrides, setTitleOverrides] = createSignal<Record<string, string>>({});
    const labelForBlock = (id: string): PaneTab => {
        // The currently-mounted member reads its own reactive block meta;
        // every other (dormant) stack member isn't mounted here — read its
        // last-persisted meta directly, same as term.tsx's termTabs does.
        const meta = id === model.blockId ? block()?.meta : WOS.getObjectValue<Block>(WOS.makeORef("block", id))?.meta;
        const definitionId = meta?.["agentId"] as string | undefined;
        const isHistoryTab = !!meta?.[HISTORY_TAB_FOR_META_KEY];
        if (isHistoryTab) {
            // Deliberately ignores titleOverrides/agentName — a history
            // tab is never user-renamed (see PaneTab.isHistoryTab) and
            // must read distinctly from its live sibling, which carries
            // the same copied agentName in its own meta.
            return { blockId: id, label: "History", isHistoryTab: true };
        }
        const label = titleOverrides()[id] ?? (meta?.["agentName"] as string) ?? definitionId ?? "New Agent";
        return { blockId: id, label, definitionId };
    };
    const stackTabs = createMemo<PaneTab[]>(() => {
        // Reactive dependency: re-derive whenever ANY layout mutation
        // happens (matches term.tsx's termTabs).
        layoutModel.localTreeStateAtom();
        const node = layoutModel.getNodeByBlockId(model.blockId);
        const stack = node?.data?.blockStack?.length ? node.data.blockStack : [model.blockId];
        return stack.map(labelForBlock);
    });
    const combinedTabs = createMemo<PaneTab[]>(() => {
        const stack = stackTabs();
        const stackIds = new Set(stack.map((t) => t.blockId));
        const extras = switchableForks()
            .filter((f) => f.blockId && !stackIds.has(f.blockId))
            .map((f): PaneTab => ({ blockId: f.blockId!, label: f.title, definitionId: f.definitionId }));
        return [...stack, ...extras];
    });
    // Only render tab pills once there's something to switch BETWEEN — a
    // lone conversation shows just the "+" (no pill for itself). The
    // moment a 2nd tab exists, both (including the first) appear.
    const visibleTabs = createMemo(() => (combinedTabs().length > 1 ? combinedTabs() : []));
    const activeBlockId = createMemo(() => {
        layoutModel.localTreeStateAtom();
        return layoutModel.getNodeByBlockId(model.blockId)?.data?.activeBlockId ?? model.blockId;
    });
    // Per-pane zoom for the tab strip itself — mirrors
    // AgentPresentationView's own zoomFactor memo (term:zoom block meta +
    // clamp, further down this file) but keyed off activeBlockId() rather
    // than model.blockId: this component renders the tab strip for the
    // whole stack, so the strip's zoom must track whichever tab is
    // actually active, not the pane's own root block — the two diverge
    // once more than one tab/fork is open. Independent read of the same
    // live meta key labelForBlock (above) already reads for non-active
    // stack members — not a shared computation with
    // AgentPresentationView's own memo, which lives in a child component
    // out of scope here (SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md §A.3).
    const tabStripZoomFactor = createMemo(() => {
        const id = activeBlockId();
        const meta = id === model.blockId ? block()?.meta : WOS.getObjectValue<Block>(WOS.makeORef("block", id))?.meta;
        const z = meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });
    // Activating a tab has two cases, both "switch," neither "create": (1)
    // the target block already lives in THIS pane's own block-stack — swap
    // the active member in place; (2) a fork open as its own separate
    // top-level pane — jump focus to it via refocusNode, same as the
    // picker's "Switch to existing" flow already does.
    const handleTabSwitch = (targetBlockId: string) => {
        if (targetBlockId === activeBlockId()) return;
        const node = layoutModel.getNodeByBlockId(model.blockId);
        if (!node) return;
        const stack = node.data?.blockStack?.length ? node.data.blockStack : [model.blockId];
        if (stack.includes(targetBlockId)) {
            // Switching to an ALREADY-OPEN pill forces the same remount as
            // creating a new one — layoutStack.ts's setActiveBlockInStack
            // evicts the NodeModel just like pushBlockOntoStack does.
            // Codex's review of PR #2761 caught that this path (unlike
            // create-new) had no reveal gate at all.
            const gen = holdLeafRevealGate(node.id);
            setActiveBlockInStack(layoutModel, node.id, targetBlockId);
            scheduleLeafRevealLift(node.id, gen);
        } else {
            refocusNode(targetBlockId);
        }
    };
    // "+" on the tab strip. Opens a blank agent tab — the same starting-view
    // picker (`AgentPicker`, "select an existing agent or create a new
    // one") a brand-new agent pane shows — instead of jumping straight into
    // the launch/fork modal. Mirrors term.tsx's handleTermTabAdd: allocate
    // an unplaced block via pane.open (no agentId meta, so this wrapper's
    // `agentId()` gate falls through to AgentPicker) and push it onto this
    // pane's own stack. No modal, no implicit fork of the current
    // conversation.
    const handleNewAgentTab = async (): Promise<void> => {
        const initialNode = layoutModel.getNodeByBlockId(model.blockId);
        if (!initialNode) return;
        // Hide this pane while the new tab settles — pane.open's RPC round
        // trip plus pushBlockOntoStack's forced remount (layoutStack.ts's
        // own doc comment) is exactly the piecemeal-paint flicker
        // SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22 addresses.
        const revealGen = holdLeafRevealGate(initialNode.id);
        try {
            let paneOpenResult: { block_id: string };
            try {
                paneOpenResult = (await TabRpcClient.rpcCall(
                    "pane.open",
                    { view: "agent", skip_placement: true, meta: { view: "agent" } },
                    {}
                )) as { block_id: string };
            } catch (e: unknown) {
                pushNotification({
                    icon: "fa-triangle-exclamation",
                    title: "New tab failed",
                    message: e instanceof Error ? e.message : String(e),
                    timestamp: new Date().toISOString(),
                    type: "error",
                    expiration: Date.now() + 8000,
                });
                return;
            }
            // This pane could have closed while the RPC above was in flight —
            // re-resolve the node fresh rather than trusting a pre-await
            // reference. If it's gone, the skip_placement block we just
            // created has nowhere to attach to; delete it instead of leaving
            // an orphaned, unreachable block behind.
            const node = layoutModel.getNodeByBlockId(model.blockId);
            if (!node) {
                await ObjectService.DeleteBlock(paneOpenResult.block_id).catch(() => {});
                return;
            }
            pushBlockOntoStack(layoutModel, node.id, paneOpenResult.block_id);
        } finally {
            // Pair with holdLeafRevealGate above — runs on every exit path
            // (success, RPC failure, pane-closed-mid-flight) so the leaf
            // never stays hidden forever.
            scheduleLeafRevealLift(initialNode.id, revealGen);
        }
    };
    // × on a tab (also middle-click, via PaneTabStrip's onMouseDown).
    // Mirrors term.tsx's handleTermTabClose: resolve the block's OWNING
    // node — for a stack member that's this pane (pop it out, delete just
    // that block; last member closes the pane), for a cross-pane fork tab
    // it's that other pane (same semantics apply there). closeBlockInStack
    // guards against a blockId that isn't a member, so a stale tab entry
    // can't close the wrong thing. Gated the same as handleTabSwitch, and
    // ONLY when targetBlockId is the resolved node's own active member
    // (reagent's follow-up review of PR #2761: gating unconditionally hides
    // the leaf's real, unchanging content for the close+settle window when
    // closing a background tab, since gatingNodeIds() hides the whole node
    // regardless of whether a remount is actually about to happen) —
    // popping the ACTIVE member out of a multi-member stack reassigns
    // activeBlockId and evicts the NodeModel, the identical forced-remount
    // pattern as switching; popping a background member changes nothing
    // visible. Note: `node` here may be a different pane than this
    // component's own (the cross-pane fork tab case), so this checks the
    // resolved node's own activeBlockId, not this pane's activeBlockId().
    const handleTabClose = (targetBlockId: string) => {
        const node = layoutModel.getNodeByBlockId(targetBlockId);
        if (!node) return;
        if (node.data?.activeBlockId !== targetBlockId) {
            void closeBlockInStack(layoutModel, node.id, targetBlockId);
            return;
        }
        const gen = holdLeafRevealGate(node.id);
        void closeBlockInStack(layoutModel, node.id, targetBlockId).finally(() => {
            scheduleLeafRevealLift(node.id, gen);
        });
    };
    // Double-click a tab to rename it (only meaningful once it has launched
    // an agent — a still-blank picker tab has nothing to rename). TWO writes
    // are required (reagent P1 on PR #2488): stack-tab labels read
    // `block.meta.agentName` (labelForBlock above), which
    // RenameAgentDefinitionTitleCommand does NOT touch — it renames only the
    // AgentDefinition row (name/branch_label), which is what fork tabs and
    // the picker's agent list read. Writing only the definition left a
    // stack tab's pill (and its pane title) showing the stale name forever;
    // writing only block meta would leave fork tabs/the picker stale. The
    // titleOverrides entry gives the pill its new label synchronously.
    const [renamingBlockId, setRenamingBlockId] = createSignal<string | null>(null);
    const handleTabRenameConfirm = async (tab: PaneTab, title: string): Promise<void> => {
        setRenamingBlockId(null);
        if (!tab.definitionId) return;
        const prevOverride = titleOverrides()[tab.blockId];
        setTitleOverrides((prev) => ({ ...prev, [tab.blockId]: title }));
        // Definition rename FIRST — it's the authoritative store (fork tabs,
        // AgentPicker). If it fails, nothing has been written anywhere:
        // roll back the optimistic override and stop, so the two stores
        // can't diverge (reagent P2 on PR #2488 round 6 — the old order
        // could land the meta write and then fail the rename, leaving the
        // stack pill and fork tabs permanently showing different names).
        try {
            await RpcApi.RenameAgentDefinitionTitleCommand(TabRpcClient, { id: tab.definitionId, title });
        } catch (e: unknown) {
            setTitleOverrides((prev) => {
                const next = { ...prev };
                if (prevOverride === undefined) delete next[tab.blockId];
                else next[tab.blockId] = prevOverride;
                return next;
            });
            pushNotification({
                icon: "fa-triangle-exclamation",
                title: "Rename failed",
                message: e instanceof Error ? e.message : String(e),
                timestamp: new Date().toISOString(),
                type: "error",
                expiration: Date.now() + 8000,
            });
            return;
        }
        // Denormalized copy the stack pill + pane title read. If THIS write
        // fails after a successful rename, the kept titleOverrides entry
        // still shows the new name for the rest of the session; the meta
        // catches up on the agent's next launch (launchAgentDefinition
        // rewrites agentName from the definition).
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", tab.blockId),
            meta: { agentName: title } as any,
        }).catch(() => {});
    };

    return (
        <ModalLayer scope="pane">
            {/* Flex-column stack: strip on top, content filling the rest.
                Required because ModalLayer's mount is a plain 100%-height
                box, NOT a flex column — without this wrapper the strip's
                own height ADDS to `.agent-view`'s height:100%, overflowing
                the pane and clipping the composer off the bottom (found
                live 2026-08-09: "agent pane has no text input"). */}
            <div class="agent-pane-stack">
                {/* Progress bar's own overlay strip — floats above the tab
                    strip, never reserving layout space (SPEC_AGENT_PANE_
                    PROGRESS_BAR_OVERLAY_NO_GAP_2026_08_25.md). Empty div;
                    its only content is whatever AgentPresentationView
                    portals into it below. Positioned in agent-view.scss
                    (.agent-pane-progress-bar-slot). */}
                <div class="agent-pane-progress-bar-slot" ref={(el) => setProgressBarSlot(el)} />
                <div class="agent-pane-stack-content">
                    {/* Tab strip floats over the content instead of
                        reserving its own row (SPEC_AGENT_PANE_TAB_STRIP_
                        OVERLAY_2026_08_10.md) — with a single conversation
                        open, the strip is exactly the "+" button's own
                        28×28px box (shrink-to-fit + hidden-until-2nd-tab,
                        SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md),
                        so the conversation renders, and can be scrolled,
                        underneath the rest of this row — unobstructed except
                        for wherever a real tab or the "+" actually sits. The
                        "+" always renders — that's how you'd get to a second
                        tab — but the tab pill itself stays hidden until
                        there's something to switch BETWEEN (see visibleTabs
                        above). */}
                    <PaneTabStrip
                        tabs={visibleTabs()}
                        activeId={activeBlockId()}
                        zoomFactor={tabStripZoomFactor}
                        getId={(t) => t.blockId}
                        getLabel={(t) => t.label}
                        onActivate={handleTabSwitch}
                        onClose={handleTabClose}
                        onTabDoubleClick={(t) => t.definitionId && setRenamingBlockId(t.blockId)}
                        renderLabel={(t) =>
                            renamingBlockId() === t.blockId && t.definitionId ? (
                                <PaneTabRenameInput
                                    initialValue={t.label}
                                    onConfirm={(title) => void handleTabRenameConfirm(t, title)}
                                    onCancel={() => setRenamingBlockId(null)}
                                />
                            ) : (
                                <span class="pane-tab-label">{t.label}</span>
                            )
                        }
                        onAdd={() => void handleNewAgentTab()}
                        addTitle="New tab"
                    />
                    <Show
                        when={isHistoryTab()}
                        fallback={
                            <>
                                <Show when={agentId()}>
                                    <AgentPresentationView
                                        model={model}
                                        agentId={agentId()}
                                        agentDefinitions={agentDefinitions}
                                        progressBarMount={progressBarSlot}
                                    />
                                </Show>
                                {/* Cross-fades out on top of AgentPresentationView
                                    once agentId() is set, instead of the two
                                    Shows above hard-swapping instantly — see
                                    pickerVisible/pickerFadingOut above.
                                    SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.3. */}
                                <Show when={pickerVisible()}>
                                    <div
                                        class="agent-picker-host"
                                        classList={{
                                            // Applied the instant agentId()
                                            // is set (same render as
                                            // AgentPresentationView
                                            // appearing) so this never sits
                                            // in normal flow alongside it,
                                            // even for one frame.
                                            "is-overlay": !!agentId(),
                                            "is-fading": pickerFadingOut(),
                                            "is-reduced-motion": atoms.prefersReducedMotionAtom(),
                                        }}
                                    >
                                        <AgentPicker model={model} />
                                    </div>
                                </Show>
                            </>
                        }
                    >
                        {/* No progressBarMount here — a history tab is a
                            read-only reader with no live turn/working state
                            of its own, so there's nothing for a progress
                            bar to represent. */}
                        <AgentHistoryTabView model={model} />
                    </Show>
                </div>
            </div>
        </ModalLayer>
    );
};

AgentViewWrapper.displayName = "AgentViewWrapper";

// Launch flow lives in `flows/launch-flow.ts` — Step 2 of
// specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.

const AgentPresentationView = ({
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
    /** DOM node (owned by AgentViewWrapper, between the tab strip and the
     *  content) the marching-ants progress bar portals into — see the
     *  signal's own doc comment in AgentViewWrapper. undefined only for the
     *  ref-not-yet-assigned instant on first mount. */
    progressBarMount: () => HTMLDivElement | undefined;
}): JSX.Element => {
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

    // Reactive agent-definition list (from the wrapper) — used to resolve
    // the current AgentDefinition object for identity/memory modal requests.
    const currentAgent = createMemo(() => agentDefinitions().find((a) => a.id === agentId));

    // Fork tab strip + in-pane "+" tab logic moved up to AgentViewWrapper
    // (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.3 follow-up,
    // 2026-08-09) so the strip stays visible even when the pane's active
    // member is a blank/picker tab with no agentId yet — see that
    // component's comment for the full rationale.

    // Wire the pane-scoped modal callback into the model so the single
    // title-bar "Stash" (backpack) icon can open the unified tabbed
    // modal (Accounts + Memory) without holding a SolidJS context in the
    // model. Mirrors the former _setOverlayTab pattern; supersedes the
    // separate _openIdentityModal / _openMemoryModal callbacks.
    const modalLayer = useModalLayer();
    onMount(() => {
        model._openAgentStashModal = () => {
            // Prefer cmd:cwd (actual launch cwd, set by launchAgentDefinition)
            // over AgentDefinition.working_directory, which is often empty or a
            // stale default for template-launched and continuation agents.
            const block = model.blockAtom();
            const workingDirectory = (block?.meta?.["cmd:cwd"] as string) || currentAgent()?.working_directory || "";
            const agent = currentAgent() ?? null;
            modalLayer.open({
                kind: "agent-stash",
                agentId,
                agentName: agentName(),
                workingDirectory,
                // No loadable definition (quick-launch pane) → default to
                // the Memory tab; the Accounts tab works from agentId alone
                // but the Memory tab is the more useful default for a pane
                // with no saved definition yet.
                initialTab: agent ? "accounts" : "memory",
            });
        };
    });
    onCleanup(() => {
        model._openAgentStashModal = null;
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
        const a = agentAtoms();
        paneModel = registerAgentPane(model.blockId, {
            agentId,
            documentSetter: a.documentAtom[1],
            projections: {
                streaming: a.streamingStateAtom[1],
                sessionStats: a.sessionStatsAtom[1],
                sessionTotals: a.sessionTotalsAtom[1],
                currentTool: a.currentToolAtom[1],
                turnTokens: a.turnTokensAtom[1],
                contextTokens: a.contextTokensAtom[1],
                contextWindow: a.contextWindowAtom[1],
                pending: a.pendingMessagesAtom[1],
                initPhase: a.initPhaseAtom[1],
                turnPhase: a.turnPhaseAtom[1],
                detailsOpen: a.detailsOpenAtom[1],
                currentToolArg: a.currentToolArgAtom[1],
                failure: a.failureAtom[1],
                compacting: a.compactingAtom[1],
                attachedTask: a.attachedTaskAtom[1],
                registryAttachedTaskSince: a.registryAttachedTaskSinceAtom[1],
                reconnecting: a.reconnectingAtom[1],
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
        (window as unknown as { __agentLayout?: () => unknown }).__agentLayout = () => layoutSnapshot(model.blockId);
    }

    // Activity log — collects per-session diagnostic entries from launch
    // flow, subprocess lifecycle, slash commands, errors, etc. `log` is
    // passed down to every hook whose signature takes a `LogFn`, but only
    // "system"-tagged entries (bang-command output, `useAgentCommands.ts`'s
    // `dispatchBangCommand`; slash-command results, `commands/dispatch.ts`)
    // are genuinely user-initiated console-style interactions written into
    // the shell terminal (AgentShellSubblock's `onTermReady`) — everything
    // else (launch-flow status, auth prompts, CLI resolution, etc.) is
    // passive app-internal noise the shell should stay clean of. First cut
    // redirected every tag, which made the shell open with a wall of
    // "[cli] checking for claude...", "[auth] ..." etc. sitting above the
    // real prompt — reported live after removing the separate log panel.
    // `logLines` stays as a backlog (system-tagged entries only) so a bang
    // command's output logged while the drawer is closed still shows once
    // it reopens. `logFlushedCount` tracks how many of `logLines()` have
    // already been written into *some* terminal instance (live or
    // replayed) — every write, whether live or catch-up, advances it.
    // Without this, each drawer close/reopen replayed the entire backlog
    // again on top of whatever real PTY content the terminal (now durably)
    // restored (SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md).
    const { lines: logLines, append: appendLog } = useActivityLog();
    const [termWrite, setTermWrite] = createSignal<((text: string) => void) | null>(null);
    let logFlushedCount = 0;

    const formatLogLine = (tag: string, text: string, level?: "info" | "error" | "warn"): string => {
        const body = `[${tag}] ${sanitizeLogTextForTerminal(text)}`;
        if (level === "error") return `\x1b[31m${body}\x1b[0m`;
        if (level === "warn") return `\x1b[33m${body}\x1b[0m`;
        return `\x1b[90m${body}\x1b[0m`;
    };

    const log = (tag: string, text: string, level?: "info" | "error" | "warn") => {
        if (tag !== "system") return;
        appendLog(tag, text, level);
        const write = termWrite();
        if (write) {
            write(formatLogLine(tag, text, level));
            logFlushedCount = logLines().length;
        }
    };

    // Fired once per terminal mount (drawer open) — replays only the log
    // lines added since the last flush (whether that flush was this same
    // catch-up on a prior mount, or a live write while the drawer was open),
    // then keeps the write function around so `log` above writes live from
    // here on.
    const handleShellTermReady = (write: (text: string) => void) => {
        const all = logLines();
        for (let i = logFlushedCount; i < all.length; i++) {
            write(formatLogLine(all[i].tag, all[i].text, all[i].level));
        }
        logFlushedCount = all.length;
        setTermWrite(() => write);
    };
    const handleShellTermDispose = () => setTermWrite(null);

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
    // Brain-spinner loading overlay (see
    // docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md):
    // shown from mount, cross-fades out once real content has actually
    // painted, so a content-heavy pane never sits blank while it replays.
    //
    // `onHistoryReady` fires right after the NDJSON parse/dispatch — BEFORE
    // the resulting DOM's layout/paint, which is the dominant cost for heavy
    // sessions (500-600ms, see SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md).
    // Starting the fade there (or after any flat delay) would make the
    // overlay disappear before painting finishes for exactly the
    // content-heavy case this exists to cover — reproducing the blank
    // window instead of fixing it. `scheduleOnSettle` (same Long-Task-quiet
    // detector `tab-reveal.ts` uses for the analogous tab-switch case) waits
    // for the main thread to actually go quiet post-dispatch before the
    // fade starts. `showLoadingOverlay` then unmounts the overlay entirely
    // once the fade transition has had time to finish, instead of leaving
    // an invisible-but-present pointer-events:none div forever.
    const [historyLoaded, setHistoryLoaded] = createSignal(false);
    const [showLoadingOverlay, setShowLoadingOverlay] = createSignal(true);
    // Separate from `historyLoaded` below: this only means "the transcript
    // has actually painted" — the effect after `status` is defined (further
    // down) decides whether that's enough to start the fade, or whether the
    // auth-panel pop-in flicker fix also needs to hold the overlay a bit
    // longer.
    const [historyPainted, setHistoryPainted] = createSignal(false);
    let cancelSettleWait: (() => void) | undefined;
    let loadingOverlayFadeTimeout: ReturnType<typeof setTimeout> | undefined;
    // Two extra rAFs between "settle detected" and actually starting the
    // fade — see the doc comment on scheduleOnSettle's call site below for
    // why: Long-Task quiet alone can be reached before the browser has
    // actually PAINTED this pane's content (live-reported flicker,
    // 2026-08-11). Tracked so a pane close mid-transition doesn't write to
    // disposed signals.
    let settlePaintRaf1: number | undefined;
    let settlePaintRaf2: number | undefined;
    onCleanup(() => {
        cancelSettleWait?.();
        clearTimeout(loadingOverlayFadeTimeout);
        if (settlePaintRaf1 !== undefined) cancelAnimationFrame(settlePaintRaf1);
        if (settlePaintRaf2 !== undefined) cancelAnimationFrame(settlePaintRaf2);
    });
    const history = useHistoryPagination({
        blockId: model.blockId,
        model: paneModel,
        outputFormat,
        // Jekt direction detection during replay: FROM == this agent →
        // outgoing bubble (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2).
        agentName,
        definitionId: agentId,
        onHistoryReady: () => {
            historyReadyFn?.();
            cancelSettleWait = scheduleOnSettle(() => {
                // `scheduleOnSettle` only watches for Long-Task quiet
                // (no synchronous block >50ms) — but this pane's actual
                // reveal work (AgentDocumentVirtualList's measure
                // ResizeObserver, scroll-pin/anchor-restore effects) is
                // spread across several async/RAF-scheduled steps that
                // never register as one long task each. "No long tasks
                // observed" can therefore be reached before the browser has
                // actually PAINTED the resulting rows — starting the fade
                // there let the spinner finish disappearing while the pane
                // was still genuinely blank underneath, then the real
                // content popped in abruptly once that async chain finally
                // caught up (live-reported flicker, 2026-08-11, repro:
                // switch tabs, screenshot-burst the transition — spinner
                // fully faded by ~400ms, content not visible until ~500ms).
                // A double requestAnimationFrame is the standard "wait for
                // an actual paint to have happened" technique: the second
                // callback is guaranteed to run only after whatever was
                // queued as of the first one's frame has been painted.
                settlePaintRaf1 = requestAnimationFrame(() => {
                    settlePaintRaf2 = requestAnimationFrame(() => {
                        setHistoryPainted(true);
                    });
                });
            });
        },
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

    // Will the next spawn continue the conversation this pane is displaying, or
    // start a new one? Asked once on mount, before the user can type — the
    // whole point is to beat the first send, since every other continuity
    // signal is retrospective. See `hooks/useResumePreflight.ts`.
    const resumePreflight = useResumePreflight(model.blockId);

    // True when content older than the working session exists out of view:
    // set by the restore/pagination clamp paths (scopeClamped) OR derived
    // from a live clamp — after the reducer's StreamFlush trim, the fresh
    // session_outcome divider is always the first document node.
    const earlierHistoryAvailable = createMemo(() => {
        if (history.scopeClamped()) return true;
        const first = agentAtoms().documentAtom[0]()[0];
        return first?.type === "session_outcome" && first.outcome === "fresh";
    });

    // Read-side view of the document with the "Open Agent History" link row
    // injected as a normal, scrolling document node (§3.2 of
    // SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md) —
    // replaces the old pinned-above-the-scroll-region PaneRow. The real
    // setter is passed through unchanged: AgentDocumentView/
    // createAgentViewState only ever read `documentAtom[0]`, never call the
    // setter, so pairing a derived read with the real write side is safe
    // (nothing writes through this pair, so there's nothing to desync).
    const displayDocumentAtom: SignalPair<DocumentNode[]> = [
        () =>
            injectResumePreflight(
                injectHistoryLink(agentAtoms().documentAtom[0](), earlierHistoryAvailable()),
                buildResumePreflightNode(
                    agentAtoms().documentAtom[0](),
                    resumePreflight.result(),
                    resumePreflight.showSteps(),
                ),
            ),
        agentAtoms().documentAtom[1],
    ];

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
        dispatchPaneIfRegistered(model.blockId, { type: "ReconcileTurnActive", at: Date.now(), active }, "system");
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
            const timer = setTimeout(() => {
                if (workingFromPhase(agentAtoms().turnPhaseAtom[0]())) return;
                dispatchDocIfRegistered(model.blockId, {
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
        dispatchDocIfRegistered(model.blockId, {
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
        // A successful recovery (seed-from-global / terminal login) refreshed
        // the credential — retry the failed turn so the agent recovers in one
        // click. Lazy arrow: retryLastTurn is defined below but only invoked at
        // runtime (post-click), by which point it's initialized.
        onRecovered: () => {
            // If a DIFFERENT, overlapping recovery flow is still running,
            // leave the failure banner and loginWaiting() both untouched
            // instead of clearing-and-retrying now. THIS flow's own
            // credential is confirmed good, but relogin()/useGlobalLogin()/
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
            dispatchPane(model.blockId, { type: "FailureCleared" }, "system");
            retryLastTurn();
        },
        getInitialTermSize: () => computeTermSizeFromEl(rootRef),
        // Mount-time TurnPhase reconciliation — see
        // docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
        // Finding 1. dispatchPaneIfRegistered (not dispatchPane) because
        // this can resolve before registerPane() has run for a pane still
        // mid-mount.
        onControllerStatus: (rts) => {
            reconcileTurnActive(!!rts.turn_active);
        },
    });

    // Brain-spinner loading overlay, part 2 (part 1 is the historyPainted/
    // scheduleOnSettle chain above `status`'s own definition — this has to
    // live down here since it reads `status.launchPhase()`). A fresh
    // agent's mount-time launch-flow.ts runs Phase 1 (resolving-cli) then
    // Phase 2 (checking-auth) before AgentAuthPanel's authUrl/authNotice can
    // ever become non-null; when they do, that panel pops into normal flex
    // flow between the scroll region and the composer strip/AgentFooter,
    // pushing both down with no warning — often well after historyPainted
    // already flipped true for a brand-new, empty-history agent (live-
    // reported flicker, 2026-08-17: "bottom paints first, then gets pushed
    // down"). Hold the fade until the launch flow has moved past the two
    // phases that can still cause that pop-in, so it happens hidden behind
    // the mask instead — same principle as the historyPainted rAF-pair fix
    // above, just gating on a different async source. Bounded by a 3s
    // safety timeout so a launch path that never calls setLaunchPhase (a
    // future code path, a test double) can't leave the pane stuck behind
    // the spinner forever — worse than the flicker this exists to fix.
    const [authPhaseTimedOut, setAuthPhaseTimedOut] = createSignal(false);
    let authPhaseSafetyTimeout: ReturnType<typeof setTimeout> | undefined;
    onMount(() => {
        authPhaseSafetyTimeout = setTimeout(() => setAuthPhaseTimedOut(true), 3000);
    });
    onCleanup(() => clearTimeout(authPhaseSafetyTimeout));
    const authPhaseSettled = createMemo(() => {
        if (authPhaseTimedOut()) return true;
        const phase = status.launchPhase();
        return phase !== null && phase.kind !== "resolving-cli" && phase.kind !== "checking-auth";
    });
    createEffect(() => {
        if (historyPainted() && authPhaseSettled() && !historyLoaded()) {
            setHistoryLoaded(true);
            loadingOverlayFadeTimeout = setTimeout(() => setShowLoadingOverlay(false), 220);
        }
    });

    // status.isLoading() is `flowRunning() || !agentReady()` — it never
    // becomes true during relogin()/loginViaTerminal()/useGlobalLogin(),
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
        if (agentAtoms().failureAtom[0]()?.data.code === "auth") return "unauthenticated";
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
            status.notifyControllerHealthy();
            // Also clear a stale live "auth"-classified state.failure —
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
            if (paneSnapshot(model.blockId)?.failure?.data.code === "auth") {
                dispatchPane(model.blockId, { type: "FailureCleared" }, "system");
            }
        },
    });

    // Focus/visibility-triggered re-poll — the mount-time GetControllerStatus
    // (onControllerStatus above) is one-shot, and the live useControllerStatusEvents
    // subscription only self-heals a missed turn-end if a LATER live event
    // arrives. If the single turn-end push is missed (backgrounded window, a
    // WPS reconnect gap, a pane remount that doesn't re-trigger the WPS
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
        turnPhase: agentAtoms().turnPhaseAtom[0],
        getRootWidth: () => rootRef?.offsetWidth,
    });

    // Ghost-text next-prompt suggestion (composer). Populated by AgentFooter
    // via its isComposerEmptyRef prop below. Defaults to "empty" if the
    // footer hasn't mounted yet — matches the common case (no suggestion
    // exists yet either, since one only appears after a completed turn).
    let composerIsEmptyFn: (() => boolean) | null = null;
    useNextPromptSuggestion({
        blockId: model.blockId,
        turnPhase: agentAtoms().turnPhaseAtom[0],
        turnJustEndedAtom,
        isComposerEmpty: () => composerIsEmptyFn?.() ?? true,
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
    // Forwarded to ActivityDock so it can render registry-known background
    // tasks the transcript itself has no record of (Tier 1 of
    // docs/reports/REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md).
    const backgroundTasksAtom = useAgentStream({
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
        // Jekt direction detection on the live stream: FROM == this agent
        // → outgoing bubble (SPEC_JEKT_SECURITY_AND_VISIBILITY §3.2).
        agentName: agentName(),
        // Re-engage message-list auto-scroll for a turn that starts from
        // the queue-drain path — see usePendingMessageAcceptance's
        // `onTurnStartFromQueue` doc comment. `scrollToBottomFn` is declared
        // below (assigned once AgentDocumentView mounts); referencing it in
        // this closure is safe regardless of declaration order since the
        // closure only runs later, on a live `agent-message-accepted` event.
        onTurnStartFromQueue: () => scrollToBottomFn?.(),
    });

    // Mutable ref to the scrollToBottom function exposed by
    // AgentDocumentView. Called by AgentFooter's onTyping when the user
    // starts composing AND by useAgentCommands.onSent after the user's
    // message has been appended to the document (SPEC_AGENT_PANE_FOLLOWUPS
    // item #1). Declared here so both useAgentCommands and the JSX below
    // can close over the same reference; assigned once AgentDocumentView
    // mounts via scrollToBottomRef.
    let scrollToBottomFn: (() => void) | null = null;

    // Tracks AgentWorkingRow's rendered height (0 when hidden) so
    // .agent-document can reserve exactly that much bottom padding — the
    // row now floats over the scroll region instead of pushing it up as a
    // normal-flow sibling (SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md
    // §3.2), so without this, "scrolled to true bottom" would leave the
    // last message hidden underneath the floating row instead of above it.
    // Exposed to .agent-document (nested inside AgentDocumentView, several
    // component boundaries away) via a CSS custom property set on the
    // shared .agent-document-scroll-region ancestor — custom properties
    // cascade through the DOM tree regardless of component boundaries.
    const [workingRowHeight, setWorkingRowHeight] = createSignal(0);

    // Shared by the anchor's <Show> and the backdrop's <Show>/color below
    // (SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md §5) so the two
    // can't drift out of sync — both need to agree on "is the row visible
    // at all" and "is it the loading (vs. worked) variant" independent of
    // the anchor's own inset-from-scrollbar geometry.
    const workingRowLoading = createMemo(
        () => showingLaunchActivity() || workingFromPhase(agentAtoms().turnPhaseAtom[0]())
    );
    const workingRowVisible = createMemo(
        () =>
            workingRowLoading() ||
            agentAtoms().sessionStatsAtom[0]() != null ||
            agentAtoms().compactingAtom[0]() != null ||
            agentAtoms().reconnectingAtom[0]() != null,
    );

    // Ref CALLBACK, not a one-shot onMount(): originally fixed a bug where
    // Agent History's now-removed `bodyMode` in-place swap (PR #2509)
    // remounted this anchor div WITHIN one persistent AgentPresentationView
    // instance, while a plain onMount (this component's own, firing once
    // for the instance's whole lifetime) kept observing the stale, now-
    // detached pre-swap node — `workingRowHeight` silently froze and the
    // floating AgentWorkingRow visibly overlapped the last message. That
    // specific in-place remount is gone now that Agent History is its own
    // tab (SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md
    // §3.1) — but a ref callback (re-observing on every mount of THIS
    // element, not just the component's first) is still the more robust
    // shape in general, so it stays: correct whether this component mounts
    // once or many times over its host block's life (which, per §1 of that
    // spec, it now does on every ordinary tab switch too).
    let workingRowRO: ResizeObserver | undefined;
    const attachWorkingRowAnchor = (el: HTMLDivElement) => {
        workingRowRO?.disconnect();
        workingRowRO = undefined;
        if (typeof ResizeObserver === "undefined") return;
        const ro = new ResizeObserver((entries) => {
            const h = entries[0]?.contentRect.height ?? 0;
            setWorkingRowHeight(h);
        });
        ro.observe(el);
        workingRowRO = ro;
    };
    onCleanup(() => workingRowRO?.disconnect());

    // True once the pane's in-flight Bash tool call has been promoted to a
    // live ActivityDock row (tool-adapter.ts) — AgentWorkingRow suppresses
    // its own "tool · arg" text once this flips, so the dock and the working
    // row never repeat the same information (report §4.3: "the dock takes
    // over, AgentWorkingRow goes calm/neutral"). Deliberately uses
    // hasRunningPromotedTool, not toolActivities — a *finished* call still
    // lingering in the dock during its retention window must not suppress a
    // different, newly-started tool call's own working-row text.
    //
    // Scheduled the same way as ActivityDock's own hasExpiring/
    // toolPromotionNonce: one setTimeout for the exact instant promotion
    // becomes due, not a continuous tick. The effect re-reads its own nonce
    // so that after that timer fires it reschedules for the next-earliest
    // still-pending promotion, instead of only ever handling one.
    const [hasPromotedTool, setHasPromotedTool] = createSignal(false);
    const [toolPromotionCheckNonce, setToolPromotionCheckNonce] = createSignal(0);
    createEffect(() => {
        toolPromotionCheckNonce();
        const nodes = agentAtoms().documentAtom[0]();
        const now = Date.now();
        setHasPromotedTool(hasRunningPromotedTool(nodes, now));
        const at = nextToolPromotionAt(nodes, now);
        if (at == null) return;
        const timer = setTimeout(() => setToolPromotionCheckNonce((n) => n + 1), Math.max(0, at - now) + 50);
        onCleanup(() => clearTimeout(timer));
    });

    // Attached-task axis dispatch — the deferred §6.1 call site of
    // SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md. Derives "≥1 live
    // agent-declared long-running activity" from the same shell + subagent +
    // tool aggregate the ActivityDock renders, and dispatches the reducer's
    // AttachedTaskObserved / AttachedTaskCleared on the 0→1 / 1→0 edges.
    // Both commands are idempotent in the reducer, so re-running this effect
    // while the level is unchanged is harmless. Wall-clock re-check timer:
    // a running Bash call crosses TOOL_PROMOTION_MS on a timer, not on a
    // document event (same discipline as the promotion effect above).
    const [attachedCheckNonce, setAttachedCheckNonce] = createSignal(0);
    createEffect(() => {
        attachedCheckNonce();
        const nodes = agentAtoms().documentAtom[0]();
        const subs = allSubagentsAtom();
        const now = Date.now();
        // `at` carries the earliest running activity's REAL start time, not
        // the observation time — a promoted Bash call has already been
        // running ≥30s when this first fires, and a pane reopened over an
        // already-running shell must not restart the elapsed counter at 0
        // (reagent P1 on PR #2489; matches AttachedTaskState.since's
        // "when this episode began" contract).
        const transcriptStartMs = earliestLiveAttachedStartMs(nodes, subs, model.blockId, now);
        // Combine with the registry-derived floor (Phase C of
        // SPEC_BACKGROUND_TASK_DASHBOARD_INTELLIGENCE_2026_08_20.md) —
        // attached if EITHER source says so, earliest start wins when both
        // do. Reading this atom here makes it a tracked dependency of this
        // effect too, same as documentAtom/allSubagentsAtom above, so a
        // registry-only update (no transcript change) still re-triggers
        // this recompute. Codex P1 on PR #2685: an earlier version had
        // useBackgroundTaskRegistry dispatch AttachedTaskObserved directly
        // into the SAME state this effect independently recomputes and
        // clears from transcript alone — that dispatch was immediately
        // undone the next time this effect ran and saw no transcript
        // evidence. Routing the registry signal through its own axis
        // instead of the shared one this effect owns fixes that.
        const registryStartMs = agentAtoms().registryAttachedTaskSinceAtom[0]();
        const startMs =
            transcriptStartMs != null && registryStartMs != null
                ? Math.min(transcriptStartMs, registryStartMs)
                : (transcriptStartMs ?? registryStartMs);
        const current = agentAtoms().attachedTaskAtom[0]() != null;
        if ((startMs != null) !== current) {
            dispatchPaneIfRegistered(
                model.blockId,
                startMs != null ? { type: "AttachedTaskObserved", at: startMs } : { type: "AttachedTaskCleared" },
                "system"
            );
        }
        const at = nextToolPromotionAt(nodes, now);
        if (at == null) return;
        const timer = setTimeout(() => setAttachedCheckNonce((n) => n + 1), Math.max(0, at - now) + 50);
        onCleanup(() => clearTimeout(timer));
    });

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
        // Scroll the user's own message into view after Enter. The hook
        // defers this to the next animation frame so the mounted node is
        // included in scrollHeight. See SPEC_AGENT_PANE_FOLLOWUPS item #1.
        onSent: () => scrollToBottomFn?.(),
        pendingMessagesAtom,
    });

    // Mark turn as active when the user sends a message — TurnStart
    // also clears stale sessionStats from the prior turn.
    const handleSendMessage = (message: string): Promise<void> => {
        // Bang commands (`!cmd`) output writes into the shell terminal (see
        // `log`/`handleShellTermReady` above). Auto-open the details drawer so
        // the shell — and thus the output — is immediately visible; without
        // this the user sees no feedback if the drawer is closed.
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
        const liveFailure = agentAtoms().failureAtom[0]();
        const authFailureToPreserve = liveFailure?.data.code === "auth" ? liveFailure.data : null;
        // Only start a NEW turn when the agent is idle. Dispatching TurnStart
        // while a turn is already running regresses Streaming → Submitting,
        // which would flicker the busy indicator back to its "Submitting"
        // look for no reason. A queued-while-busy message rides the running
        // turn; the queue-drain (agent-message-accepted) re-enters Submitting
        // if needed.
        if (!wasAlreadyWorking) {
            dispatchPane(model.blockId, { type: "TurnStart", at: Date.now(), content: message }, "user");
        }
        return commands.sendMessage(message, wasAlreadyWorking, authFailureToPreserve);
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
    const failureUI = useAgentFailure({
        blockId: model.blockId,
        // Per-pane model keeps dispatch sites default-safe; see useAgentStream above.
        model: paneModel,
        failure: agentAtoms().failureAtom[0],
        onRetry: retryLastTurn,
        onOpenArmory: () => void openOrFocusPaneByView("armory"),
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
            const [startupContentResult, startupBundleIdResult, version, identityLinks] = await Promise.all([
                RpcApi.GetAgentContentCommand(TabRpcClient, {
                    agent_id: agentId,
                    content_type: "startup",
                }).catch(() => null),
                RpcApi.GetAgentContentCommand(TabRpcClient, {
                    agent_id: agentId,
                    content_type: "startup_bundle_id",
                }).catch(() => null),
                Promise.resolve(getApi().getAboutModalDetails().version),
                RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agentId }).catch(() => []),
            ]);

            // If this agent has a Bundle selected as its startup source
            // (AgentStartupModal, Armory → ABF content), its
            // `instructions` take precedence over the legacy freeform
            // "startup" blob — which has no live authoring UI anywhere, see
            // docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md §5. Falls back to
            // the freeform blob when no bundle is selected (or it no longer
            // resolves, e.g. deleted), preserving any seed-manifest content.
            const startupBundleId = startupBundleIdResult?.content?.trim() || null;
            const startupBundle = startupBundleId
                ? await RpcApi.GetMemoryCommand(TabRpcClient, { id: startupBundleId }).catch(() => null)
                : null;
            const startupContent = startupBundle?.instructions?.trim()
                ? startupBundle.instructions
                : (startupContentResult?.content ?? null);

            // Resolve assigned accounts from the same db_agent_identity_links
            // rows spawn-time credential resolution and the agent pane's own
            // Identity tab already use — NOT the legacy AgentDefinition.accounts
            // JSON blob, which can silently diverge from what the agent
            // actually launches with (see docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md §1).
            const agentAccounts: AgentAccounts = {};
            for (const link of identityLinks) {
                agentAccounts[link.provider as keyof AgentAccounts] = link.account_id;
            }
            const accounts = resolveAccounts(agentAccounts, loadAccounts());

            const payload = buildStartupPayload({
                agent,
                providerDisplayName: provider()?.displayName ?? providerKey(),
                workDir: block()?.meta?.["cmd:cwd"] ?? "",
                version,
                accounts,
                peerAgents: agentDefinitions(),
                startupContent,
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

    // Context menu for copy
    const handleContextMenu = (e: MouseEvent) => {
        const sel = window.getSelection()?.toString();
        if (!sel) return; // no selection, let default behavior
        e.preventDefault();
        ContextMenuModel.showContextMenu([{ label: "Copy", click: () => clipboardWriteText(sel) }], e);
    };

    return (
        // Pane-scope `<ModalLayer>` lives in AgentViewWrapper (above)
        // so it covers BOTH this presentation view AND the picker
        // fallback. Anything in this subtree that calls
        // `useModalLayer()` resolves to that outer pane-scope layer.
        <div
            ref={rootRef}
            class="agent-view agent-view--presentation"
            classList={{ "agent-view--working-row-visible": workingRowVisible() }}
            style={{ zoom: zoomFactor(), "--agent-pane-zoom": String(zoomFactor()) }}
            onContextMenu={handleContextMenu}
            tabIndex={-1}
        >
            {/* Loading overlay — covers the pane from mount until the initial
                history load resolves, so a content-heavy pane never sits
                blank while it replays. See
                docs/specs/REPORT_AGENT_PANE_BLANK_LOAD_BRAIN_INDICATOR_2026_07_04.md. */}
            <Show when={showLoadingOverlay()}>
                <div
                    class="agent-pane-loading-overlay"
                    classList={{ "is-fading": historyLoaded(), "is-reduced-motion": atoms.prefersReducedMotionAtom() }}
                >
                    <BrainSpinner fading={historyLoaded()} />
                </div>
            </Show>
            {/* Gradient progress bar — 3px, marching-ants shimmer while
                working, hidden at rest. Colors derived from --accent-color
                via color-mix() so it adapts to all themes. Portaled into a
                slot AgentViewWrapper owns, between the tab strip and the
                content (its own row, never overlapping either) — this
                component's state (turnPhase, launch activity) is what
                drives it, but .agent-view (this component's own root,
                nested inside .agent-pane-stack-content, itself BELOW the
                tab strip in DOM order) can't reach a position above the
                tab strip through CSS alone; every ancestor between here and
                there clips overflow before an absolutely-positioned escape
                could ever become visible. See
                SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md §4 and
                SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md.
                Renders nothing until the slot ref is assigned (one frame,
                first mount only). */}
            <Show when={progressBarMount()}>
                <Portal mount={progressBarMount()!}>
                    <div
                        class="agent-pane-progress-bar"
                        classList={{
                            "agent-pane-progress-bar--active":
                                showingLaunchActivity() || workingFromPhase(agentAtoms().turnPhaseAtom[0]()),
                            "agent-pane-progress-bar--stopping":
                                agentAtoms().turnPhaseAtom[0]().kind === "Interrupting",
                        }}
                        role="progressbar"
                        aria-label="Agent working"
                        aria-valuemin={0}
                        aria-valuemax={100}
                    />
                </Portal>
            </Show>
            <DragOverlay message={dropAttach.dropMessage()} visible={dropAttach.isDragOver()} />
            {/* Pane title + back button now live in the block frame header,
                driven by AgentViewModel.viewName / viewIcon / endIconButtons.
                See SPEC_AGENT_PANE_FOLLOWUPS item #8. */}

            {/* Tab strip now lives in AgentViewWrapper — see its comment. */}

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
                by injectHistoryLink into displayDocumentAtom below, so it
                scrolls with the transcript instead of staying fixed in
                place. See SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.2. */}
            {/* Scroll region wrapper — .agent-document (inside AgentDocumentView)
                is absolutely positioned to fill this box, and AgentWorkingRow
                floats over its bottom edge instead of pushing it up as a
                normal-flow sibling. Lets the message list's scrollbar reach
                all the way to the bottom of this region instead of stopping
                short by AgentWorkingRow's own height.
                See docs/specs/SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md §3.2. */}
            <div
                class="agent-document-scroll-region"
                style={{ "--agent-working-row-height": `${workingRowHeight()}px` }}
            >
                {/* Full-width decorative backdrop, stacked below
                    .agent-document (and thus below its scrollbar) so the
                    working row's color reaches the pane's true right edge
                    without ever painting over the scrollbar — the anchor
                    below stays inset from the scrollbar gutter for that
                    reason, but nothing filled the gutter itself with a
                    matching color until now (the scrollbar track is
                    fully transparent). See
                    docs/specs/SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md. */}
                <Show when={workingRowVisible()}>
                    <div
                        class="agent-working-row-backdrop"
                        classList={{
                            // Must mirror AgentWorkingRow's OWN loading gate
                            // (`props.loading || !!props.compacting ||
                            // !!props.reconnecting`, AgentFooter.tsx), not
                            // just `workingRowLoading()` — reagent P1 on
                            // PR #2826: since `workingRowVisible()` above
                            // already renders this backdrop for
                            // compacting/reconnecting too, using only
                            // `workingRowLoading()` here left the backdrop in
                            // its `--worked` (non-loading) color while the
                            // row itself rendered its loading/accent-tinted
                            // variant during those two states — the exact
                            // backdrop/row color-mismatch
                            // SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md
                            // fixed for the plain loading case.
                            "agent-working-row-backdrop--loading":
                                workingRowLoading() ||
                                agentAtoms().compactingAtom[0]() != null ||
                                agentAtoms().reconnectingAtom[0]() != null,
                        }}
                    />
                </Show>

                <AgentDocumentView
                    documentAtom={displayDocumentAtom}
                    documentStateAtom={agentAtoms().documentStateAtom}
                    onOpenHistory={() => void openOrFocusHistoryTab({ currentBlockId: model.blockId, agentId })}
                    onAgentErrorLogin={() => {
                        // Must match onLoginAgain above: the button is labeled "Login
                        // Again", so it has to force a fresh OAuth regardless of
                        // provider. A prior version special-cased Claude into
                        // useGlobalLogin() instead — silently reusing the (possibly
                        // equally-stale) global credential under a "Login Again"
                        // label, which is exactly the kind of no-op this button
                        // exists to avoid (retro-agent-auth-relogin-noop-2026-07-01).
                        log("auth", "Login Again (inline error node) — forcing a fresh provider login");
                        void status.relogin();
                    }}
                    onLoadOlder={history.loadOlder}
                    loadingOlder={history.loadingOlder}
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
                    workingRowHeight={workingRowHeight}
                />

                {/* Working indicator — floats over the bottom of the scroll
                    region instead of occupying its own flex row. The ref'd
                    wrapper collapses to 0 height when the Show is false, so
                    the ResizeObserver above naturally reports 0 (no working
                    row currently rendered) without extra branching.
                    Shows spinner + elapsed while loading, "✓ Worked · Ns" on completion.
                    Acts as a visual turn delimiter; stays until next message is sent.
                    See SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md §2. */}
                <div class="agent-working-row-anchor" ref={attachWorkingRowAnchor}>
                    <Show when={workingRowVisible()}>
                        <AgentWorkingRow
                            loading={workingRowLoading()}
                            stopping={agentAtoms().turnPhaseAtom[0]().kind === "Interrupting"}
                            currentTool={agentAtoms().currentToolAtom[0]()}
                            currentToolArg={agentAtoms().currentToolArgAtom[0]()}
                            toolPromoted={hasPromotedTool()}
                            sessionStats={agentAtoms().sessionStatsAtom[0]()}
                            turnTokens={agentAtoms().turnTokensAtom[0]()}
                            launchPhase={status.launchPhase()}
                            onCancelLogin={status.cancelLogin}
                            hasAuthUrl={!!status.authUrl()}
                            waitingReason={(() => {
                                const phase = agentAtoms().turnPhaseAtom[0]();
                                return phase.kind === "Streaming" ? (phase.waitingReason ?? null) : null;
                            })()}
                            retryAfterMs={(() => {
                                const phase = agentAtoms().turnPhaseAtom[0]();
                                return phase.kind === "Streaming" ? (phase.retryAfterMs ?? null) : null;
                            })()}
                            compacting={agentAtoms().compactingAtom[0]()}
                            reconnecting={agentAtoms().reconnectingAtom[0]()}
                        />
                    </Show>
                </div>
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

            <Show when={status.canRetry()}>
                <div class="agent-retry-bar">
                    {/* Wired to relogin(), not startLaunchFlow: the mount-time
                        launch flow now only ever notifies and stops on
                        auth_failed (launch-flow.ts), so re-running it here
                        would immediately hit the same not-authenticated check
                        and bail again with no login ever attempted — an
                        infinite dead-end. relogin() is the one path that
                        actually starts a login.
                        retryAfterLogin: false — no turn was ever attempted
                        here (Phase 2 bailed before Phase 3), so there's no
                        failed turn to retry. Without this, a successful
                        login on an agent with prior history silently resent
                        its last old message as a new turn, burying the
                        "Login successful" notification under that turn's
                        immediate stream of output. */}
                    <button class="agent-retry-btn" onClick={() => void status.relogin({ retryAfterLogin: false })}>
                        Log in
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
                No "Send now" affordance — Esc on an empty composer delivers
                a queued message immediately instead (mirrors Claude Code
                CLI). See SPEC_AGENT_ESCAPE_STEER_QUEUED_MESSAGE_2026_07_06.md. */}
            <PendingMessagesPanel pendingMessages={pendingMessagesAtom[0]} />

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
                    dispatchPane(model.blockId, { type: "StreamSubscribe", at: Date.now() }, "user");
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
                documentAtom={agentAtoms().documentAtom}
                blockId={model.blockId}
                backgroundTasksAtom={backgroundTasksAtom}
            />

            {/* Composer status strip — single 28-32px row with live
                activity ticker and Log button that toggles the log panel.
                State (detailsOpen) is reducer-owned (PR #1068). */}
            <AgentComposerStrip
                loading={showingLaunchActivity() || workingFromPhase(agentAtoms().turnPhaseAtom[0]())}
                sessionTotals={agentAtoms().sessionTotalsAtom[0]()}
                turnTokens={agentAtoms().turnTokensAtom[0]()}
                processCount={processCount()}
                onProcessBadgeClick={() => {
                    createBlock({ meta: { view: "swarm" } });
                }}
                logOpen={agentAtoms().detailsOpenAtom[0]()}
                onToggleLog={() => dispatchPane(model.blockId, { type: "DetailsToggle" }, "user")}
                contextTokens={agentAtoms().contextTokensAtom[0]()}
                contextWindow={agentAtoms().contextWindowAtom[0]() ?? provider()?.contextWindow}
                authStatus={loginStatus()}
                blockId={model.blockId}
                blockAtom={block}
                providerId={provider()?.id ?? ""}
                agentMode={block()?.meta?.["agentMode"] as string | undefined}
                compacting={agentAtoms().compactingAtom[0]()}
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
                        scrollToBottomFn?.();
                    }}
                    onStopAgent={handleEscapeOnEmptyComposer}
                    onRecallLatestQueued={commands.recallLatestHeld}
                    getCompletions={commands.completions}
                    viewModel={model}
                    isComposerEmptyRef={(fn) => {
                        composerIsEmptyFn = fn;
                    }}
                />
                {/* Details panel — just the shell + control bar now. Activity-log
                    lines write directly into the terminal (handleShellTermReady)
                    instead of a separate panel here. Docked BELOW the composer
                    (SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md): the shell
                    stacks under the text input (which shifts up to make room,
                    since this region hugs the pane bottom). */}
                <Show when={agentAtoms().detailsOpenAtom[0]()}>
                    <div class="agent-composer-details" id={`agent-composer-details-${model.blockId}`}>
                        <AgentControlBar
                            blockId={model.blockId}
                            blockAtom={block}
                            providerId={provider()?.id ?? ""}
                            onOpenHistory={() => void openOrFocusHistoryTab({ currentBlockId: model.blockId, agentId })}
                        />
                        {/* Drag-to-height drawer wrapping the terminal — the actual
                            scrollable/resizable content. */}
                        <ResizableDetailsDrawer
                            blockId={model.blockId}
                            persistedHeight={block()?.meta?.["term:shellheight"] as number | undefined}
                        >
                            {/* Phase 0 spike (SPEC_AGENT_SHELL_XTERM_TERMINAL_2026_07_03.md):
                                real xterm+PTY terminal, spawned lazily on first
                                drawer open via a headless term sub-block. */}
                            <AgentShellSubblock
                                parentBlockId={model.blockId}
                                cwd={block()?.meta?.["cmd:cwd"] ?? ""}
                                existingSubBlockId={block()?.meta?.["term:shellsubblockid"] as string | undefined}
                                // Passed so the shell can cancel it out of its own
                                // font-size math -- total decoupling: the pane's
                                // zoom (this) and the shell's own zoom (term:zoom
                                // on the sub-block) are independent controls, and
                                // neither should visually leak into the other.
                                agentPaneZoom={zoomFactor}
                                onSubBlockCreated={(subBlockId) => {
                                    void RpcApi.SetMetaCommand(TabRpcClient, {
                                        oref: WOS.makeORef("block", model.blockId),
                                        meta: { "term:shellsubblockid": subBlockId } as any,
                                    });
                                }}
                                onTermReady={handleShellTermReady}
                                onTermDispose={handleShellTermDispose}
                            />
                        </ResizableDetailsDrawer>
                    </div>
                </Show>
            </div>
            <Show when={closeConfirm()}>
                {(info) => (
                    <ConfirmModal
                        open={true}
                        title="Close pane?"
                        description={`This agent has ${info().count} ${
                            info().count === 1 ? "process" : "processes"
                        } still running. Close and kill them all?`}
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
