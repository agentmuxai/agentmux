// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { DragOverlay } from "@/app/element/dragoverlay";
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
import { workingFromPhase, type PaneFailure } from "@/app/store/agent-pane-state/types";
import {
    registerActivity as registerAgentActivity,
    unregisterActivity as unregisterAgentActivity,
} from "@/app/store/agentActivity";
import { AgentDormancyProvider } from "./agent-dormancy";
import { usePaneTabVisibility } from "@/app/block/pane-tab-visibility";
import { getRecentDispatches } from "@/app/store/command-source";
import { resolveContextMenuRegion } from "@/app/block/context-menu-region";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { accountPickerItems, planBind, type BindMode } from "./failure/account-picker";
import {
    atoms,
    getApi,
    getBlockMetaKeyAtom,
    getSettingsKeyAtom,
    openOrFocusPaneByView,
    refocusNode,
    MOS,
} from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { BlockService } from "@/app/store/services";
import { muxEventSubscribe } from "@/app/store/mps";
import { createPaneReadiness } from "@/app/store/pane-readiness";
import { PaneLoadingCover } from "@/app/element/PaneLoadingCover";
import { scheduleOnSettle } from "@/app/util/settle-detector";
import {
    accountLabel,
    boundAccountEmail,
    loadAccounts,
    subscribeAccountChanges,
    type Account,
    type AgentAccounts,
} from "@/app/view/identity/identity-model";
import { handleAgentIdChange } from "@/app/view/term/termagent";
import { makeWindowFocusSignal } from "@/app/window/window-focus";
import { ModalLayer } from "@/element/ModalLayer";
import { ErrorBoundary } from "@/element/errorboundary";
import {
    setActiveBlockInStack,
    type NodeModel,
} from "@/layout/index";
import { findNode } from "@/layout/lib/layoutNode";
import { getTrail } from "@/log/render-trail";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { sleep } from "@/util/util";
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
import { Portal } from "solid-js/web";
import { earliestLiveAttachedStartMs } from "./activity/attached-task";
import { allSubagentsAtom } from "./activity/subagent-source";
import {
    hasBlockingForegroundToolCall,
    hasRunningPromotedTool,
    nextToolPromotionAt,
} from "./activity/tool-adapter";
import { paneBusyForInput } from "./working-indicator";
import { quickForkAgent } from "./quick-fork";
import { isBangCommand } from "./bang-command";
import { askSideQuestion } from "./btw";
import type { AgentViewModel } from "./agent-model";
import { agentModels } from "./agent-models";
import "./agent-view.scss";
import { ActivityDock } from "./components/ActivityDock";
import { AgentComposerStrip } from "./components/AgentComposerStrip";
import { AgentSessionNotices } from "./components/AgentSessionNotices";
import { AgentShellInfoPanel } from "./components/AgentShellInfoPanel";
import { AgentCredentialsRevokedChip } from "./components/AgentCredentialsRevokedChip";
import { ShutdownPendingBanner } from "./shutdown/ShutdownPendingBanner";
import { AgentDecisionPanel } from "./components/AgentDecisionPanel";
import { AgentDisconnectedBanner } from "./components/AgentDisconnectedBanner";
import { AgentAuthPanel, AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter, AgentWorkingRow } from "./components/AgentFooter";
import { AgentPicker, useOpenDefinitionMap } from "./components/AgentPicker";
import { AgentQuestionPanel } from "./components/AgentQuestionPanel";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { AgentShellSubblock } from "./components/AgentShellSubblock";
import { collapseDrawerOnShellExit } from "./shell-exit-collapse";
import { ForkProviderFallbackBanner } from "./components/ForkProviderFallbackBanner";
import { PaneRow } from "./components/PaneRow";
import { PendingMessagesPanel } from "./components/PendingMessagesPanel";
import { ResizableDetailsDrawer } from "./components/ResizableDetailsDrawer";
import { AgentStashModal } from "./components/AgentStashModal";
import { BtwOverlay } from "./components/BtwOverlay";
import { SlashCommandPicker } from "./components/SlashCommandPicker";
import { SlashHelpPanel } from "./components/SlashHelpPanel";
import { useForkSet } from "./fork/useForkSet";
import { AgentHistoryTabView } from "./history/AgentHistoryTabView";
import { useActivityLog } from "./hooks/useActivityLog";
import { useAmbientNarration } from "./hooks/useAmbientNarration";
import { useAgentActivitySummary } from "./hooks/useAgentActivitySummary";
import { useAgentCommands } from "./hooks/useAgentCommands";
import { useAgentControllerStatus } from "./hooks/useAgentControllerStatus";
import { useAgentDecisions } from "./hooks/useAgentDecisions";
import { useAgentDropAttach } from "./hooks/useAgentDropAttach";
import { useAgentFailure } from "./hooks/useAgentFailure";
import { computeAccountBindCandidates } from "./failure/bind-account-candidates";
import { retryRecheckAfterBind } from "./failure/recheck-after-bind";
import { decideSyntheticRow } from "./failure/synthetic-row";
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
import { closeAgentTab } from "./close-agent-tab";
import { HISTORY_TAB_FOR_META_KEY, openOrFocusHistoryTab } from "./open-history-tab";
import { getProvider } from "./providers";
import { lastLinkedAccountId } from "./providers/provider-id-aliases";
import { buildStartupPayload, resolveAccounts } from "./startup/buildStartupPayload";
import { createAgentAtoms } from "./state";
import type { DocumentNode } from "./types";
import { ShutdownOverlay } from "./shutdown/ShutdownOverlay";
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
// the AgentPicker->AgentPresentationView cross-fade (AgentBlockContent,
// below) reuses the same visual timing so the two fades feel like one brand
// moment rather than two differently-tuned animations back to back.
const PICKER_FADE_OUT_MS = 200;

