// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createMemo, onMount, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { getProvider } from "./providers";
import { createAgentAtoms } from "./state";
import type { SubagentLinkNode } from "./types";
import { openSubagentPane, isSubagentPaneOpen } from "@/app/store/subagent-pane-manager";
import { useAgentStream } from "./useAgentStream";
import { useLaunchLogs } from "./hooks/useLaunchLogs";
import { useSessionDigest } from "./hooks/useSessionDigest";
import { useHistoryPagination } from "./hooks/useHistoryPagination";
import { useAgentControllerStatus } from "./hooks/useAgentControllerStatus";
import { useInSessionSearch } from "./hooks/useInSessionSearch";
import { useBookmarks } from "./hooks/useBookmarks";
import { useScrollToNode } from "./hooks/useScrollToNode";
import { useAgentKeyboard } from "./hooks/useAgentKeyboard";
import { useSubagentEvents } from "./hooks/useSubagentEvents";
import { useControllerStatusEvents } from "./hooks/useControllerStatusEvents";
import { useAgentCommands } from "./hooks/useAgentCommands";
import { AgentControlBar } from "./components/AgentControlBar";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter } from "./components/AgentFooter";
import { AgentPicker } from "./components/AgentPicker";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { SlashCommandPicker } from "./components/SlashCommandPicker";
import { SlashHelpPanel } from "./components/SlashHelpPanel";
import { BookmarksPanel } from "./components/BookmarksPanel";
import { SessionDigestBanner } from "./components/SessionDigestBanner";
import { ContextMenuModel } from "@/app/store/contextmenu";
import "./agent-view.scss";

/**
 * Top-level wrapper — switches between agent picker and presentation view.
 */
export const AgentViewWrapper = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const block = model.blockAtom;
    const agentId = () => block()?.meta?.["agentId"];

    return (
        <Show
            when={agentId()}
            fallback={<AgentPicker model={model} />}
        >
            <AgentPresentationView model={model} agentId={agentId()} />
        </Show>
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

    const agentAtoms = createMemo(() => createAgentAtoms(model.blockId));

    // Log buffer — the LogFn is passed down to every hook that needs it.
    const { lines: logLines, append: log } = useLaunchLogs();

    // History pagination: owns the document slice, loadingOlder state,
    // loadOlder handler, and documentVersion (bumped on every external
    // document mutation so useAgentStream can rebuild its dedup index).
    const history = useHistoryPagination({
        blockId: model.blockId,
        documentAtom: agentAtoms().documentAtom,
        outputFormat,
        log,
    });

    // Session digest banner state + auto-trigger.
    const digest = useSessionDigest({ blockId: model.blockId, block, log });

    // Auth + launch flow state and the onCleanup that kills the CLI
    // if the pane closes mid-login.
    const status = useAgentControllerStatus({ blockId: model.blockId, provider, log });

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
        documentAtom: agentAtoms().documentAtom,
        log,
    });

    // Subscribe to subprocess output and parse into DocumentNodes.
    // `documentVersion` is bumped whenever we mutate the document externally
    // (history load / prepend), causing useAgentStream to rebuild its
    // nodeIdSet and nodeIndexMap.
    useAgentStream({
        blockId: model.blockId,
        outputFormat: outputFormat(),
        documentAtom: agentAtoms().documentAtom,
        streamingStateAtom: agentAtoms().streamingStateAtom,
        enabled: true,
        documentVersion: history.documentVersion,
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
    });
    const handleSendMessage = commands.sendMessage;

    // ── Jump-to-node + Bookmarks ────────────────────────────────────────────────

    // Signal-based jump command. AgentDocumentView reacts via a
    // createEffect and scrolls inside its own container — no mutable
    // refs crossing component boundaries. See hooks/useScrollToNode.ts.
    const scroll = useScrollToNode();

    // Bookmarks: list, derived id set, panel visibility, CRUD callbacks.
    const bookmarks = useBookmarks({
        blockId: model.blockId,
        block,
        log,
        jumpTo: scroll.jumpTo,
    });

    // In-session search: matches, navigation, highlight. Searches over
    // the currently-loaded document slice only.
    const search = useInSessionSearch({
        document: agentAtoms().documentAtom[0],
        jumpTo: scroll.jumpTo,
    });

    // Pane-scoped Ctrl+B / Ctrl+F listener. See hooks/useAgentKeyboard.ts.
    useAgentKeyboard({
        blockId: model.blockId,
        onToggleBookmarks: () => bookmarks.setVisible((v) => !v),
        onToggleSearch: () => {
            // Second Ctrl+F press closes and clears state.
            if (search.visible()) {
                search.close();
            } else {
                search.setVisible(true);
            }
        },
    });

    // Per-pane zoom: read term:zoom from block meta (same key as terminal panes)
    const zoomFactor = createMemo(() => {
        const meta = block()?.meta;
        const z = meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });

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
        <div
            class="agent-view agent-view--presentation"
            style={{ zoom: zoomFactor() }}
            onContextMenu={handleContextMenu}
        >
            {/* Pane title + back button now live in the block frame header,
                driven by AgentViewModel.viewName / viewIcon / endIconButtons.
                See SPEC_AGENT_PANE_FOLLOWUPS item #8. */}

            <Show when={bookmarks.visible()}>
                <BookmarksPanel
                    bookmarks={bookmarks.bookmarks}
                    onJump={bookmarks.jump}
                    onDelete={bookmarks.remove}
                    onRename={bookmarks.rename}
                />
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

            <Show when={!digest.dismissed()}>
                <SessionDigestBanner
                    summary={digest.summary}
                    generatedAt={digest.generatedAt}
                    loading={digest.loading}
                    onDismiss={digest.dismiss}
                    onRegenerate={() => digest.fetch(true)}
                />
            </Show>

            <AgentDocumentView
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
                logLines={logLines}
                authUrl={status.authUrl}
                onSubagentClick={handleSubagentClick}
                onLoadOlder={history.loadOlder}
                loadingOlder={history.loadingOlder}
                bookmarkedNodeIds={bookmarks.bookmarkedNodeIds}
                onBookmark={bookmarks.add}
                scrollCommand={scroll.command}
                scrollToBottomRef={(fn) => { scrollToBottomFn = fn; }}
                highlightNodeId={search.highlightId}
            />

            <Show when={status.loginWaiting()}>
                <div class="agent-retry-bar">
                    <button class="agent-retry-btn agent-retry-btn--cancel" onClick={status.cancelLogin}>
                        Cancel Login
                    </button>
                </div>
            </Show>
            <Show when={status.canRetry()}>
                <div class="agent-retry-bar">
                    <button class="agent-retry-btn" onClick={status.startLaunchFlow}>
                        Retry Login
                    </button>
                </div>
            </Show>

            <div class="agent-composer-region">
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
                    agentId={agentId}
                    onSendMessage={commands.sendMessage}
                    onTyping={() => scrollToBottomFn?.()}
                    onStopAgent={commands.stopAgent}
                    loading={status.isLoading()}
                    getCompletions={commands.completions}
                />
                <AgentControlBar
                    blockId={model.blockId}
                    blockAtom={block}
                    providerId={provider()?.id ?? ""}
                />
            </div>
        </div>
    );
};

AgentPresentationView.displayName = "AgentPresentationView";
