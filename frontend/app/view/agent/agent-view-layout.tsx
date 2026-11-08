// Copyright 2024, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an agent pane draws: the cover and shutdown log, the zoomed column of transcript, panels,
 * strip and composer, and the shell drawer beside it. Split out of AgentPresentationView; each part
 * reads the hook groups the view built.
 */

import { resolveContextMenuRegion } from "@/app/block/context-menu-region";
import { PaneLoadingCover } from "@/app/element/PaneLoadingCover";
import { isStopping } from "@/app/store/agent-pane-state/types";
import { showCopyContextMenu } from "@/app/store/contextmenu";
import { isContainerAgentMode } from "@/util/runtime-kind";
import { createSignal, Show, type Accessor, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { viewChipLabel } from "./agent-view-controls";
import type { PaneFoundation } from "./agent-view-foundation";
import type { useFailureHandling } from "./agent-view-failure";
import type { PaneLate } from "./agent-view-late";
import type { usePaneInput } from "./agent-view-input";
import type { useSending } from "./agent-view-sending";
import type { useTurnStreams } from "./agent-view-streams";
import type { HistoryAndFeed, useShownDocument } from "./agent-view-transcript";
import type { TurnAndLaunch } from "./agent-view-turn";
import { AgentPaneProviders } from "./agent-media";
import { AgentBottomPanels } from "./components/AgentBottomPanels";
import { AgentComposerStrip } from "./components/AgentComposerStrip";
import { AgentComposerRegion, AgentTranscriptRegion, AgentViewPopoverHost } from "./components/AgentPaneRegions";
import { AgentPreparingRow } from "./components/AgentPreparingRow";
import { AgentProgressBar } from "./components/AgentProgressBar";
import { AgentFindBandItem } from "./components/AgentFindBandItem";
import { AgentStatsBandItem } from "./components/AgentStatsBandItem";
import { AgentShellDrawer } from "./components/AgentShellDrawer";
import { BtwOverlay } from "./components/BtwOverlay";
import { createMessagesPanelControls, MessagesPanelContext } from "./messages/messages-state";
import { MessagesPanel } from "./messages/MessagesPanel";
import { openOrFocusHistoryTab } from "./open-history-tab";
import { AgentTopBand } from "./top-band/AgentTopBand";
import { ShutdownOverlay } from "./shutdown/ShutdownOverlay";
import { isForcedRow } from "./virtualization/verbosity";

/** The hook groups AgentPresentationView built, handed to the parts that draw them. */
export interface AgentViewParts {
    model: AgentViewModel;
    agentId: string;
    /** Where the progress bar portals: a node AgentPaneChrome owns, bridged through the model (`progressBarMount` in agent-model.ts). */
    progressBarMount: () => HTMLDivElement | undefined;
    found: PaneFoundation;
    feed: HistoryAndFeed;
    shown: ReturnType<typeof useShownDocument>;
    turn: TurnAndLaunch;
    streams: ReturnType<typeof useTurnStreams>;
    sending: ReturnType<typeof useSending>;
    failure: ReturnType<typeof useFailureHandling>;
    input: ReturnType<typeof usePaneInput>;
    late: PaneLate;
}

export function AgentViewLayout({ v }: { v: AgentViewParts }): JSX.Element {
    const { model, found, feed, input, late } = v;
    // The Messages panel's state and what the conversation's rows need from it (messages/messages-state.ts).
    const messages = createMessagesPanelControls(model.blockId, isForcedRow);
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
        <AgentPaneProviders dormant={found.renderDormant} block={found.block} agent={found.currentAgent} loadToolResult={feed.liveFeed.loadToolResult}>
            {/* The pane-scope ModalLayer is in AgentBlockContent, this component's parent: it covers this view and the picker. */}
        <div
            ref={(el) => (late.root = el)}
            class="agent-view agent-view--presentation"
            // `zoom` lives on `.agent-view-zoomed` below, not here: the Shell drawer must render unscaled. xterm
            // measures cells in visual px and reclaims width in layout px, and CSS `zoom` makes them disagree
            // (fractional cells, a misplaced link layer; xtermjs/xterm.js #2584). `--agent-pane-zoom` is published
            // here for other consumers (SPEC_AGENT_SHELL_DRAWER_ZOOM_COORDINATE_SPACE_2026_09_20.md).
            style={{ "--agent-pane-zoom": String(input.zoomFactor()) }}
            onContextMenu={handleContextMenu}
            tabIndex={-1}
        >
            {/* Covers the pane from mount until the initial history load resolves. A direct child of `.agent-view`,
                outside `.agent-view-zoomed`: it is `position: absolute; inset: 0`, so inside the zoomed wrapper it
                would not cover the Shell drawer (a sibling), and the pane zoom would scale it
                (REPORT_AGENT_PANE_LOADING_UI_2026_09_20.md §F). */}
            <PaneLoadingCover phase={feed.readiness.phase} />
            {/* Shutdown log while the pane closes in place (SPEC_AGENT_SELF_QUIT_2026_09_24.md §5.5). */}
            <ShutdownOverlay blockId={model.blockId} agentName={found.agentName()} />
            {/* The conversation and, beside it, the Messages panel (a sibling of the zoomed column, so the
                transcript's bottom panels stay direct children of it). */}
            <MessagesPanelContext.Provider value={messages}>
                {/* The top band: what the pane puts across its top (the find bar). Above the row, so the Messages
                    panel, which lies over the conversation, never covers it. */}
                <AgentTopBand blockId={model.blockId} zoom={input.zoomFactor} />
                <div class="agent-view-row">
                    <div class="agent-view-zoomed" style={{ zoom: input.zoomFactor() }}>
                        <AgentViewOverlays v={v} />
                        <AgentViewTranscript v={v} />
                        <AgentViewBottom v={v} />
                    </div>
                    <MessagesPanel
                        blockId={model.blockId}
                        controls={messages}
                        nodes={found.paneModel.document}
                        jumpTo={input.scroll.jumpTo}
                        zoom={input.zoomFactor()}
                    />
                </div>
            </MessagesPanelContext.Provider>
            {/* The shell drawer: outside `.agent-view-zoomed`, a flex child of `.agent-view`. */}
            <AgentShellDrawer
                open={found.paneModel.state.detailsOpen}
                blockId={model.blockId}
                shellSubBlockId={found.meta.shellSubBlockId()}
                cwd={found.meta.cwd() || undefined}
                persistedHeight={found.meta.shellHeight()}
                onTermReady={found.shell.onTermReady}
                onTermDispose={found.shell.onTermDispose}
                onShellExited={found.handleShellExited}
            />
        </div>
        </AgentPaneProviders>
    );
}

