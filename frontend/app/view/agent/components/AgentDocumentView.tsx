// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentView — owns the agent pane's data state interface
 * (collapsed/pinned toggles, auto-collapse policy, auth-url and
 * loading-older banner), then delegates list rendering to
 * AgentDocumentVirtualList (Phase 2 of the virtualization redesign,
 * see docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md).
 *
 * All scroll behavior — stick-to-bottom, anchor capture, jump-to-node,
 * pagination restore — lives in the VirtualList. This component is
 * just glue between the existing AgentAtoms contract and that new
 * component.
 *
 * Diagnostic / launch-flow logs used to render inline at the top of this
 * scroll area. That moved to the dedicated `<ActivityLogPanel>` docked
 * above the composer in the activity-log-panel PR — see
 * `agentmux-ai/AGENT_PANE_ACTIVITY_LOG_SPEC.md`.
 */

import { createSignal, onMount, Show, type Accessor, type JSX } from "solid-js";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState, SubagentLinkNode } from "../types";
import type { ScrollCommand } from "../hooks/useScrollToNode";
import type { LayoutView } from "@/app/store/agent-pane-layout-store";
import { AgentDocumentVirtualList } from "../virtualization/AgentDocumentVirtualList";
import { createAgentViewState } from "../virtualization/state";

interface AgentDocumentViewProps {
    documentAtom: SignalPair<DocumentNode[]>;
    documentStateAtom: SignalPair<DocumentState>;
    authUrl?: Accessor<string | null>;
    /** Provider ID for the active auth flow — used when submitting a pasted auth code. */
    authProviderId?: string;
    onSubagentClick?: (node: SubagentLinkNode) => void;
    /** Called when the user scrolls near the top — load the previous page of history. */
    onLoadOlder?: () => Promise<void>;
    /** Whether an older-history load is currently in progress. */
    loadingOlder?: Accessor<boolean>;
    /** Set of bookmarked node IDs — drives the bookmarked visual indicator. */
    bookmarkedNodeIds?: Accessor<Set<string>>;
    /** Called when the user bookmarks or un-bookmarks a node via context menu. */
    onBookmark?: (node: DocumentNode) => void;
    /**
     * Signal-based jump command. The parent owns a `useScrollToNode`
     * hook and passes its `command` accessor here; the VirtualList
     * watches for changes and scrolls the target node into view.
     */
    scrollCommand?: Accessor<ScrollCommand | null>;
    /**
     * Expose a scrollToBottom function to the parent so the composer can
     * bring the document to the latest content when the user starts typing.
     * Re-engages stick-to-bottom as a side effect.
     */
    scrollToBottomRef?: (fn: () => void) => void;
    /** The node id of the currently highlighted search match (if any). */
    highlightNodeId?: Accessor<string | null>;
    /**
     * Called synchronously during mount with the viewState.markHistoryReady
     * function so the parent can wire it to useHistoryPagination's
     * onHistoryReady callback. Drives the new-message enter animation gate.
     * See PR #1212.
     */
    registerHistoryReadyCallback?: (fn: () => void) => void;
    /**
     * Live per-pane zoom factor (the CSS `zoom` applied on `.agent-view`).
     * Forwarded to the virtual list to normalize measured row heights — see
     * SPEC_AGENT_PANE_VIRTUALIZATION_ZOOM_OVERLAP_2026_06_01.
     */
    zoomFactor?: Accessor<number>;
    /**
     * blockId for the agent-pane-layout slice (Phase 2+). When present,
     * forwarded to AgentDocumentVirtualList so it can route estimates and
     * measurements through the slice (INV-3).
     */
    blockId?: string;
    /** Derived layout view from the agent-pane-layout slice (Phase 3) —
     *  forwarded to the list, which renders rows from its prefix-sum positions. */
    layoutView?: Accessor<LayoutView | null>;
}

