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

import { createEffect, createSignal, Show, type Accessor, type JSX } from "solid-js";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState, SubagentLinkNode } from "../types";
import type { ScrollCommand } from "../hooks/useScrollToNode";
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
}

export const AgentDocumentView = (props: AgentDocumentViewProps): JSX.Element => {
    const [documentState, setDocumentState] = props.documentStateAtom;

    // Phase 2: build the view state once per mount. Lifetime matches
    // this component (not the agent ViewModel) — that's fine because
    // scroll state is per-pane-mount, not per-agent-session.
    const viewState = createAgentViewState(props.documentAtom);

    // Auto-collapse large user_messages on first arrival. The startup
    // session-context payload that `buildStartupPayload` sends is a huge
    // JSON block that visually dominates the pane; short user turns
    // (one-line prompts) fit unchanged. Tracked per-node via `seenIds`
    // so we don't re-collapse after the user has explicitly expanded.
    const seenUserMessageIds = new Set<string>();
    createEffect(() => {
        const doc = viewState.nodes();
        const toCollapse: string[] = [];
        for (const n of doc) {
            if (n.type !== "user_message") continue;
            if (seenUserMessageIds.has(n.id)) continue;
            seenUserMessageIds.add(n.id);
            const msg = (n as { message?: string }).message ?? "";
            if (msg.length > 200 || msg.includes("\n")) {
                toCollapse.push(n.id);
            }
        }
        if (toCollapse.length > 0) {
            setDocumentState((prev) => {
                const next = new Set(prev.collapsedNodes);
                for (const id of toCollapse) next.add(id);
                return { ...prev, collapsedNodes: next };
            });
        }
    });

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