/** The progress bar, the /btw overlay and the find bar's place in the top band. */
function AgentViewOverlays({ v }: { v: AgentViewParts }): JSX.Element {
    const { model, input, sending } = v;
    return (
        <>
            {/* Portaled above the tab strip; see components/AgentProgressBar.tsx. */}
            <AgentProgressBar
                mount={v.progressBarMount}
                active={sending.paneBusy()}
                stopping={isStopping(v.found.paneModel.state.turnPhase)}
            />
            {/* /btw side-question overlay: ephemeral, floats over the whole pane (styles/_btw.scss), not gated on
                the turn state, so it stays usable while a turn streams. */}
            <Show when={sending.commands.btwOverlay()}>
                {(state) => (
                    <BtwOverlay
                        blockId={model.blockId}
                        askId={state().askId}
                        question={state().question}
                        requestId={state().requestId}
                        error={state().error}
                        onClose={sending.commands.closeBtw}
                    />
                )}
            </Show>
            <AgentStatsBandItem blockId={model.blockId} />
            <AgentFindBandItem
                blockId={model.blockId}
                visible={input.search.visible}
                onSearch={input.search.performSearch}
                onNext={input.search.next}
                onPrev={input.search.prev}
                onClose={input.search.close}
                matchIndex={input.search.currentIndex}
                matchCount={input.search.matchCount}
                searching={input.search.searching}
            />
        </>
    );
}

/** The scroll region: `.agent-document` inside AgentDocumentView is absolutely positioned to fill it. The
 *  working row is a sibling below the dock, not an overlay on this box
 *  (SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md). */