export const AgentDocumentView = (props: AgentDocumentViewProps): JSX.Element => {
    const [documentState, setDocumentState] = props.documentStateAtom;

    // Phase 2: build the view state once per mount. Lifetime matches
    // this component (not the agent ViewModel) — that's fine because
    // scroll state is per-pane-mount, not per-agent-session.
    const viewState = createAgentViewState(props.documentAtom);
    // Register the markHistoryReady callback with the parent so
    // useHistoryPagination (called in agent-view.tsx) can signal when
    // the initial history load is done. Called synchronously — before
    // any async history work starts. See PR #1212.
    props.registerHistoryReadyCallback?.(viewState.markHistoryReady);

    // Auto-collapse-on-size for user messages was retired in PR #1020
    // — `UserMessageBlock` keys collapse off `node.isStartup` and
    // `documentState.pinnedNodes`, not `collapsedNodes`. Writing to
    // `collapsedNodes` here would be dead state from the renderer's
    // perspective. See
    // `docs/specs/SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`
    // §D.

    // Toggle collapsed state for collapsible nodes (agent messages, sections, user messages).
    const toggleCollapse = (nodeId: string): void => {
        setDocumentState((prev) => {
            const collapsed = new Set(prev.collapsedNodes);
            if (collapsed.has(nodeId)) collapsed.delete(nodeId);
            else collapsed.add(nodeId);
            return { ...prev, collapsedNodes: collapsed };
        });
    };

    // Toggle the "pinned open" state for a tool node.
    const togglePin = (nodeId: string): void => {
        setDocumentState((prev) => {
            const pinned = new Set(prev.pinnedNodes);
            if (pinned.has(nodeId)) pinned.delete(nodeId);
            else pinned.add(nodeId);
            return { ...prev, pinnedNodes: pinned };
        });
    };

    // Header slot: loading-older banner + optional auth-url box,
    // rendered above the virtualizer inside the scroll container.
    // VirtualList's scrollMargin (=virtualContainerRef.offsetTop)
    // handles the offset automatically.
    const headerSlot = (): JSX.Element => (
        <>
            <Show when={props.loadingOlder?.()}>
                <div class="agent-history-loading">Loading older messages...</div>
            </Show>
            <Show when={props.authUrl?.()}>
                {(url) => <AuthUrlBox url={url()} authProviderId={props.authProviderId} />}
            </Show>
        </>
    );

    return (
        <AgentDocumentVirtualList
            viewState={viewState}
            documentState={documentState}
            bookmarkedNodeIds={props.bookmarkedNodeIds}
            onBookmark={props.onBookmark}
            onSubagentClick={props.onSubagentClick}
            onLoadOlder={props.onLoadOlder}
            loadingOlder={props.loadingOlder}
            highlightNodeId={props.highlightNodeId}
            scrollCommand={props.scrollCommand}
            scrollToBottomRef={props.scrollToBottomRef}
            onToggleCollapse={toggleCollapse}
            onTogglePin={togglePin}
            zoomFactor={props.zoomFactor}
            blockId={props.blockId}
            layoutView={props.layoutView}
            headerSlot={headerSlot()}
        />
    );
};

AgentDocumentView.displayName = "AgentDocumentView";

// ── Auth URL box ────────────────────────────────────────────────────────────

interface AuthUrlBoxProps {
    url: string;
    authProviderId?: string;
}

/**
 * OAuth code-paste box. Rendered at the top of the scroll container
 * while a login flow has a pending auth URL. Previously inline in
 * AgentDocumentView; extracted as a sibling for the Phase 2 shell.
 */
function AuthUrlBox(props: AuthUrlBoxProps): JSX.Element {
    const [pasteCode, setPasteCode] = createSignal("");
    const [pasting, setPasting] = createSignal(false);
    const [pasteResult, setPasteResult] = createSignal<string | null>(null);
    let inputRef: HTMLInputElement | undefined;

    // Grab focus when the box appears so the user's pasted code lands HERE,
    // not in the main agent message input (which otherwise holds focus and
    // silently swallows the paste — the login then stalls waiting on stdin).
    // Deferred past the current focus cycle (the pane's focus-reclaim runs
    // synchronously on render) so this wins the race.
    onMount(() => {
        requestAnimationFrame(() => inputRef?.focus());
    });

    const handleSubmitCode = async (): Promise<void> => {
        const code = pasteCode().trim();
        if (!code) return;
        setPasting(true);
        setPasteResult(null);
        try {
            const { getApi } = await import("@/app/store/global");
            await getApi().setProviderAuth(props.authProviderId ?? "claude", code);
            setPasteResult("Code accepted — waiting for confirmation...");
            setPasteCode("");
        } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            setPasteResult(`Error: ${msg}`);
        } finally {
            setPasting(false);
        }
    };

    return (
        <div class="agent-auth-url-box">
            <div class="agent-auth-url-label">Open this URL to log in:</div>
            <div class="agent-auth-url-row">
                <span class="agent-auth-url-text">{props.url}</span>
                <button
                    class="agent-auth-url-copy"
                    onClick={() => { import("@/util/clipboard").then(c => c.writeText(props.url)); }}
                    title="Copy URL"
                >
                    Copy
                </button>
            </div>
            <div class="agent-auth-paste-row">
                <input
                    ref={inputRef}
                    class="agent-auth-paste-input"
                    type="text"
                    placeholder="Paste auth code from Anthropic..."
                    value={pasteCode()}
                    onInput={(e) => setPasteCode((e.target as HTMLInputElement).value)}
                    onKeyDown={(e) => { if (e.key === "Enter") void handleSubmitCode(); }}
                />
                <button
                    class="agent-auth-url-copy"
                    title="Paste from clipboard"
                    onClick={() => {
                        import("@/util/clipboard").then(c => c.readText()).then(text => {
                            if (text) setPasteCode(text.trim());
                        }).catch(() => {
                            setPasteResult("Could not read clipboard — paste manually");
                        });
                    }}
                >
                    Paste
                </button>
                <button
                    class="agent-auth-url-copy"
                    onClick={() => { void handleSubmitCode(); }}
                    disabled={!pasteCode().trim() || pasting()}
                >
                    {pasting() ? "..." : "Submit"}
                </button>
            </div>
            <Show when={pasteResult()}>
                <div class="agent-auth-paste-result">{pasteResult()}</div>
            </Show>
        </div>
    );
}
