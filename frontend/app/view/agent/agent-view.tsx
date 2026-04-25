// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createMemo, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { getProvider } from "./providers";
import { createAgentAtoms } from "./state";
import type { SubagentLinkNode } from "./types";
import { openSubagentPane, isSubagentPaneOpen } from "@/app/store/subagent-pane-manager";
import { useAgentStream } from "./useAgentStream";
import { useActivityLog } from "./hooks/useActivityLog";
import { useSessionDigest } from "./hooks/useSessionDigest";
import { useHistoryPagination } from "./hooks/useHistoryPagination";
import { useAgentControllerStatus } from "./hooks/useAgentControllerStatus";
import { useInSessionSearch } from "./hooks/useInSessionSearch";
import { useBookmarks } from "./hooks/useBookmarks";
import { useScrollToNode } from "./hooks/useScrollToNode";
import { useAgentKeyboard } from "./hooks/useAgentKeyboard";
import { useProcessCount } from "./hooks/useProcessCount";
import { useSubagentEvents } from "./hooks/useSubagentEvents";
import { useControllerStatusEvents } from "./hooks/useControllerStatusEvents";
import { useAgentCommands } from "./hooks/useAgentCommands";
import { AgentControlBar } from "./components/AgentControlBar";
import { ActivityLogPanel } from "./components/ActivityLogPanel";
import { AgentDecisionPanel } from "./components/AgentDecisionPanel";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter, AgentStatusLine } from "./components/AgentFooter";
import { PendingMessagesPanel } from "./components/PendingMessagesPanel";
import { AgentPicker, useForgeAgents } from "./components/AgentPicker";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { AgentFocusedPanel } from "./components/AgentFocusedPanel";
import { SlashCommandPicker } from "./components/SlashCommandPicker";
import { SlashHelpPanel } from "./components/SlashHelpPanel";
import { BookmarksPanel } from "./components/BookmarksPanel";
import { SessionDigestBanner } from "./components/SessionDigestBanner";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createBlock, getApi, WOS } from "@/app/store/global";
import { ConfirmModal } from "@/element/modal-v2";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { parseAgentAccounts, loadAccounts } from "@/app/view/identity/identity-model";
import { buildStartupPayload, resolveAccounts } from "./startup/buildStartupPayload";
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

    // Overlay tab signal — lives in the component so SolidJS can track it.
    // The model's _setOverlayTab callback is wired on mount and cleaned up on unmount.
    const [showOverlayTab, setShowOverlayTab] = createSignal<import("./agent-model").OverlayTab | null>(null);
    onMount(() => {
        model._setOverlayTab = setShowOverlayTab;
    });
    onCleanup(() => {
        model._setOverlayTab = null;
    });

    // Reactive forge agent list — used to resolve the current ForgeAgent object
    // so the overlay can pass it to AgentCardSettingsPanel / rename input.
    const forgeAgents = useForgeAgents();
    const currentAgent = createMemo(() => forgeAgents().find((a) => a.id === agentId));

    const agentAtoms = createMemo(() => createAgentAtoms(model.blockId));

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
    const [getDocument, setDocument] = agentAtoms().documentAtom;

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
        setDocument((prev) =>
            prev.map((n) => {
                if (n.type !== "tool" || n.status !== "pending_approval") return n;
                if (n.pendingPermission?.request_id !== decision.request_id) return n;
                return {
                    ...n,
                    status: decision.outcome === "allow" ? "running" : "denied",
                    pendingPermission: undefined,
                };
            }),
        );
        // Send the decision to the sidecar so it can write y/n to the
        // subprocess's stdin (PR-3a). Rules persistence + memory store
        // come in PR-3b. Spec: SPEC_DECISION_PROMPT_2026_04_24.md §4.3.
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
    const status = useAgentControllerStatus({
        blockId: model.blockId,
        provider,
        log,
        onLoginSuccess: (email) => {
            const display = email ? `Logged in as **${email}**` : "Login successful";
            setDocument((prev) => [
                ...prev,
                {
                    type: "markdown",
                    id: `login_success_${Date.now()}`,
                    content: `\u2713 ${display}`,
                } as import("./types").MarkdownNode,
            ]);
        },
        onReady: () => onReadyFn?.(),
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
    // `documentVersion` is bumped whenever we mutate the document externally
    // (history load / prepend), causing useAgentStream to rebuild its
    // nodeIdSet and nodeIndexMap.
    const stoppingAtom = agentAtoms().stoppingAtom;
    const pendingMessagesAtom = agentAtoms().pendingMessagesAtom;
    useAgentStream({
        blockId: model.blockId,
        outputFormat: outputFormat(),
        documentAtom: agentAtoms().documentAtom,
        streamingStateAtom: agentAtoms().streamingStateAtom,
        sessionStatsAtom: agentAtoms().sessionStatsAtom,
        currentToolAtom: agentAtoms().currentToolAtom,
        turnTokensAtom: agentAtoms().turnTokensAtom,
        turnActiveAtom: agentAtoms().turnActiveAtom,
        stoppingAtom,
        pendingMessagesAtom,
        enabled: true,
        documentVersion: history.documentVersion,
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
        stoppingAtom,
        pendingMessagesAtom,
    });

    // Clear session stats and mark turn as active when the user sends a message.
    const [, setSessionStats] = agentAtoms().sessionStatsAtom;
    const [, setTurnActive] = agentAtoms().turnActiveAtom;
    const handleSendMessage = (message: string): Promise<void> => {
        setSessionStats(null);
        setTurnActive(true);
        return commands.sendMessage(message);
    };

    // ── Startup sequence ────────────────────────────────────────────────────────
    // On first connect (no existing session), assemble a structured startup
    // payload from Forge + Identity data and send it as the opening turn.
    // See docs/specs/SPEC_AGENT_STARTUP_SEQUENCE_2026_04_16.md
    onReadyFn = async () => {
        // Skip if this is a resumed session
        if (block()?.meta?.["agent:sessionid"]) return;

        try {
            const agent = currentAgent();
            if (!agent) return;

            // Gather inputs in parallel where possible
            const [startupContentResult, version] = await Promise.all([
                RpcApi.GetForgeContentCommand(TabRpcClient, {
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
                peerAgents: forgeAgents(),
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

    // Per-pane zoom: read term:zoom from block meta (same key as terminal panes).
    const zoomFactor = createMemo(() => {
        const meta = block()?.meta;
        const z = meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });

    // Persist a zoom change to block meta. Null when back at 1.0 so
    // the key round-trips clean instead of persisting a default.
    const setZoom = (next: number): void => {
        const clamped = Math.max(0.5, Math.min(2.0, Math.round(next * 100) / 100));
        void RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", model.blockId),
            meta: { "term:zoom": clamped === 1.0 ? null : clamped },
        });
    };

    let rootRef: HTMLDivElement | undefined;

    // Ctrl+Wheel to zoom — capture phase on the root so we intercept
    // before child scroll handlers (AgentDocumentView's scrollable
    // region) and before CEF's native page zoom. Same pattern as
    // `term.tsx`.
    onMount(() => {
        if (!rootRef) return;
        const el = rootRef;
        const handleCtrlWheel = (ev: WheelEvent) => {
            if (!ev.ctrlKey) return;
            ev.preventDefault();
            ev.stopPropagation();
            const STEP = 0.1;
            const delta = ev.deltaY > 0 ? -STEP : STEP;
            setZoom(zoomFactor() + delta);
        };
        el.addEventListener("wheel", handleCtrlWheel, { passive: false, capture: true });
        onCleanup(() => el.removeEventListener("wheel", handleCtrlWheel, { capture: true }));
    });

    // Ctrl+Plus / Ctrl+Minus / Ctrl+0 for keyboard zoom. Attached to
    // `document` in capture phase with a containment check against
    // `rootRef`, so:
    //   - it fires regardless of which descendant (textarea, doc view,
    //     etc.) currently has focus;
    //   - it only fires for this pane (multiple agent panes each
    //     zoom independently);
    //   - it wins over a child that might call stopPropagation on
    //     Ctrl+±.
    onMount(() => {
        if (!rootRef) return;
        const el = rootRef;
        const handleKey = (ev: KeyboardEvent) => {
            if (!ev.ctrlKey || ev.altKey || ev.metaKey) return;
            if (!(ev.target instanceof Node) || !el.contains(ev.target)) return;
            const STEP = 0.1;
            if (ev.key === "+" || ev.key === "=") {
                ev.preventDefault();
                ev.stopPropagation();
                setZoom(zoomFactor() + STEP);
            } else if (ev.key === "-" || ev.key === "_") {
                ev.preventDefault();
                ev.stopPropagation();
                setZoom(zoomFactor() - STEP);
            } else if (ev.key === "0") {
                ev.preventDefault();
                ev.stopPropagation();
                setZoom(1.0);
            }
        };
        document.addEventListener("keydown", handleKey, { capture: true });
        onCleanup(() => document.removeEventListener("keydown", handleKey, { capture: true }));
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
            ref={rootRef}
            class="agent-view agent-view--presentation"
            style={{ zoom: zoomFactor() }}
            onContextMenu={handleContextMenu}
            tabIndex={-1}
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

            {/* Title-bar action overlay: ⚙ Forge / 👤 Identity */}
            <Show when={showOverlayTab() != null && currentAgent() != null}>
                <AgentFocusedPanel
                    blockId={model.blockId}
                    nodeModel={model.nodeModel}
                    agent={currentAgent()!}
                    initialTab={showOverlayTab()!}
                    onClose={() => setShowOverlayTab(null)}
                    onTabChange={(tab) => { model._lastOverlayTab = tab; }}
                />
            </Show>

            <AgentDocumentView
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
                authUrl={status.authUrl}
                authProviderId={provider()?.id ?? providerKey()}
                onSubagentClick={handleSubagentClick}
                onLoadOlder={history.loadOlder}
                loadingOlder={history.loadingOlder}
                bookmarkedNodeIds={bookmarks.bookmarkedNodeIds}
                onBookmark={bookmarks.add}
                scrollCommand={scroll.command}
                scrollToBottomRef={(fn) => { scrollToBottomFn = fn; }}
                highlightNodeId={search.highlightId}
            />

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

            {/* Queue sits directly below the feed so the user's newly-
                typed message lands next to the live conversation it's
                queued against. Previously lived below the activity log;
                repositioning per SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.
                "Send now" lives inside the queue header (right side)
                so it sits adjacent to the messages it accelerates. */}
            <PendingMessagesPanel
                pendingMessages={pendingMessagesAtom[0]}
                showSendNow={() =>
                    agentAtoms().turnActiveAtom[0]() &&
                    pendingMessagesAtom[0]().length > 0
                }
                onSendImmediately={() => {
                    commands.stopAgent();
                }}
            />

            {/* Working…/Stopping… status — reads as "what the agent is
                doing about the queue above". Used to live inside the
                composer region right above the input; moved here so the
                activity log doesn't push it off-screen during long
                agent output. */}
            <AgentStatusLine
                loading={status.isLoading() || agentAtoms().turnActiveAtom[0]() || stoppingAtom[0]()}
                stopping={stoppingAtom[0]()}
                currentTool={agentAtoms().currentToolAtom[0]()}
                sessionStats={agentAtoms().sessionStatsAtom[0]()}
                turnTokens={agentAtoms().turnTokensAtom[0]()}
                processCount={processCount()}
                onProcessBadgeClick={() => {
                    // Open the swarm pane so the user can see every
                    // process (and eventually kill them). Idempotent
                    // by design — createBlock doesn't dedupe, so
                    // clicking repeatedly creates multiple panes.
                    // Acceptable trade-off until we add a "focus if
                    // already open" swarm-pane-manager entry point.
                    createBlock({ meta: { view: "swarm" } });
                }}
            />

            <ActivityLogPanel entries={logLines} />

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
                <AgentControlBar
                    blockId={model.blockId}
                    blockAtom={block}
                    providerId={provider()?.id ?? ""}
                />
                <AgentFooter
                    agentId={agentId}
                    onSendMessage={handleSendMessage}
                    onTyping={() => scrollToBottomFn?.()}
                    onStopAgent={commands.stopAgent}
                    getCompletions={commands.completions}
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