function AgentViewTranscript({ v }: { v: AgentViewParts }): JSX.Element {
    const { model, agentId, found, feed, shown, input, late } = v;
    const { history, liveFeed } = feed;
    const { status } = v.turn;
    // The list's "content grew while the reader is scrolled up" state, for the New activity pill.
    const [newActivitySource, setNewActivitySource] = createSignal<Accessor<boolean>>(() => false);
    return (
        <AgentTranscriptRegion
            readiness={feed.readiness}
            documentNodes={shown.shownDocument}
            documentStateAtom={found.agentAtoms().documentStateAtom}
            onOpenHistory={() => void openOrFocusHistoryTab({ currentBlockId: model.blockId, agentId })}
            onAgentErrorLogin={() => {
                // The button says "Login Again", so it forces a fresh OAuth for every provider, like the failure
                // row's. Seeding from the personal credential here would reuse a possibly equally stale token
                // (retro-agent-auth-relogin-noop-2026-07-01).
                found.shell.log("auth", "Login Again (inline error node) — forcing a fresh provider login");
                void status.relogin();
            }}
            onLoadOlder={history.loadOlder}
            loadingOlder={history.loadingOlder}
            // The one paging gate: the list pages only while this is true.
            hasOlderHistory={() => liveFeed.canPageOlder() && history.historyOffset() > 0}
            followingRef={(f) => liveFeed.setFollowingBottom(f)}
            newActivityRef={(a) => setNewActivitySource(() => a)}
            scrollCommand={input.scroll.command}
            scrollToBottomRef={(fn) => {
                late.scrollToBottom = fn;
            }}
            highlightNodeId={input.search.highlightId}
            registerHistoryReadyCallback={feed.registerHistoryReady}
            zoomFactor={input.zoomFactor}
            blockId={model.blockId}
            layoutView={found.layoutView}
            newActivityVisible={() => newActivitySource()()}
            onJumpToNewActivity={() => late.scrollToBottom?.("new-activity-pill")}
        />
    );
}

/** Below the transcript: the panels, the preparing row, the composer strip and the composer. */
function AgentViewBottom({ v }: { v: AgentViewParts }): JSX.Element {
    const { model, agentId, found, shown, turn, streams, sending, failure, late } = v;
    const { paneModel, block, provider, providerKey, meta, context } = found;
    const { commands, paneBusy } = sending;
    const { account } = failure;
    return (
        <>
            {/* Login, decisions, questions, queue, recovery banners, activity dock,
                working row and session notices (components/AgentBottomPanels.tsx). */}
            <AgentBottomPanels
                blockId={model.blockId}
                agentId={agentId}
                agentName={found.agentName()}
                authProviderId={provider()?.id ?? providerKey()}
                providerId={provider()?.id ?? ""}
                block={block}
                paneModel={paneModel}
                status={turn.status}
                log={found.shell.log}
                pendingDecisions={turn.pendingDecisions}
                onDecide={turn.handleDecide}
                pendingQuestions={turn.pendingQuestions}
                onAnswer={turn.handleAnswer}
                onCancel={turn.handleCancel}
                pendingMessages={streams.pendingMessages}
                failureRow={failure.failureUI.row}
                backgroundTasksAtom={streams.backgroundTasksAtom}
                workingRowVisible={sending.workingRowVisible}
                workingRowLoading={paneBusy}
                onStop={commands.stopAgent}
                offline={failure.offline}
            />

            {/* A container agent's container starting in the background (preparing.ts). */}
            <AgentPreparingRow nodes={shown.displayDocument} enabled={meta.isContainer} />

            {/* The composer status strip: the activity ticker and the Log button (`detailsOpen` is reducer-owned). */}
            <AgentComposerStrip
                sessionTotals={paneModel.state.sessionTotals}
                loading={paneBusy()}
                logOpen={paneModel.state.detailsOpen}
                onToggleLog={found.shell.withShellFocus(() => paneModel.dispatchPane({ type: "DetailsToggle" }, "user"))}
                contextTokens={context.reading()?.tokens ?? null}
                contextWindow={context.reading()?.window ?? undefined}
                contextNote={context.note()}
                autoCompact={context.autoCompact()}
                lastReplyModel={paneModel.state.lastContextModel}
                authStatus={turn.loginStatus()}
                authEmail={account.authEmail()}
                signInCheckedLater={isContainerAgentMode(block()?.meta?.["agentMode"])}
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
                viewChip={viewChipLabel(shown.viewControls.resolved())}
                viewOpen={shown.viewControls.popoverOpen()}
                onOpenView={() => shown.viewControls.setPopoverOpen(!shown.viewControls.popoverOpen())}
            />
            {/* Hangs from the chip, or from the strip while there is none (opened from the menu or the palette). Keyed by
                which: the panel reads its anchor once, and a change of view can remove the chip under it. */}
            <AgentViewPopoverHost controls={shown.viewControls} rootEl={() => late.root} />

            <AgentComposerRegion
                commands={commands}
                offline={failure.offline}
                model={model}
                agentName={found.agentName}
                onTyping={() => late.scrollToBottom?.("typing")}
                isComposerEmptyRef={(fn) => (late.composerIsEmpty = fn)}
            />
        </>
    );
}
