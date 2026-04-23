// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentView — Renders the styled document as a list of DocumentNodes.
 * Routes each node type to the appropriate block component.
 *
 * Diagnostic / launch-flow logs used to render inline at the top of this
 * scroll area. That moved to the dedicated `<ActivityLogPanel>` docked
 * above the composer in the activity-log-panel PR — see
 * `agentmux-ai/AGENT_PANE_ACTIVITY_LOG_SPEC.md`.
 */

import { createEffect, createSignal, For, Show, type Accessor, type JSX, onCleanup } from "solid-js";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState, SubagentLinkNode } from "../types";
import type { ScrollCommand } from "../hooks/useScrollToNode";
import { AgentMessageBlock } from "./AgentMessageBlock";
import { MarkdownBlock } from "./MarkdownBlock";
import { NodeHoverStrip } from "./NodeHoverStrip";
import { SubagentLinkBlock } from "./SubagentLinkBlock";
import { ToolBlock } from "./ToolBlock";
import { ContextMenuModel } from "@/app/store/contextmenu";

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
     * hook and passes its `command` accessor here; a createEffect
     * watches for changes and scrolls the target node into view.
     * Replaces the old mutable `scrollToNodeRef` ref pattern.
     */
    scrollCommand?: Accessor<ScrollCommand | null>;
    /**
     * Expose a scrollToBottom function to the parent so the composer can
     * bring the document to the latest content when the user starts typing.
     * Re-enables auto-scroll as a side effect.
     */
    scrollToBottomRef?: (fn: () => void) => void;
    /** The node id of the currently highlighted search match (if any). */
    highlightNodeId?: Accessor<string | null>;
}

