// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import { createMemo, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import type { AgentViewModel } from "./agent-model";
import { buildRuntimeArgs, getRuntimeConfig } from "./buildRuntimeArgs";
import { getProvider, type ProviderDefinition } from "./providers";
import { createAgentAtoms } from "./state";
import type { Bookmark, DocumentNode, SubagentLinkNode } from "./types";
import { openSubagentPane, isSubagentPaneOpen } from "@/app/store/subagent-pane-manager";
import { useAgentStream } from "./useAgentStream";
import { parseHistoryLines } from "./parseHistoryLines";
import { runLaunchFlow } from "./flows/launch-flow";
import { useLaunchLogs } from "./hooks/useLaunchLogs";
import { useSessionDigest } from "./hooks/useSessionDigest";
import { useHistoryPagination } from "./hooks/useHistoryPagination";
import { useInSessionSearch } from "./hooks/useInSessionSearch";
import { useBookmarks } from "./hooks/useBookmarks";
import { useScrollToNode } from "./hooks/useScrollToNode";
import { useAgentKeyboard } from "./hooks/useAgentKeyboard";
import { AgentControlBar } from "./components/AgentControlBar";
import { AgentDocumentView } from "./components/AgentDocumentView";
import { AgentFooter } from "./components/AgentFooter";
import { AgentPicker } from "./components/AgentPicker";
import { AgentSearchBar } from "./components/AgentSearchBar";
import { BookmarksPanel } from "./components/BookmarksPanel";
import { SessionDigestBanner } from "./components/SessionDigestBanner";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { waveEventSubscribe } from "@/app/store/wps";
import * as WOS from "@/app/store/wos";
import { BlockService } from "@/app/store/services";
import { getApi, staticTabId } from "@/app/store/global";
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

    // Accumulated terminal-style log lines — managed by useLaunchLogs hook.
    // `logLines` is the reactive accessor; `log` is the append function
    // that matches the LogFn signature expected by runLaunchFlow().
    // Declared early so subsequent hooks (history, digest, controller
    // status) can take it as a dependency.
    const launchLogs = useLaunchLogs();
    const logLines = launchLogs.lines;
    const log = launchLogs.append;

    // History pagination — owns historyOffset/historyTotal/loadingOlder,
    // documentVersion, loadOlder handler, and the initial async history
    // load that fires on hook mount. See hooks/useHistoryPagination.ts.
    //
    // documentVersion is bumped on every external document mutation
    // (initial load + every loadOlder prepend). useAgentStream subscribes
    // to it and rebuilds its dedup index after the document is reshaped.
    const history = useHistoryPagination({
        blockId: model.blockId,
        documentAtom: agentAtoms().documentAtom,
        outputFormat,
        log,
    });
    const historyOffset = history.historyOffset;
    const historyTotal = history.historyTotal;
    const loadingOlder = history.loadingOlder;
    const loadOlder = history.loadOlder;
    const documentVersion = history.documentVersion;

    // Session digest — owns summary/generatedAt/loading/dismissed signals,
    // the fetch RPC wrapper, and the auto-trigger that decides whether to
    // surface or generate a digest on pane open. See hooks/useSessionDigest.ts.
    const digest = useSessionDigest({
        blockId: model.blockId,
        block,
        log,
    });
    const digestSummary = digest.summary;
    const digestGeneratedAt = digest.generatedAt;
    const digestLoading = digest.loading;
    const digestDismissed = digest.dismissed;
    const fetchDigest = digest.fetch;
    const dismissDigest = digest.dismiss;

    // OAuth URL — shown prominently with a copy button when login is needed
    const [authUrl, setAuthUrl] = createSignal<string | null>(null);
    // Whether to show the retry button (after auth_failed)
    const [canRetry, setCanRetry] = createSignal(false);
    // Whether the launch flow is currently running
    const [flowRunning, setFlowRunning] = createSignal(false);
    // Whether the agent is ready (launch complete, controller registered)
    const [agentReady, setAgentReady] = createSignal(false);
    // Show spinner during launch and until agent is ready.
    // createMemo ensures derived value is cached and only re-evaluates
    // when underlying signals change — not on every caller read.
    const isLoading = createMemo(() => flowRunning() || !agentReady());
    // Whether we're specifically in the login-polling phase
    const [loginWaiting, setLoginWaiting] = createSignal(false);
    // Mutable flag for cancelling the polling loop (set by cancel or onCleanup)
    let loginCancelled = false;

    // Build auth env for a given provider
    const buildAuthEnv = async (prov: ReturnType<typeof provider>): Promise<Record<string, string> | undefined> => {
        if (!prov?.authConfigDirEnvVar || !prov?.authDirName) return undefined;
        try {
            const authDir = await getApi().ensureAuthDir(prov.id);
            const env: Record<string, string> = { [prov.authConfigDirEnvVar]: authDir };
            if (prov.authExtraEnv) Object.assign(env, prov.authExtraEnv);
            return env;
        } catch {
            return undefined; // non-fatal — fall back to default auth dir
        }
    };

    // loadOlder + initial-history-load are owned by useHistoryPagination
    // (local aliases declared above).

    // fetchDigest + dismissDigest are owned by useSessionDigest
    // (local aliases declared above).

    // Runs the full launch flow; can be triggered at mount time or via retry.
    const startLaunchFlow = async () => {
        if (flowRunning()) return;
        loginCancelled = false;
        setFlowRunning(true);
        setCanRetry(false);
        const prov = provider();
        try {
            const authEnv = await buildAuthEnv(prov);
            const result = await runLaunchFlow({
                blockId: model.blockId,
                provider: prov,
                log,
                setAuthUrl,
                isCancelled: () => loginCancelled,
                setLoginWaiting,
                authEnv,
            });
            if (result === "success") {
                setAgentReady(true);
            } else if (result === "auth_failed" && !loginCancelled) {
                setCanRetry(true);
                setAgentReady(true); // clear spinner so retry button is usable
            }
        } catch (err: any) {
            log("error", err?.message ?? String(err), "error");
            setAgentReady(true); // clear spinner on error
        } finally {
            setFlowRunning(false);
        }
    };

    // Cancel login: stop polling and kill the background CLI process.
    const cancelLogin = () => {
        loginCancelled = true;
        getApi().cancelCliLogin().catch(() => {});
        log("auth", "login cancelled", "warn");
    };

    // If the pane is closed while login is in progress, cancel and kill the CLI process.
    onCleanup(() => {
        if (loginWaiting()) {
            loginCancelled = true;
            getApi().cancelCliLogin().catch(() => {});
        }
    });

    onMount(() => {
        const name = block()?.meta?.["agentName"] ?? agentId;
        const prov = provider();
        const provName = prov?.displayName ?? providerKey();
        const cwd = block()?.meta?.["cmd:cwd"] ?? "";

        log("agent", `${name} selected (provider: ${provName})`);
        if (cwd) log("env", `working directory: ${cwd}`);

        // Initial history load is owned by useHistoryPagination's own
        // onMount — fires automatically when the hook mounts. No work
        // here. See hooks/useHistoryPagination.ts.

        // Session digest auto-trigger is owned by useSessionDigest's
        // own onMount (see hooks/useSessionDigest.ts).

        // Full launch flow: CLI resolution → auth check → controller registration
        startLaunchFlow();

        // Subscribe to status changes
        const unsub = waveEventSubscribe({
            eventType: "controllerstatus",
            scope: WOS.makeORef("block", model.blockId),
            handler: (event) => {
                const status = (event as any)?.data?.shellprocstatus;
                if (status === "running") {
                    log("subprocess", "spawned, waiting for response...");
                } else if (status === "done") {
                    const exitCode = (event as any)?.data?.shellprocexitcode;
                    if (exitCode != null && exitCode !== 0) {
                        log("subprocess", `exited with code ${exitCode}`, "error");
                    } else {
                        log("subprocess", "turn complete");
                    }
                }
            },
        });
        onCleanup(() => unsub());

        // Subscribe to subagent:spawned events — render clickable links
        const unsubSpawned = waveEventSubscribe({
            eventType: "subagent:spawned",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (!data?.agentId) return;

                const linkNode: SubagentLinkNode = {
                    type: "subagent_link",
                    id: `subagent_${data.agentId}`,
                    subagentId: data.agentId,
                    slug: data.slug ?? "",
                    parentAgent: data.parentAgent ?? "",
                    sessionId: data.sessionId ?? "",
                    status: "active",
                    model: data.model ?? null,
                };

                const [, setDoc] = agentAtoms().documentAtom;
                setDoc((prev) => [...prev, linkNode]);
                log("subagent", `spawned: ${data.slug || data.agentId}`);
            },
        });
        onCleanup(() => unsubSpawned());

        // Subscribe to subagent:completed — update link status
        const unsubCompleted = waveEventSubscribe({
            eventType: "subagent:completed",
            handler: (event: WaveEvent) => {
                const data = event?.data as any;
                if (!data?.agentId) return;

                const nodeId = `subagent_${data.agentId}`;
                const [, setDoc] = agentAtoms().documentAtom;
                setDoc((prev) =>
                    prev.map((n) =>
                        n.id === nodeId && n.type === "subagent_link"
                            ? { ...n, status: "completed" as const }
                            : n
                    )
                );
            },
        });
        onCleanup(() => unsubCompleted());

        // Ctrl+B / Ctrl+F handling is owned by useAgentKeyboard (called
        // at the top level below — hooks can't mount inside onMount).
    });

    // Pane-scoped Ctrl+B / Ctrl+F listener. See hooks/useAgentKeyboard.ts.
    useAgentKeyboard({
        blockId: model.blockId,
        onToggleBookmarks: () => setShowBookmarks((v) => !v),
        onToggleSearch: () => {
            // Second Ctrl+F press closes and clears state.
            if (searchVisible()) {
                searchClose();
            } else {
                setSearchVisible(true);
            }
        },
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
        documentVersion,
    });

    // Send user message
    const handleSendMessage = async (message: string) => {
        // Add user message as a document node so it appears in the chat
        const [, setDocument] = agentAtoms().documentAtom;
        setDocument((prev) => [
            ...prev,
            {
                type: "user_message",
                id: `user_${Date.now()}`,
                message,
                timestamp: Date.now(),
                collapsed: false,
                summary: "",
            } as DocumentNode,
        ]);

        // Intercept interactive-only slash commands that don't work in
        // stream-json mode. These require TTY/browser interaction.
        const trimmed = message.trim();
        if (trimmed === "/login") {
            const prov = provider();
            const cliPath = block()?.meta?.["cmd"] ?? "";
            if (!prov || !cliPath) {
                log("error", "/login: provider or CLI path not available", "error");
                return;
            }
            log("auth", "running /login via GUI flow...");
            try {
                const authEnv: Record<string, string> = {};
                const envMeta = block()?.meta?.["cmd:env"];
                if (envMeta && typeof envMeta === "object") {
                    for (const [k, v] of Object.entries(envMeta)) {
                        if (typeof v === "string") authEnv[k] = v;
                    }
                }
                const url = await getApi().runCliLogin(cliPath, prov.authLoginCommand, authEnv);
                if (url) {
                    setAuthUrl(url);
                    log("auth", `OAuth URL captured — browser should open automatically`);
                    log("auth", `if it didn't, copy the URL from the box above`);
                } else {
                    log("auth", "a browser window should have opened — complete login there");
                }
                log("auth", "run /cost to verify authentication once logged in");
            } catch (err: any) {
                log("error", `/login failed: ${err?.message ?? String(err)}`, "error");
            }
            return;
        }
        if (trimmed === "/clear") {
            // Frontend-only: clear the document
            setDocument([]);
            log("system", "chat cleared");
            return;
        }

        // Apply runtime args (permission mode, model, effort) before this turn
        const prov = provider();
        if (prov) {
            const runtimeConfig = getRuntimeConfig(block()?.meta);
            const baseArgs = prov.controllerType === "persistent" && prov.persistentLaunchArgs
                ? prov.persistentLaunchArgs
                : prov.launchArgs;
            const updatedArgs = buildRuntimeArgs(baseArgs, runtimeConfig);
            const oref = WOS.makeORef("block", model.blockId);
            try {
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref,
                    meta: { "cmd:args": updatedArgs },
                });
            } catch (err) {
                log("error", `Failed to update runtime args: ${err}`, "error");
            }
        }

        RpcApi.AgentInputCommand(TabRpcClient, {
            blockid: model.blockId,
            message: message,
        }).catch((err) => {
            const errMsg = err?.message ?? String(err);
            log("error", errMsg, "error");
        });
    };

    const handleBack = async () => {
        const oref = WOS.makeORef("block", model.blockId);
        try {
            await RpcApi.SetMetaCommand(TabRpcClient, {
                oref,
                meta: {
                    agentId: null,
                    agentProvider: null,
                    agentOutputFormat: null,
                    agentName: null,
                    agentIcon: null,
                    agentCliPath: null,
                    agentCliArgs: null,
                    agentBinDir: null,
                    controller: null,
                },
            });
        } catch {
            // model logs internally
        }
    };

    // ── Jump-to-node + Bookmarks ────────────────────────────────────────────────

    // Signal-based jump command. AgentDocumentView reacts to changes via a
    // createEffect and does the DOM scroll inside its own container — no
    // mutable refs crossing the component boundary. See hooks/useScrollToNode.ts.
    const scroll = useScrollToNode();

    // Bookmarks — owns reactive list, derived id set, panel visibility,
    // and CRUD callbacks. See hooks/useBookmarks.ts.
    const bookmarksHook = useBookmarks({
        blockId: model.blockId,
        block,
        log,
        jumpTo: scroll.jumpTo,
    });
    const bookmarks = bookmarksHook.bookmarks;
    const bookmarkedNodeIds = bookmarksHook.bookmarkedNodeIds;
    const showBookmarks = bookmarksHook.visible;
    const setShowBookmarks = bookmarksHook.setVisible;
    const handleBookmark = bookmarksHook.add;
    const handleBookmarkDelete = bookmarksHook.remove;
    const handleBookmarkRename = bookmarksHook.rename;
    const handleBookmarkJump = bookmarksHook.jump;
    // Mutable ref to the scrollToBottom function exposed by AgentDocumentView.
    // Called by AgentFooter's onTyping when the user starts composing.
    let scrollToBottomFn: (() => void) | null = null;

    // In-session search — owns matches, current index, navigation, highlight.
    // Searches over the currently-loaded document slice only. See
    // hooks/useInSessionSearch.ts.
    const search = useInSessionSearch({
        document: agentAtoms().documentAtom[0],
        jumpTo: scroll.jumpTo,
    });
    const searchVisible = search.visible;
    const setSearchVisible = search.setVisible;
    const searchCurrentIndex = search.currentIndex;
    const searchMatchCount = search.matchCount;
    const performSearch = search.performSearch;
    const searchNext = search.next;
    const searchPrev = search.prev;
    const searchClose = search.close;
    const searchHighlightId = search.highlightId;

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
            <div class="agent-pres-header">
                <span class="agent-pres-icon">{block()?.meta?.["agentIcon"] ?? "\u26A1"}</span>
                <span class="agent-pres-name">{block()?.meta?.["agentName"] ?? provider()?.displayName ?? agentId}</span>
                <button class="agent-pres-back" onClick={handleBack} title="Back to agents">
                    {"\u2715"}
                </button>
            </div>

            <AgentControlBar
                blockId={model.blockId}
                blockAtom={block}
                providerId={provider()?.id ?? ""}
            />

            <Show when={showBookmarks()}>
                <BookmarksPanel
                    bookmarks={bookmarks}
                    onJump={handleBookmarkJump}
                    onDelete={handleBookmarkDelete}
                    onRename={handleBookmarkRename}
                />
            </Show>

            <AgentSearchBar
                visible={searchVisible}
                onSearch={performSearch}
                onNext={searchNext}
                onPrev={searchPrev}
                onClose={searchClose}
                matchIndex={searchCurrentIndex}
                matchCount={searchMatchCount}
            />

            <Show when={!digestDismissed()}>
                <SessionDigestBanner
                    summary={digestSummary}
                    generatedAt={digestGeneratedAt}
                    loading={digestLoading}
                    onDismiss={dismissDigest}
                    onRegenerate={() => fetchDigest(true)}
                />
            </Show>

            <AgentDocumentView
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
                logLines={logLines}
                authUrl={authUrl}
                onSubagentClick={handleSubagentClick}
                onLoadOlder={loadOlder}
                loadingOlder={loadingOlder}
                bookmarkedNodeIds={bookmarkedNodeIds}
                onBookmark={handleBookmark}
                scrollCommand={scroll.command}
                scrollToBottomRef={(fn) => { scrollToBottomFn = fn; }}
                highlightNodeId={searchHighlightId}
            />

            <Show when={loginWaiting()}>
                <div class="agent-retry-bar">
                    <button class="agent-retry-btn agent-retry-btn--cancel" onClick={cancelLogin}>
                        Cancel Login
                    </button>
                </div>
            </Show>
            <Show when={canRetry()}>
                <div class="agent-retry-bar">
                    <button class="agent-retry-btn" onClick={startLaunchFlow}>
                        Retry Login
                    </button>
                </div>
            </Show>

            <AgentFooter
                agentId={agentId}
                onSendMessage={handleSendMessage}
                onTyping={() => scrollToBottomFn?.()}
                loading={isLoading()}
            />
        </div>
    );
};

AgentPresentationView.displayName = "AgentPresentationView";
