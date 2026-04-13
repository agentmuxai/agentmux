// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import { focusedBlockId } from "@/util/focusutil";
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
import { useHistoryPagination } from "./hooks/useHistoryPagination";
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

    // ── Session digest ──────────────────────────────────────────────────────────
    // Shows a collapsible AI-generated summary when the user returns to a stale
    // session (idle >1 hour with >20 lines of new activity).
    const [digestSummary, setDigestSummary] = createSignal<string | null>(null);
    const [digestGeneratedAt, setDigestGeneratedAt] = createSignal<number | null>(null);
    const [digestLoading, setDigestLoading] = createSignal(false);
    const [digestDismissed, setDigestDismissed] = createSignal(false);

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

    // Fetch (or regenerate) the session digest via the session:digest RPC.
    // `force=true` bypasses the cache and re-invokes the Claude CLI.
    const fetchDigest = async (force = false): Promise<void> => {
        if (digestLoading()) return;
        setDigestLoading(true);
        try {
            const result = await RpcApi.SessionDigestCommand(TabRpcClient, {
                block_id: model.blockId,
                force,
            }, { timeout: 90000 }); // 60s CLI + headroom

            if (result.summary) {
                setDigestSummary(result.summary);
                setDigestGeneratedAt(result.generated_at > 0 ? result.generated_at : null);
            } else {
                // Backend returned an empty summary (CLI unavailable, etc.) — hide the banner
                setDigestSummary(null);
            }
        } catch (err: any) {
            log("digest", `failed to generate session digest: ${err?.message ?? String(err)}`, "warn");
            setDigestSummary(null);
        } finally {
            setDigestLoading(false);
        }
    };

    const dismissDigest = () => setDigestDismissed(true);

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

        // ── Session digest auto-trigger ─────────────────────────────────────────
        // Decide whether to show (or generate) a digest on pane open.
        // Conditions: the session was idle for >1 hour AND has >20 lines.
        // We use block meta for both facts; no extra RPC needed.
        (() => {
            const meta = block()?.meta ?? {};
            const lastActivityMs: number = typeof meta["session:last_activity_ms"] === "number"
                ? (meta["session:last_activity_ms"] as number)
                : 0;
            const lineCount: number = typeof meta["session:line_count"] === "number"
                ? (meta["session:line_count"] as number)
                : 0;
            const cachedDigest = typeof meta["session:digest_summary"] === "string"
                ? (meta["session:digest_summary"] as string)
                : null;
            const cachedDigestAt: number = typeof meta["session:digest_generated_at"] === "number"
                ? (meta["session:digest_generated_at"] as number)
                : 0;
            const digestLastLineCount: number = typeof meta["session:digest_last_line_count"] === "number"
                ? (meta["session:digest_last_line_count"] as number)
                : 0;

            const idleMs = lastActivityMs > 0 ? Date.now() - lastActivityMs : 0;
            const idleOverOneHour = idleMs > 3600000;
            const linesSinceDigest = lineCount - digestLastLineCount;

            if (cachedDigest) {
                // Always show the cached digest if available — let the backend decide
                // on staleness when the user clicks Regenerate.
                setDigestSummary(cachedDigest);
                setDigestGeneratedAt(cachedDigestAt > 0 ? cachedDigestAt : null);

                // Auto-regenerate in the background if idle >1h AND stale (>20 new lines)
                if (idleOverOneHour && linesSinceDigest >= 20) {
                    fetchDigest(false); // non-forced — backend will regenerate due to line delta
                }
            } else if (idleOverOneHour && lineCount > 20) {
                // No cached digest — auto-generate one (takes 2-5s; show loading state)
                fetchDigest(false);
            }
        })();

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

        // Ctrl+B — toggle bookmarks panel.
        // Ctrl+F — toggle search bar.
        // Both are scoped to THIS pane via focusedBlockId() so that only the
        // focused pane responds when multiple agent panes are open.
        const handleKeyDown = (e: KeyboardEvent) => {
            const focused = focusedBlockId();
            if (focused !== model.blockId) return;

            if (e.ctrlKey && e.key === "b") {
                e.preventDefault();
                setShowBookmarks((v) => !v);
            } else if (e.ctrlKey && e.key === "f") {
                e.preventDefault();
                // Capture current state BEFORE toggling so we know if this
                // Ctrl+F press is opening or closing the bar.
                const wasVisible = searchVisible();
                if (wasVisible) {
                    // Second Ctrl+F press closes and clears state.
                    searchClose();
                } else {
                    setSearchVisible(true);
                }
            }
        };
        window.addEventListener("keydown", handleKeyDown);
        onCleanup(() => window.removeEventListener("keydown", handleKeyDown));
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

    // ── Bookmarks ───────────────────────────────────────────────────────────────

    // Read bookmarks reactively from block meta ("agent:bookmarks").
    const bookmarks = createMemo<Bookmark[]>(() => {
        const raw = block()?.meta?.["agent:bookmarks"];
        if (!Array.isArray(raw)) return [];
        return raw as Bookmark[];
    });

    // Derived set of bookmarked nodeIds for O(1) look-up in the renderer.
    const bookmarkedNodeIds = createMemo<Set<string>>(
        () => new Set(bookmarks().map((b) => b.nodeId)),
    );

    // Whether the bookmarks panel is visible.
    const [showBookmarks, setShowBookmarks] = createSignal(false);

    // Mutable ref to the scrollToNode function exposed by AgentDocumentView.
    let scrollToNodeFn: ((nodeId: string) => void) | null = null;
    // Mutable ref to the scrollToBottom function exposed by AgentDocumentView.
    // Called by AgentFooter's onTyping when the user starts composing.
    let scrollToBottomFn: (() => void) | null = null;

    // ── In-session search ───────────────────────────────────────────────────────
    // Searches over the currently-loaded document slice only. Searching the
    // full persisted history would require a backend blockfile:search RPC —
    // out of scope for this PR.

    const [searchVisible, setSearchVisible] = createSignal(false);
    /** Node IDs whose text content matches the active query. */
    const [searchMatches, setSearchMatches] = createSignal<string[]>([]);
    /** 0-based index into searchMatches. -1 when there are no matches. */
    const [searchCurrentIndex, setSearchCurrentIndex] = createSignal(-1);

    /** Extract searchable plain text from any document node. */
    const nodeSearchText = (node: DocumentNode): string => {
        switch (node.type) {
            case "markdown":      return node.content;
            case "user_message":  return node.message;
            case "agent_message": return node.message;
            case "tool":          return node.tool + " " + JSON.stringify(node.params ?? {});
            case "section":       return node.title;
            case "subagent_link": return node.slug + " " + node.subagentId;
            default:              return "";
        }
    };

    const performSearch = (query: string) => {
        if (!query.trim()) {
            setSearchMatches([]);
            setSearchCurrentIndex(-1);
            return;
        }
        const q = query.toLowerCase();
        const [doc] = agentAtoms().documentAtom;
        const matches: string[] = [];
        for (const node of doc()) {
            if (nodeSearchText(node).toLowerCase().includes(q)) {
                matches.push(node.id);
            }
        }
        setSearchMatches(matches);
        const newIndex = matches.length > 0 ? 0 : -1;
        setSearchCurrentIndex(newIndex);
        if (newIndex >= 0 && scrollToNodeFn) {
            scrollToNodeFn(matches[0]);
        }
    };

    const searchNext = () => {
        const matches = searchMatches();
        if (matches.length === 0) return;
        const next = (searchCurrentIndex() + 1) % matches.length;
        setSearchCurrentIndex(next);
        if (scrollToNodeFn) scrollToNodeFn(matches[next]);
    };

    const searchPrev = () => {
        const matches = searchMatches();
        if (matches.length === 0) return;
        const prev = (searchCurrentIndex() - 1 + matches.length) % matches.length;
        setSearchCurrentIndex(prev);
        if (scrollToNodeFn) scrollToNodeFn(matches[prev]);
    };

    const searchClose = () => {
        setSearchVisible(false);
        setSearchMatches([]);
        setSearchCurrentIndex(-1);
    };

    /** Node id of the currently highlighted search result, or null. */
    const searchHighlightId = createMemo<string | null>(() => {
        const matches = searchMatches();
        const idx = searchCurrentIndex();
        return idx >= 0 && idx < matches.length ? matches[idx] : null;
    });

    const saveBookmarks = async (next: Bookmark[]): Promise<void> => {
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("block", model.blockId),
            meta: { "agent:bookmarks": next },
        });
    };

    /** Extract plain text preview from a DocumentNode (≤80 chars). */
    const nodePreview = (node: DocumentNode): string => {
        let raw = "";
        switch (node.type) {
            case "markdown":    raw = node.content; break;
            case "user_message": raw = node.message; break;
            case "tool":        raw = node.summary || node.tool; break;
            case "agent_message": raw = node.summary || node.message; break;
            case "section":     raw = node.title; break;
            case "subagent_link": raw = node.slug || node.subagentId; break;
        }
        return raw.replace(/\s+/g, " ").trim().slice(0, 80);
    };

    const handleBookmark = (node: DocumentNode): void => {
        const current = bookmarks();
        const existingIdx = current.findIndex((b) => b.nodeId === node.id);
        let next: Bookmark[];
        if (existingIdx >= 0) {
            // Remove existing bookmark
            next = current.filter((_, i) => i !== existingIdx);
        } else {
            const preview = nodePreview(node);
            const label = preview.slice(0, 60) || node.id;
            const newBookmark: Bookmark = {
                id: crypto.randomUUID(),
                nodeId: node.id,
                createdAt: Date.now(),
                label,
                preview,
            };
            next = [...current, newBookmark];
            // Open the panel on first bookmark
            setShowBookmarks(true);
        }
        saveBookmarks(next).catch((err) => {
            log("bookmark", `failed to save: ${err?.message ?? String(err)}`, "warn");
        });
    };

    const handleBookmarkDelete = (id: string): void => {
        const next = bookmarks().filter((b) => b.id !== id);
        saveBookmarks(next).catch((err) => {
            log("bookmark", `failed to save: ${err?.message ?? String(err)}`, "warn");
        });
    };

    const handleBookmarkRename = (id: string, label: string): void => {
        const next = bookmarks().map((b) => (b.id === id ? { ...b, label } : b));
        saveBookmarks(next).catch((err) => {
            log("bookmark", `failed to save: ${err?.message ?? String(err)}`, "warn");
        });
    };

    const handleBookmarkJump = (nodeId: string): void => {
        if (scrollToNodeFn) scrollToNodeFn(nodeId);
    };

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
                matchCount={() => searchMatches().length}
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
                scrollToNodeRef={(fn) => { scrollToNodeFn = fn; }}
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