export const AgentDocumentView = ({ documentAtom, documentStateAtom, authUrl, authProviderId, onSubagentClick, onLoadOlder, loadingOlder, bookmarkedNodeIds, onBookmark, scrollCommand, scrollToBottomRef, highlightNodeId }: AgentDocumentViewProps): JSX.Element => {
    const [document] = documentAtom;
    const [documentState, setDocumentState] = documentStateAtom;
    let scrollRef!: HTMLDivElement;
    let autoScroll = true;
    // Guard against concurrent older-history fetches triggered by scroll
    let loadingOlderInFlight = false;

    // Scroll to a node by its data-node-id attribute.
    // Exposed to the parent via scrollToNodeRef so BookmarksPanel can call it.
    //
    // We deliberately DO NOT use `el.scrollIntoView()` here. scrollIntoView
    // walks every scrollable ancestor and scrolls each one until the target
    // is inside its visible region — which in our pane/tab/window nesting
    // scrolls the outer block frame, pushing the agent pane header and
    // adjacent pane titles out of the viewport. The symptom is "pane titles
    // disappear across the entire app when I click a bookmark" — a real bug
    // reported against 0.33.106.
    //
    // Instead, compute the target element's top relative to scrollRef and
    // set scrollRef.scrollTop directly. That scrolls ONLY the document
    // container and leaves every ancestor untouched.
    //
    // See docs/analysis/agent-pane-rich-features-structure-2026-04-13.md §1.
    const scrollToNode = (nodeId: string) => {
        if (!scrollRef) return;
        const el = scrollRef.querySelector(`[data-node-id="${nodeId}"]`) as HTMLElement | null;
        if (!el) return;
        const elRect = el.getBoundingClientRect();
        const containerRect = scrollRef.getBoundingClientRect();
        // Top of el relative to scrollRef's content origin
        const offsetWithinContainer = elRect.top - containerRect.top + scrollRef.scrollTop;
        // Center the element in the visible region
        const centerOffset =
            offsetWithinContainer - scrollRef.clientHeight / 2 + el.clientHeight / 2;
        scrollRef.scrollTo({ top: Math.max(0, centerOffset), behavior: "smooth" });
        // Disable auto-scroll after a manual jump
        autoScroll = false;
    };

    // React to jump commands emitted by the parent's useScrollToNode hook.
    // We run the scroll inside our own effect (rather than exposing the
    // function upward) so the mutable-ref pattern goes away and callers
    // don't need to touch AgentDocumentView internals.
    createEffect(() => {
        const cmd = scrollCommand?.();
        if (cmd) scrollToNode(cmd.nodeId);
    });

    // Jump to the bottom of the document and re-enable auto-scroll.
    //
    // Called by AgentFooter's onInput handler when the user starts typing, so
    // their composer input is visually anchored to the latest content instead
    // of floating offscreen at the bottom while older content sits above.
    //
    // Named `jumpToBottom` to distinguish from the existing `scrollToBottom`
    // below, which is gated on `autoScroll` and only runs when new content
    // arrives. This one is unconditional and also re-enables auto-scroll.
    //
    // Uses `Number.MAX_SAFE_INTEGER` instead of reading `scrollHeight` so we
    // never force a synchronous layout on the keystroke hot path (PR #345's
    // autoGrow regression was exactly that pattern). The browser clamps to
    // the actual max internally during compositing.
    const jumpToBottom = () => {
        if (!scrollRef) return;
        autoScroll = true;
        scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "instant" });
    };

    if (scrollToBottomRef) scrollToBottomRef(jumpToBottom);

    // Auto-collapse large user_messages on first arrival. The startup
    // session-context payload that `buildStartupPayload` sends is a huge
    // JSON block that visually dominates the pane; short user turns
    // (one-line prompts) fit unchanged. Tracked per-node via `seenIds`
    // so we don't re-collapse after the user has explicitly expanded.
    const seenUserMessageIds = new Set<string>();
    createEffect(() => {
        const doc = document();
        const toCollapse: string[] = [];
        for (const n of doc) {
            if (n.type !== "user_message") continue;
            if (seenUserMessageIds.has(n.id)) continue;
            seenUserMessageIds.add(n.id);
            const msg = (n as any).message ?? "";
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
    const toggleCollapse = (nodeId: string) => {
        setDocumentState((prev) => {
            const collapsed = new Set(prev.collapsedNodes);
            if (collapsed.has(nodeId)) {
                collapsed.delete(nodeId);
            } else {
                collapsed.add(nodeId);
            }
            return { ...prev, collapsedNodes: collapsed };
        });
    };

    // Toggle the "pinned open" state for a tool node.
    const togglePin = (nodeId: string) => {
        setDocumentState((prev) => {
            const pinned = new Set(prev.pinnedNodes);
            if (pinned.has(nodeId)) pinned.delete(nodeId);
            else pinned.add(nodeId);
            return { ...prev, pinnedNodes: pinned };
        });
    };

    // Auto-scroll to bottom when new content arrives.
    let scrollRafId: number | null = null;

    const scrollToBottom = () => {
        if (autoScroll && scrollRef) {
            scrollRef.scrollTop = scrollRef.scrollHeight;
        }
    };

    // Scroll when the document changes — throttled to one RAF per batch.
    // Activity log no longer lives in this scroll container, so its
    // length is no longer a trigger (that panel scrolls internally).
    createEffect(() => {
        const _docLen = document().length;
        if (scrollRafId == null) {
            scrollRafId = requestAnimationFrame(() => {
                scrollRafId = null;
                scrollToBottom();
            });
        }
    });

    onCleanup(() => {
        if (scrollRafId != null) cancelAnimationFrame(scrollRafId);
    });

    // Detect if user scrolled up (disable auto-scroll) and trigger older-history load
    const handleScroll = () => {
        if (!scrollRef) return;
        const { scrollTop, scrollHeight, clientHeight } = scrollRef;
        autoScroll = scrollHeight - scrollTop - clientHeight < 200;

        // Trigger older-history load when near the top
        if (
            onLoadOlder &&
            scrollTop < 50 &&
            !loadingOlderInFlight &&
            !(loadingOlder?.())
        ) {
            // Snapshot scroll anchor before content is prepended
            const snapshotScrollHeight = scrollHeight;
            const snapshotScrollTop = scrollTop;
            loadingOlderInFlight = true;
            onLoadOlder().then(() => {
                // Restore scroll position so the user's viewport doesn't jump.
                // Use a RAF so the DOM has updated with the new nodes first.
                requestAnimationFrame(() => {
                    if (scrollRef) {
                        scrollRef.scrollTop =
                            scrollRef.scrollHeight - snapshotScrollHeight + snapshotScrollTop;
                    }
                    loadingOlderInFlight = false;
                });
            }).catch(() => {
                loadingOlderInFlight = false;
            });
        }
    };

    return (
        <div
            class="agent-document"
            ref={scrollRef}
            onScroll={handleScroll}
        >
            {/* Older-history loading indicator — pinned at the very top */}
            <Show when={loadingOlder?.()}>
                <div class="agent-history-loading">Loading older messages...</div>
            </Show>

            {/* OAuth code-paste box, rendered at top of conversation while a
                login flow has a pending auth URL. Previously nested inside
                `.agent-status-log` with the activity log lines; the log
                moved to `<ActivityLogPanel>` above the composer, so the
                auth box stays here on its own. */}
            <Show when={authUrl?.()}>
                {(url) => {
                    const [pasteCode, setPasteCode] = createSignal("");
                    const [pasting, setPasting] = createSignal(false);
                    const [pasteResult, setPasteResult] = createSignal<string | null>(null);

                    const handleSubmitCode = async () => {
                        const code = pasteCode().trim();
                        if (!code) return;
                        setPasting(true);
                        setPasteResult(null);
                        try {
                            const { getApi } = await import("@/app/store/global");
                            await getApi().setProviderAuth(authProviderId ?? "claude", code);
                            setPasteResult("Code accepted — waiting for confirmation...");
                            setPasteCode("");
                        } catch (err: any) {
                            setPasteResult(`Error: ${err?.message ?? String(err)}`);
                        } finally {
                            setPasting(false);
                        }
                    };

                    return (
                        <div class="agent-auth-url-box">
                            <div class="agent-auth-url-label">Open this URL to log in:</div>
                            <div class="agent-auth-url-row">
                                <span class="agent-auth-url-text">{url()}</span>
                                <button
                                    class="agent-auth-url-copy"
                                    onClick={() => { import("@/util/clipboard").then(c => c.writeText(url())); }}
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
                                    onClick={handleSubmitCode}
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
                }}
            </Show>

            {/* Document nodes render below log lines.
                `content-visibility: auto` on the wrapper lets the browser
                skip layout/paint for off-screen nodes — critical for
                long sessions where thousands of DOM elements accumulate. */}
            <For each={document()}>
                {(node) => {
                    const isBookmarked = () => bookmarkedNodeIds?.().has(node.id) ?? false;
                    const canExpand = () => {
                        switch (node.type) {
                            case "tool":
                            case "agent_message":
                            case "user_message":
                            case "section":
                                return true;
                            default:
                                return false;
                        }
                    };
                    const isExpanded = () => {
                        if (node.type === "tool") return documentState().pinnedNodes.has(node.id);
                        return !documentState().collapsedNodes.has(node.id);
                    };
                    const onExpand = () => {
                        if (node.type === "tool") togglePin(node.id);
                        else toggleCollapse(node.id);
                    };
                    // Phase 6: "new pane" for tool rows — deferred until a scratch view exists.
                    const onOpenInNewPane = node.type === "tool"
                        ? () => console.warn("[hover-strip] open in new pane — not yet implemented")
                        : undefined;
                    const onOpenInNewWindow = () =>
                        console.warn("[hover-strip] open in new window — not yet implemented");
                    const onNewAgentFromHere = () =>
                        console.warn("[hover-strip] new agent from here — not yet implemented");

                    const handleRowKey = (e: KeyboardEvent): void => {
                        if (e.metaKey || e.ctrlKey || e.altKey) return;
                        switch (e.key.toLowerCase()) {
                            case "e":
                                if (canExpand()) { onExpand(); e.preventDefault(); }
                                break;
                            case "b":
                                if (onBookmark != null) { onBookmark(node); e.preventDefault(); }
                                break;
                            case "p":
                                if (onOpenInNewPane) { onOpenInNewPane(); e.preventDefault(); }
                                break;
                            case "w":
                                onOpenInNewWindow(); e.preventDefault();
                                break;
                            case "n":
                                onNewAgentFromHere(); e.preventDefault();
                                break;
                            case "escape":
                                (e.currentTarget as HTMLElement).blur();
                                e.preventDefault();
                                break;
                        }
                    };

                    const handleContextMenu = (e: MouseEvent) => {
                        if (!onBookmark) return;
                        // Don't show bookmark menu on top of text selections — let the parent handle those.
                        const sel = window.getSelection()?.toString();
                        if (sel) return;
                        e.preventDefault();
                        e.stopPropagation();
                        ContextMenuModel.showContextMenu(
                            [
                                {
                                    label: isBookmarked() ? "Remove bookmark" : "Bookmark this message",
                                    click: () => onBookmark(node),
                                },
                            ],
                            e,
                        );
                    };

                    return (
                        <div
                            class="hover-strip-host agent-document-node-wrapper"
                            classList={{
                                "agent-node-bookmarked": isBookmarked(),
                                "agent-node-search-match": highlightNodeId?.() === node.id,
                            }}
                            data-node-id={node.id}
                            tabindex="0"
                            onKeyDown={handleRowKey}
                            onContextMenu={handleContextMenu}
                        >
                            <DocumentNodeRenderer
                                node={node}
                                collapsed={documentState().collapsedNodes.has(node.id)}
                                onToggle={() => toggleCollapse(node.id)}
                                toolPinned={documentState().pinnedNodes.has(node.id)}
                                onToggleToolPin={() => togglePin(node.id)}
                                onSubagentClick={onSubagentClick}
                            />
                            <NodeHoverStrip
                                timestamp={(node as any).timestamp}
                                nodeId={node.id}
                                isBookmarked={isBookmarked()}
                                onBookmark={onBookmark != null ? () => onBookmark(node) : undefined}
                                canExpand={canExpand()}
                                isExpanded={isExpanded()}
                                onExpand={onExpand}
                                onOpenInNewPane={onOpenInNewPane}
                                onOpenInNewWindow={onOpenInNewWindow}
                                onNewAgentFromHere={onNewAgentFromHere}
                            />
                        </div>
                    );
                }}
            </For>
        </div>
    );
};

AgentDocumentView.displayName = "AgentDocumentView";

// ── Node renderer ────────────────────────────────────────────────────────────

// SolidJS reactivity note: props are accessed via `props.X` and NEVER
// destructured in the parameter list. Destructuring captures mount-time
// values and breaks reactivity for any prop that changes without triggering
// the parent to re-create this component. Since the tool pin state
// (`toolPinned`) lives in documentState — separate from the document array
// that <For> keys off — pin toggles do NOT re-create DocumentNodeRenderer,
// so a destructured `toolPinned` would stay stale forever even though the
// child ToolBlock uses `props.pinned` correctly.
//
// See commit adcec38 for the first half of this fix (ToolBlock), and the
// reagent review on PR #346 that caught this upstream companion.

interface DocumentNodeRendererProps {
    node: DocumentNode;
    collapsed: boolean;
    onToggle: () => void;
    toolPinned: boolean;
    onToggleToolPin: () => void;
    onSubagentClick?: (node: SubagentLinkNode) => void;
}

const DocumentNodeRenderer = (props: DocumentNodeRendererProps): JSX.Element => {
    switch (props.node.type) {
        case "markdown":
            return <MarkdownBlock node={props.node} />;

        case "tool":
            return (
                <ToolBlock
                    node={props.node}
                    pinned={props.toolPinned}
                    onTogglePin={props.onToggleToolPin}
                />
            );

        case "agent_message":
            return (
                <AgentMessageBlock
                    node={props.node}
                    collapsed={props.collapsed}
                    onToggle={props.onToggle}
                />
            );

        case "user_message": {
            const node = props.node;
            return (
                <div
                    class="agent-user-message"
                    classList={{ "agent-user-message--collapsed": props.collapsed }}
                >
                    <div class="agent-user-message-content">
                        <pre>{node.message}</pre>
                    </div>
                </div>
            );
        }

        case "subagent_link":
            return (
                <SubagentLinkBlock
                    node={props.node}
                    onClick={props.onSubagentClick ?? (() => {})}
                />
            );

        case "section": {
            const node = props.node;
            return (
                <div class={`agent-section level-${node.level}`}>
                    <Show when={node.level === 1}>
                        <h1>{node.title}</h1>
                    </Show>
                    <Show when={node.level === 2}>
                        <h2>{node.title}</h2>
                    </Show>
                    <Show when={node.level === 3}>
                        <h3>{node.title}</h3>
                    </Show>
                </div>
            );
        }

        default:
            return null;
    }
};

DocumentNodeRenderer.displayName = "DocumentNodeRenderer";