// Shell drawer's height until the user drags it (then `term:shellheight`
// wins). 80% of the drawers' shared 220px default — the shell opens on its
// own for every `!cmd`, so it should take less of the transcript by default.
const SHELL_DRAWER_DEFAULT_HEIGHT = 176;

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
 * Content half of the agent pane — becomes `AgentViewModel.viewComponent`.
 * Switches between the agent picker and the live presentation view (or the
 * read-only history reader). Constructed fresh per stack member, exactly
 * like every other `viewComponent` — the "one instance, one immutable
 * blockId for its lifetime" `ViewModel` contract is unchanged here.
 *
 * The pane-scope `<ModalLayer>` wrap lives HERE, not in `AgentPaneChrome`.
 * Chrome does now mount for every agent pane, so this is no longer
 * load-bearing the way it was when chrome was stack-size-gated — but it
 * stays here deliberately: the launch picker (`useModalLayer()`, opened
 * before any agentId exists) belongs to CONTENT, so wrapping at the content
 * root keeps the layer's lifetime tied to the thing that opens modals
 * rather than to chrome. Wrapping here covers both the
 * pre-launch picker AND the post-launch presentation view, and (once
 * `AgentPaneChrome` does mount) sits inside it, so the pane-scope lock
 * still holds across the entire pane lifecycle either way.
 * SPEC_LAUNCH_MODAL_PANE_SCOPE_2026_05_25.md.
 */
export const AgentBlockContent = ({ model }: { model: AgentViewModel }): JSX.Element => {
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

    // ReAgent P2 on SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md's
    // PR: owned by the model (one useAgentDefinitions() subscription per
    // ViewModel instance) instead of a fresh call here, so AgentPaneChrome
    // can read the SAME list via nodeModel.activeViewModel() instead of
    // independently subscribing a second time — see AgentViewModel.agentDefinitions'
    // own doc comment (agent-model.ts).
    const agentDefinitions = model.agentDefinitions;

    return (
        <ModalLayer scope="pane">
            <Show
                when={isHistoryTab()}
                fallback={
                    <>
                        <Show when={agentId()}>
                            <AgentPresentationView
                                model={model}
                                agentId={agentId()}
                                agentDefinitions={agentDefinitions}
                                progressBarMount={model.progressBarMount}
                            />
                        </Show>
                        {/* Cross-fades out on top of AgentPresentationView
                            once agentId() is set, instead of the two Shows
                            above hard-swapping instantly — see
                            pickerVisible/pickerFadingOut above.
                            SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.3. */}
                        <Show when={pickerVisible()}>
                            <div
                                class="agent-picker-host"
                                classList={{
                                    // Applied the instant agentId() is set
                                    // (same render as AgentPresentationView
                                    // appearing) so this never sits in
                                    // normal flow alongside it, even for
                                    // one frame.
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
                {/* No progressBarMount here — a history tab is a read-only
                    reader with no live turn/working state of its own, so
                    there's nothing for a progress bar to represent. */}
                <AgentHistoryTabView model={model} />
            </Show>
        </ModalLayer>
    );
};

AgentBlockContent.displayName = "AgentBlockContent";

/**
 * Chrome half of the agent pane — the header, tab strip and progress-bar
 * slot, rendered by the shared `renderPaneChromeShell` (pane-leaf-chrome.tsx's
 * fallback when a view supplies no `renderPaneChrome`) for EVERY agent pane
 * (see `pane-leaf-chrome.tsx`'s `hoisted` memo for why gating this on stack
 * size was a catch-22), wrapping whichever `AgentBlockContent` instance is
 * currently the active stack member's own switch-scoped `<Block>`
 * (`content` prop, supplied by `pane-leaf-chrome.tsx`). Unlike `AgentBlockContent`, this component is
 * constructed ONCE per leaf and stays mounted across every subsequent
 * switch — that persistence is the entire point of this file's split
 * (`SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`).
 *
 * `anchorBlockId` is the blockId of whichever stack member's `ViewModel`
 * FIRST called `renderPaneChrome` — frozen for this component's entire
 * lifetime, the same "one instance, one immutable blockId" contract every
 * other `ViewModel`/`NodeModel` consumer already follows. It is NOT used to
 * resolve the owning `LayoutNode`, though — ReAgent P0 on #3136:
 * `anchorBlockId` can itself be closed by the user (removed from
 * `blockStack` by `closeBlockInStack`'s `filter`), and chrome never unmounts
 * to recover — a `getNodeByBlockId(anchorBlockId)` lookup that outlives that
 * tab's closure would return `null` forever after, breaking the whole tab
 * strip. Node resolution uses `nodeModel.nodeId` instead
 * (`getOwnNode`, below) — the leaf's own id, stable regardless of which
 * stack members come and go.
 */
/**
 * Agent's opt-in to the ONE shared pane chrome
 * (`renderPaneChromeShell`) — see `PaneChromeModel` (custom.d.ts).
 * Everything the old `AgentPaneChrome` component rendered around the
 * content (root box, focus ring, header row, ErrorBoundary) is the shared
 * chrome's job now; what stays here is only what is genuinely agent's:
 * its tab model (fork lineage merged with this pane's own stack, rename,
 * per-pane zoom) and the below-header slot its turn-progress bar portals
 * into. That slot is a generic capability — any view type can take it.
 */
export function buildAgentPaneChromeModel(anchorBlockId: string, nodeModel: NodeModel): PaneChromeModel {
    // `nodeModel.layoutModel`, NOT `getLayoutModelForStaticTab()` — see that
    // field's own doc comment (layout/lib/types.ts) and
    // SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md: this pane's
    // own tab is not necessarily "whichever tab is globally active right
    // now" at the moment chrome first constructs (e.g. a brand-new tab's
    // default agent pane, seeded by `applyTabPreset` before `setActiveTab`
    // ever runs).
    const layoutModel = nodeModel.layoutModel;

    // ReAgent P0 on this PR: resolving the owning node via
    // layoutModel.getNodeByBlockId(anchorBlockId) — anchorBlockId's own
    // originating tab — breaks permanently the moment the user closes THAT
    // specific tab: closeBlockInStack removes a closed member from
    // blockStack via filter, so getNodeByBlockId(anchorBlockId) would
    // return null forever after (chrome never unmounts to recover once
    // mounted). The leaf's own nodeId
    // (NodeModel.nodeId) is stable regardless of which stack members come
    // and go — resolve on that instead, everywhere in this component.
    const getOwnNode = () => findNode(layoutModel.treeState.rootNode, nodeModel.nodeId);

    // Reads the SAME reactive field `pane-leaf-chrome.tsx`'s inner `<Key>`
    // is keyed on — see NodeModel.activeBlockId's own doc comment
    // (layout/lib/types.ts).
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? anchorBlockId;

    // Block-scoped reads (agentId) must track the
    // CURRENTLY ACTIVE member, not `anchorBlockId` (frozen to whichever
    // ViewModel instance first rendered this chrome) — getMuxObjectAtom
    // inside a memo, not useMuxObjectValue, the same reactive-oref pattern
    // PR #3134 already established for BlockFrame_Header
    // (frontend/app/store/mos.ts's own doc comments explain why).
    const activeBlockData = createMemo(() => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", activeBlockId()))());
    const agentId = () => activeBlockData()?.meta?.["agentId"];

    // Fork tabs: conversations sharing this one's `parent_id` lineage that
    // are open in ANOTHER top-level pane, shown as extra pills (PaneChrome
    // dedupes them against this pane's own stack) so cross-pane
    // fork-switching keeps working.
    const [openDefinitions] = useOpenDefinitionMap();
    // ReAgent P2: reads the active tab's OWN agentDefinitions
    // (agent-model.ts) instead of calling useAgentDefinitions() again here
    // — that would be a second, independent RPC + agents:changed
    // subscription for the same pane, on top of the one AgentBlockContent
    // already owns. Found by block id (agent-models.ts): the host's view
    // model is only an adapter once the agent is a native pane tab.
    const forks = useForkSet({
        definitions: () => agentModels.get(activeBlockId())?.agentDefinitions() ?? [],
        openBlockByDef: openDefinitions,
        activeDefinitionId: () => agentId() ?? "",
    });
    // Same "switchable" filter AgentPresentationView used to apply: a fork
    // with no open blockId anywhere can't be jumped to, so it isn't offered.
    const switchableForks = createMemo(() => forks().filter((f) => f.isActive || !!f.blockId));

    // Forks open in ANOTHER pane appear as extra pills after this pane's own
    // stack members, labeled with their branch/definition title.
    const extraTabs = () =>
        switchableForks()
            .filter((f) => !!f.blockId)
            .map((f) => ({ blockId: f.blockId!, label: f.title }));

    // Activating a tab has two cases, both "switch," neither "create": (1)
    // the target block already lives in THIS pane's own block-stack — swap
    // the active member in place; (2) a fork open as its own separate
    // top-level pane — jump focus to it via refocusNode, same as the
    // picker's "Switch to existing" flow already does.
    const handleTabSwitch = (targetBlockId: string) => {
        if (targetBlockId === activeBlockId()) return;
        const node = getOwnNode();
        if (!node) return;
        const stack = node.data?.blockStack?.length ? node.data.blockStack : [activeBlockId()];
        if (stack.includes(targetBlockId)) {
            // No reveal gate: reaching this branch at all requires
            // node.data.blockStack.length > 1 (otherwise `stack` above is
            // just [activeBlockId()], and targetBlockId !== activeBlockId()
            // already ruled out targetBlockId matching it) — the
            // precondition for a second pill to exist to click at all.
            // AgentPaneChrome is mounted for the whole pane's life and
            // never remounts on a switch, so there is nothing for a gate to
            // hide: only the inner <Block> rebuilds, and that is already
            // covered by its own ready-gate cross-fade.
            // See SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md.
            setActiveBlockInStack(layoutModel, node.id, targetBlockId);
        } else {
            refocusNode(targetBlockId);
        }
    };
    // × on a tab (also middle-click, via PaneTabStrip's onMouseDown).
    // closeAgentTab resolves the block's OWNING node — for a stack member
    // that's this pane, for a cross-pane fork tab it's that other pane — and
    // closes it like closeBlockInStack does (pop it out and activate the
    // neighbor; last member closes the pane), with one exception: the last
    // tab of THIS pane holding a loaded agent is swapped for a fresh My
    // Agents tab instead of closing the pane
    // (SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §2).
    //
    // No reveal gate on the ordinary close paths — same reasoning as
    // handleTabSwitch: that node's own AgentPaneChrome (if it's an agent
    // pane) is mounted for the pane's whole life and never remounts on a
    // switch, so a gate would only hide content already covered by its own
    // ready-gate cross-fade. See SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md.
    // The picker swap holds its own gate (see close-agent-tab.ts).
    const handleTabClose = (targetBlockId: string) => {
        void closeAgentTab({ layoutModel, ownNodeId: nodeModel.nodeId, blockId: targetBlockId });
    };
    // No progress-bar slot here: the shared chrome renders it on every pane
    // and hands it to whichever view model is active (PaneChrome.tsx), so an
    // agent tab in a pane that started as another view type gets it too.
    return {
        extraTabs,
        // Both return true ("handled"): an agent tab may live in a
        // DIFFERENT pane (a fork open as its own top-level pane), so
        // activating/closing it isn't necessarily this pane's own stack
        // operation — handleTabSwitch/handleTabClose resolve the owning
        // node themselves.
        onActivate: (id: string) => {
            handleTabSwitch(id);
            return true;
        },
        onClose: (id: string) => {
            handleTabClose(id);
            return true;
        },
        rootClass: "agent-pane-stack",
        contentClass: "agent-pane-stack-content",
    };
}


// Launch flow lives in `flows/launch-flow.ts` — Step 2 of
// docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.

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
            clearTermWrite: () => setTermWrite(null),
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
    // One readiness authority for this pane — see
    // docs/specs/SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md. Phase 1 changes no
    // behaviour: the same two conditions gate the reveal, the fade still runs for
    // 220ms, and the overlay still unmounts after it. What changes is that "may the
    // pane appear" is now ONE stated decision instead of several components each
    // deciding independently — a prerequisite for collapsing the four overlapping
    // loading indicators (two were measured on screen at once) in later phases.
    //
    // The phase maps onto exactly what the two old booleans encoded:
    //   assembling → covered, not yet fading   (was: !historyLoaded && showOverlay)
    //   revealing  → covered, fading            (was:  historyLoaded && showOverlay)
    //   live       → unmounted                  (was: !showOverlay)
    const readiness = createPaneReadiness({ label: `block:${model.blockId}` });
    const releaseHistoryGate = readiness.gate("history");
    const releaseAuthGate = readiness.gate("auth");
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
        onHistoryReady: () => {
            historyReadyFn?.();
            // A pane opens with K turns, not the load window's worth (§6.9).
            scheduleRollOff();
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
    // Report each dependency to the readiness controller as it completes, rather
    // than re-deriving "are we done yet" from a conjunction. Same two conditions,
    // same resulting moment — but now each one STATES that it is finished, so a
    // stuck reveal names the gate (`readiness.pendingGates()`) instead of being an
    // unexplained hang. Both releases are idempotent, so re-running this effect on
    // an unrelated signal change is harmless.
    createEffect(() => {
        if (historyPainted()) releaseHistoryGate();
    });
    createEffect(() => {
        if (authPhaseSettled()) releaseAuthGate();
    });
    // `revealing` → `live`: hold the overlay mounted for the fade's own duration
    // (matching PaneLoadingCover.scss's transition) before unmounting, so it fades
    // as one visual unit with the spinner instead of vanishing mid-transition.
    createEffect(() => {
        if (readiness.phase() === "revealing") {
            loadingOverlayFadeTimeout = setTimeout(() => readiness.revealComplete(), 220);
        }
    });

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
        if (paneModel.state.failure?.data.code === "auth") return "unauthenticated";
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
        const nodes = paneModel.document();
        const now = Date.now();
        setHasPromotedTool(hasRunningPromotedTool(nodes, now));
        const at = nextToolPromotionAt(nodes, now);
        if (at == null) return;
        const timer = setTimeout(() => setToolPromotionCheckNonce((n) => n + 1), Math.max(0, at - now) + 50);
        onCleanup(() => clearTimeout(timer));
    });

    // THE busy predicate — one meaning, three renderings: this row, the top
    // progress bar, and the composer strip. All three read this memo and
    // nothing else, so they cannot disagree. Definition and the reasoning for
    // collapsing them live in working-indicator.ts.
    //
    // This used to subtract workingRowSupersededByDock() here, standing the row
    // down once a tool call was promoted to the ActivityDock. That was wrong:
    // promotion is a DISPLAY change at TOOL_PROMOTION_MS, not the harness
    // backgrounding the call, so the turn is still blocked and input still
    // queues — the row was hiding a gate that was still closed, while the bar
    // (which never had the term) kept running. See
    // docs/reports/REPORT_AGENT_PANE_PROGRESS_INDICATORS_CONSOLIDATION_2026_09_09.md
    // §3.1 (still valid: mere dock PROMOTION never relaxes busy-ness).
    //
    // §2.3a (2026-09-17, supersedes §2.3) DOES relax busy-ness, but only for
    // genuinely accepted background work (isAcceptedBackgroundLaunch), not
    // mere promotion — see hasBlockingForegroundToolCall's doc comment in
    // ./activity/tool-adapter for exactly how those two are told apart.
    const paneBusy = createMemo(() =>
        paneBusyForInput({
            showingLaunchActivity: showingLaunchActivity(),
            turnPhase: paneModel.state.turnPhase,
            compacting: paneModel.state.compacting,
            reconnecting: paneModel.state.reconnecting,
            hasAttachedBackgroundWork:
                paneModel.state.attachedTask != null || paneModel.state.registryAttachedTaskSince != null,
            hasBlockingForegroundToolCall: hasBlockingForegroundToolCall(paneModel.document()),
        })
    );

    const workingRowLoading = paneBusy;
    const workingRowVisible = createMemo(
        () =>
            workingRowLoading() ||
            paneModel.state.sessionStats != null ||
            paneModel.state.compacting != null ||
            paneModel.state.reconnecting != null,
    );

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
        const nodes = paneModel.document();
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
        const registryStartMs = paneModel.state.registryAttachedTaskSince;
        const startMs =
            transcriptStartMs != null && registryStartMs != null
                ? Math.min(transcriptStartMs, registryStartMs)
                : (transcriptStartMs ?? registryStartMs);
        const current = paneModel.state.attachedTask != null;
        if ((startMs != null) !== current) {
            paneModel.dispatchPane(
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
    const handleSendMessage = (message: string): Promise<void> => {
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
        const authFailureToPreserve = liveFailure?.data.code === "auth" ? liveFailure : null;
        // Only start a NEW turn when the agent is idle. Dispatching TurnStart
        // while a turn is already running regresses Streaming → Submitting,
        // which would flicker the busy indicator back to its "Submitting"
        // look for no reason. A queued-while-busy message rides the running
        // turn; the queue-drain (agent-message-accepted) re-enters Submitting
        // if needed.
        if (!wasAlreadyWorking) {
            paneModel.dispatchPane({ type: "TurnStart", at: Date.now(), content: message }, "user");
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
    // Pre-launch auth failure → the SAME failure row every other auth failure
    // uses, instead of the separate blue "Log in" bar this replaced
    // (docs/specs/PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md).
    //
    // The DECISION lives in decideSyntheticRow (failure/synthetic-row.ts) and
    // is unit-tested there; this is wiring only. It was extracted after this
    // effect produced several P1s across PR #2951 — it sits inline in the pane
    // component and no existing harness can reach it, so the logic was
    // unassertable while it lived here.
    //
    // Tracks BOTH canRetry and the failure signal. Tracking the failure is
    // what lets a dismissed REAL failure fall back to this row while the agent
    // is still unauthenticated (reagent P1) — without it the pane kept no login
    // affordance at all, strictly worse than the undismissable bar it replaced.
    // Dismissing THIS row still sticks; decideSyntheticRow tells the two apart
    // from the previous value, which is why that state is threaded here.
    let prevFailure: PaneFailure | null = null;
    let syntheticDismissed = false;
    createEffect(() => {
        const decision = decideSyntheticRow({
            canRetry: status.canRetry(),
            current: paneModel.state.failure,
            previous: prevFailure,
            syntheticDismissed,
        });
        prevFailure = untrack(() => paneModel.state.failure);
        syntheticDismissed = decision.syntheticDismissed;
        if (decision.action === "raise") {
            paneModel.dispatchPane(
                {
                    type: "FailureObserved",
                    at: Date.now(),
                    turnAttempted: false,
                    failure: {
                        code: "auth",
                        title: "Not signed in",
                        detail:
                            "This agent hasn't been signed in to its provider yet, so it never started. " +
                            "Sign in to launch it — nothing has run, so there's no turn to retry.",
                        retryable: true,
                    },
                },
                "system",
            );
            // Keep prevFailure in step with what we just dispatched, so the
            // re-run this write triggers sees "our row is showing" rather than
            // "a row just appeared from nowhere".
            prevFailure = untrack(() => paneModel.state.failure);
        } else if (decision.action === "retract") {
            paneModel.dispatchPane({ type: "FailureCleared" }, "system");
            prevFailure = null;
        }
    });

    // ---- Bind-account (SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md §2/§3) ----
    //
    // Cross-pane/cross-tab live account cache — same subscription pattern
    // `AgentLaunchModal.tsx` already uses. Seeded synchronously (no network
    // round trip; the app-wide cache is already warm) then kept live.
    const [accountCache, setAccountCache] = createSignal<Account[]>(loadAccounts());
    createEffect(() => {
        const unsub = subscribeAccountChanges((list) => setAccountCache(list));
        onCleanup(unsub);
    });

    // This agent's own currently-linked account for its provider, if any —
    // excluded from bind candidates (nothing to adopt). Refreshed on mount
    // and whenever this agent's identity links change (below) — the same
    // event that also drives the auto-unblock check, since both concerns
    // become stale for the identical reason.
    const [linkedAccountId, setLinkedAccountId] = createSignal<string | undefined>(undefined);
    const refreshLinkedAccountId = async () => {
        const agentDefinitionId = getBlockMetaKeyAtom(model.blockId, "agentId")() as string | undefined;
        const prov = provider();
        if (!agentDefinitionId || !prov) {
            setLinkedAccountId(undefined);
            return;
        }
        try {
            const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agentDefinitionId });
            setLinkedAccountId(lastLinkedAccountId(links, prov.id));
        } catch {
            setLinkedAccountId(undefined);
        }
    };
    // Re-resolve whenever the agent id or provider resolves or changes — both
    // come from block meta, which may not have loaded on the first run, and a
    // one-shot mount-time call left the link (and the chip's email) unset.
    createEffect(
        on(
            () => [getBlockMetaKeyAtom(model.blockId, "agentId")(), provider()?.id] as const,
            () => void refreshLinkedAccountId()
        )
    );

    // The bound account's login email for the composer's sign-in chip
    // (SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md) — live across logins and
    // rebinds via accountCache + linkedAccountId.
    const authEmail = createMemo(() => boundAccountEmail(accountCache(), linkedAccountId()));

    const bindCandidates = createMemo(() => {
        const prov = provider();
        if (!prov) return [];
        return computeAccountBindCandidates(prov.id, accountCache(), linkedAccountId());
    });

    // A flat picker of accounts to bind, anchored at the click — same
    // ContextMenuModel primitive the Armory's Bind-to-Agent menu uses
    // (SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md), just a flat
    // list here (the trigger IS the button — no outer submenu to nest under).
    // What to do for a given candidate list is `planBind` (failure/account-picker.ts,
    // unit-tested): the failure row's "adopt" binds a lone candidate at once; the
    // composer chip's "switch" never does — the agent works and a switch restarts
    // it, so the user picks a named destination
    // (SPEC_COMPOSER_ACCOUNT_SWITCH_AND_JEKT_HEIGHT_CAP_2026_09_26.md Part A).
    const runBind = (mode: BindMode, e: MouseEvent | undefined, labelPrefix = "") => {
        const plan = planBind(mode, bindCandidates());
        if (plan.kind === "none") return;
        if (plan.kind === "bind") {
            void status.bindExistingAccount(plan.account);
            return;
        }
        // No event to anchor on (shouldn't happen — PaneRow's render call site
        // always passes one) → no-op rather than guessing a position.
        if (!e) return;
        ContextMenuModel.showContextMenu(
            accountPickerItems(plan.candidates, (acct) => void status.bindExistingAccount(acct), labelPrefix),
            e,
        );
    };
    const onBindAccount = (e?: MouseEvent) => runBind("adopt", e);
    const onSwitchAccount = (e: MouseEvent) => runBind("switch", e, "Switch to ");

    // Declare the auth-blocking state resolved: clears canRetry/authNotice
    // (notifyControllerHealthy) and, ONLY when the live failure is actually
    // an auth failure, clears it too — never unconditionally, so an
    // unrelated concurrent failure (rate_limited, context_exceeded, …) that
    // happens to be showing isn't silently wiped. Shared by two independent
    // proofs of health: a live controllerstatus event showing an active turn
    // (below), and a verified auth re-check after an external bind (below).
    const declareAuthHealthy = () => {
        status.notifyControllerHealthy();
        if (paneSnapshot(model.blockId)?.failure?.data.code === "auth") {
            paneModel.dispatchPane({ type: "FailureCleared" }, "system");
        }
    };

    // Bounded retry around recheckAuthAfterBind — NOT a stylistic choice, a
    // correctness fix. `agentidentities:changed` is published by the
    // backend SYNCHRONOUSLY inside the `LinkAgentIdentityCommand` handler,
    // before it even responds to the RPC (agent_handlers/identity.rs:590-611);
    // RPC responses and WS events share one in-order connection, so this
    // pane's subscription below fires before `bindAccountToAgent`'s own
    // `SetMetaCommand` — which only runs AFTER that same Link RPC resolves
    // client-side — has refreshed `cmd:env` to the newly-bound account's
    // dir. The very first recheck therefore reads STALE env and fails on
    // essentially every bind, not as an edge case but as the common case —
    // reagentx P1 on PR #2969. Retry ladder lives in recheck-after-bind.ts,
    // unit-tested there (dependency-injected, no DOM/RPC mocking needed) —
    // kept out of this file for the same reason
    // PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md's retrospective
    // extracted decideSyntheticRow out of here after several P1s: inline
    // logic in this component is unassertable by any existing harness.
    const recheckAuthAfterBindWithRetry = () =>
        retryRecheckAfterBind({
            recheck: status.recheckAuthAfterBind,
            stillBlocked: () => status.canRetry() || paneSnapshot(model.blockId)?.failure?.data.code === "auth",
            sleep,
            onHealthy: declareAuthHealthy,
        });

    // Auto-unblock: a bind can happen from ANYWHERE (the Armory's
    // Bind-to-Agent menu, the per-agent Identity tab, or this pane's own
    // "Bind account" above) — this pane must notice regardless of source.
    // `agentidentities:changed:<agentId>` already fires on every one of
    // those; nothing previously listened for it here.
    //
    // Re-verifies via CheckCliAuth before declaring healthy (a bind event is
    // not itself proof the new credential works) and NEVER auto-retries a
    // turn — see recheckAuthAfterBind's and declareAuthHealthy's own doc
    // comments. SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md §2.
    createEffect(() => {
        const agentDefinitionId = getBlockMetaKeyAtom(model.blockId, "agentId")() as string | undefined;
        if (!agentDefinitionId) return;
        const unsub = muxEventSubscribe({
            eventType: `agentidentities:changed:${agentDefinitionId}`,
            handler: () => {
                void refreshLinkedAccountId();
                const blocked = status.canRetry() || paneSnapshot(model.blockId)?.failure?.data.code === "auth";
                if (!blocked) return;
                void recheckAuthAfterBindWithRetry();
            },
        });
        onCleanup(unsub);
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
            // (AgentStartupModal, Armory → Bundles content), its
            // `instructions` take precedence over the legacy freeform
            // "startup" blob — which has no live authoring UI anywhere, see
            // docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md §5. Falls back to
            // the freeform blob when no bundle is selected (or it no longer
            // resolves, e.g. deleted), preserving any seed-manifest content.
            const startupBundleId = startupBundleIdResult?.content?.trim() || null;
            const startupBundle = startupBundleId
                ? await RpcApi.GetBundleCommand(TabRpcClient, { id: startupBundleId }).catch(() => null)
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
        // directly).
        <AgentDormancyProvider dormant={hidden}>
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
            {/* Stash drawer — top-anchored, directly under the pane header
                where its own backpack toggle lives
                (SPEC_AGENT_STASH_PANE_MIGRATION_2026_09_22.md §3.1).
                Replaced the former `agent-stash` MODAL; the header icon
                (agent-model.ts's endIconButtons) drives `stashOpen` through
                the three callbacks wired in onMount above.

                Deliberately OUTSIDE `.agent-view-zoomed`, exactly like the
                Shell drawer below, for two reasons beyond symmetry: the
                drag-to-resize math reads `ev.clientY` (visual px) and writes
                a `height` (layout px), which CSS `zoom` makes disagree —
                the same coordinate-space trap
                SPEC_AGENT_SHELL_DRAWER_ZOOM_COORDINATE_SPACE_2026_09_20.md
                records for the shell — and §3.2a's composer-scale density
                values are already tuned small, so compounding them with a
                per-pane zoom would read as either unusable or enormous
                rather than merely scaled. It stays a flex child of
                `.agent-view` so the transcript below still shrinks to make
                room for it. */}
            <Show when={paneModel.state.stashOpen}>
                {/* Rendered as a DIRECT flex child of `.agent-view`, with no
                    wrapper div, and that placement is load-bearing rather
                    than incidental (reagentx P1 on PR #3540). The 50% height
                    cap lives on `.agent-stash-drawer-resizable` — the same
                    element that holds BOTH the content body and the resize
                    handle — and a percentage `max-height` only resolves
                    against a containing block whose height is definite.
                    `.agent-view` is `height: 100%` (agent-view.scss), so it
                    qualifies; an intermediate auto-height wrapper would NOT,
                    and the percentage would compute to `none`. The first cut
                    had exactly that wrapper, which let the inner element
                    render at its full dragged height while the wrapper
                    clipped it — carrying the bottom-edge handle into the
                    clipped-away region, where it was invisible and
                    unreachable, so a drawer dragged past 50% could never be
                    shrunk again. See _stash-drawer.scss for the flex
                    compression that keeps the handle on screen instead. */}
                <ResizableDetailsDrawer
                    blockId={model.blockId}
                    anchor="top"
                    classPrefix="agent-stash-drawer"
                    persistMetaKey="agent:stashheight"
                    persistedHeight={block()?.meta?.["agent:stashheight"] as number | undefined}
                >
                    <AgentStashModal
                        agentId={agentId}
                        agentName={agentName()}
                        // Prefer cmd:cwd (the actual launch cwd, set by
                        // launchAgentDefinition) over
                        // AgentDefinition.working_directory, which is often
                        // empty or a stale default for template-launched and
                        // continuation agents.
                        workingDirectory={
                            (block()?.meta?.["cmd:cwd"] as string) ||
                            currentAgent()?.working_directory ||
                            ""
                        }
                        // No loadable definition (quick-launch pane) → default
                        // to the Memory tab; the Accounts tab works from
                        // agentId alone but Memory is the more useful default
                        // for a pane with no saved definition yet.
                        initialTab={currentAgent() ? "accounts" : "memory"}
                        // No `onClose` — closing is the header icon's job, so
                        // the Memory tab hides its footer Close button rather
                        // than rendering a dead one (§3.4).
                    />
                </ResizableDetailsDrawer>
            </Show>
            <div class="agent-view-zoomed" style={{ zoom: zoomFactor() }}>
            {/* Gradient progress bar — marching-ants shimmer traced around
                the full pane perimeter while working, hidden at rest.
                Color matches the pane's own selection-ring color (not a
                fixed --accent-color) via --progress-bar-color, set on
                .agent-pane-stack (agent-view.scss). Portaled into a
                slot AgentPaneChrome owns, between the tab strip and the
                content (its own row, never overlapping either), bridged
                through this AgentViewModel instance's progressBarMount
                signal — see that field's own doc comment (agent-model.ts)
                for why chrome and content, now separate component trees,
                need that indirection. This component's state (turnPhase,
                launch activity) is what drives the bar, but .agent-view
                (this component's own root, nested inside .agent-pane-stack-content,
                itself BELOW the tab strip in DOM order) can't reach a
                position above the tab strip through CSS alone; every
                ancestor between here and there clips overflow before an
                absolutely-positioned escape could ever become visible. See
                SPEC_AGENT_PANE_STATUS_GRADIENT_2026_06_14.md §4 and
                SPEC_AGENT_PANE_PROGRESS_BAR_ABOVE_TAB_STRIP_2026_08_10.md.
                Renders nothing until the slot ref is assigned (one frame,
                first mount only). */}
            <Show when={progressBarMount()}>
                <Portal mount={progressBarMount()!}>
                    <div
                        class="agent-pane-progress-bar"
                        classList={{
                            "agent-pane-progress-bar--active": paneBusy(),
                            "agent-pane-progress-bar--stopping":
                                paneModel.state.turnPhase.kind === "Interrupting",
                        }}
                        role="progressbar"
                        aria-label="Agent working"
                        aria-valuemin={0}
                        aria-valuemax={100}
                    />
                </Portal>
            </Show>
            <DragOverlay message={dropAttach.dropMessage()} visible={dropAttach.isDragOver()} />
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
                switchAccountCandidates={bindCandidates().map((a) => ({ id: a.id, name: accountLabel(a) }))}
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
            {/* Details panel — just the shell + control bar now. Activity-log
                lines write directly into the terminal (handleShellTermReady)
                instead of a separate panel here. Docked BELOW the composer
                (SPEC_AGENT_SHELL_BELOW_COMPOSER_2026_08_08.md): the shell
                stacks under the text input (which shifts up to make room,
                since this region hugs the pane bottom).

                Deliberately OUTSIDE `.agent-view-zoomed` — see the note on the root
                element. The terminal has to render at a 1:1 device-pixel ratio, so it
                must not be inside the per-pane `zoom`. It stays a flex child of
                `.agent-view` so the composer still shifts up to make room for it.
                Same move `agent-view.scss:350` records for the progress bar, and the
                same cure as SPEC_STATUS_BAR_POPOVER_DOUBLE_ZOOM_OFFSET_2026_08_22.md. */}
            <Show when={paneModel.state.detailsOpen}>
                    <div class="agent-composer-details" id={`agent-composer-details-${model.blockId}`}>
                        {/* One line: what this shell is, and what the agent
                            has left running. Takes the slot AgentControlBar
                            used to occupy with session UI — see
                            SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §4. */}
                        <AgentShellInfoPanel
                            blockId={model.blockId}
                            shellSubBlockId={block()?.meta?.["term:shellsubblockid"] as string | undefined}
                            cwd={block()?.meta?.["cmd:cwd"] as string | undefined}
                        />
                        {/* Drag-to-height drawer wrapping the terminal — the actual
                            scrollable/resizable content. */}
                        <ResizableDetailsDrawer
                            blockId={model.blockId}
                            persistedHeight={block()?.meta?.["term:shellheight"] as number | undefined}
                            defaultHeight={SHELL_DRAWER_DEFAULT_HEIGHT}
                        >
                            {/* Phase 0 spike (SPEC_AGENT_SHELL_XTERM_TERMINAL_2026_07_03.md):
                                real xterm+PTY terminal, spawned lazily on first
                                drawer open via a headless term sub-block. */}
                            <AgentShellSubblock
                                parentBlockId={model.blockId}
                                cwd={block()?.meta?.["cmd:cwd"] ?? ""}
                                existingSubBlockId={block()?.meta?.["term:shellsubblockid"] as string | undefined}
                                // No `agentPaneZoom` prop any more. The shell used to
                                // divide the pane's zoom out of its own font-size math
                                // to fake independence; now it genuinely IS independent,
                                // because it renders outside `.agent-view-zoomed`.
                                onSubBlockCreated={(subBlockId) => {
                                    void RpcApi.SetMetaCommand(TabRpcClient, {
                                        oref: MOS.makeORef("block", model.blockId),
                                        meta: { "term:shellsubblockid": subBlockId } as any,
                                    });
                                }}
                                onTermReady={handleShellTermReady}
                                onTermDispose={handleShellTermDispose}
                                onShellExited={handleShellExited}
                            />
                        </ResizableDetailsDrawer>
                    </div>
            </Show>
        </div>
        </AgentDormancyProvider>
    );
};

AgentPresentationView.displayName = "AgentPresentationView";
