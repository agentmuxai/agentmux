// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentView — Renders the styled document as a list of DocumentNodes.
 * Routes each node type to the appropriate block component.
 * When no document nodes exist yet, shows accumulated log lines (terminal-style).
 */

import { createEffect, For, Show, type Accessor, type JSX, onCleanup } from "solid-js";
import { getApi } from "@/app/store/global";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState, SubagentLinkNode } from "../types";
import { AgentMessageBlock } from "./AgentMessageBlock";
import { MarkdownBlock } from "./MarkdownBlock";
import { SubagentLinkBlock } from "./SubagentLinkBlock";
import { ToolBlock } from "./ToolBlock";

export interface LogLine {
    tag: string;        // "agent", "cli", "auth", "env", "error", etc.
    text: string;
    level?: "info" | "error" | "warn";
}

interface AgentDocumentViewProps {
    documentAtom: SignalPair<DocumentNode[]>;
    documentStateAtom: SignalPair<DocumentState>;
    logLines: Accessor<LogLine[]>;
    authUrl?: Accessor<string | null>;
    onSubagentClick?: (node: SubagentLinkNode) => void;
}

export const AgentDocumentView = ({ documentAtom, documentStateAtom, logLines, authUrl, onSubagentClick }: AgentDocumentViewProps): JSX.Element => {
    const [document] = documentAtom;
    const [documentState, setDocumentState] = documentStateAtom;
    let scrollRef!: HTMLDivElement;
    let autoScroll = true;

    // Toggle collapsed state for a node
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

    // Auto-scroll to bottom when new content arrives.
    // Use MutationObserver to scroll AFTER the DOM has updated.
    let observer: MutationObserver | null = null;

    const scrollToBottom = () => {
        if (autoScroll && scrollRef) {
            scrollRef.scrollTop = scrollRef.scrollHeight;
        }
    };

    // Also trigger on signal changes (for initial content)
    createEffect(() => {
        const _docLen = document().length;
        const _logLen = logLines().length;
        // Defer to next frame so DOM is updated
        requestAnimationFrame(scrollToBottom);
    });

    // Watch for any DOM mutations inside the scroll container
    // This catches content appended to existing nodes (text streaming)
    createEffect(() => {
        if (!scrollRef) return;
        observer = new MutationObserver(() => {
            if (autoScroll) {
                scrollRef.scrollTop = scrollRef.scrollHeight;
            }
        });
        observer.observe(scrollRef, { childList: true, subtree: true, characterData: true });
        onCleanup(() => observer?.disconnect());
    });

    // Detect if user scrolled up (disable auto-scroll)
    const handleScroll = () => {
        if (!scrollRef) return;
        const { scrollTop, scrollHeight, clientHeight } = scrollRef;
        autoScroll = scrollHeight - scrollTop - clientHeight < 50;
    };

    return (
        <div class="agent-document" ref={scrollRef} onScroll={handleScroll}>
            {/* Log lines always shown at the top */}
            <Show when={logLines().length > 0}>
                <div class="agent-status-log">
                    <For each={logLines()}>
                        {(line) => (
                            <div
                                class="agent-status-line"
                                classList={{
                                    "agent-status-line--error": line.level === "error",
                                    "agent-status-line--warn": line.level === "warn",
                                }}
                            >
                                <span class="agent-status-tag">[{line.tag}]</span> {line.text}
                            </div>
                        )}
                    </For>
                    <Show when={authUrl?.()}>
                        {(url) => {
                            let codeInput: HTMLInputElement | undefined;
                            return (
                                <div class="agent-auth-url-box">
                                    <div class="agent-auth-url-label">Login URL (if browser didn't open):</div>
                                    <div class="agent-auth-url-row">
                                        <span class="agent-auth-url-text">{url()}</span>
                                        <button
                                            class="agent-auth-url-copy"
                                            onClick={() => navigator.clipboard.writeText(url())}
                                            title="Copy URL"
                                        >
                                            Copy
                                        </button>
                                    </div>
                                    <div class="agent-auth-url-label" style={{ "margin-top": "8px" }}>
                                        Authentication Code (paste here if prompted):
                                    </div>
                                    <div class="agent-auth-url-row">
                                        <input
                                            ref={codeInput}
                                            class="agent-auth-code-input"
                                            type="text"
                                            placeholder="Paste authentication code..."
                                            onKeyDown={(e) => {
                                                if (e.key === "Enter" && codeInput?.value) {
                                                    getApi().writeCliLoginStdin(codeInput.value).catch(() => {});
                                                    codeInput.value = "";
                                                }
                                            }}
                                        />
                                        <button
                                            class="agent-auth-url-copy"
                                            onClick={() => {
                                                if (codeInput?.value) {
                                                    getApi().writeCliLoginStdin(codeInput.value).catch(() => {});
                                                    codeInput.value = "";
                                                }
                                            }}
                                        >
                                            Submit
                                        </button>
                                    </div>
                                </div>
                            );
                        }}
                    </Show>
                </div>
            </Show>

            {/* Document nodes render below log lines */}
            <For each={document()}>
                {(node) => (
                    <DocumentNodeRenderer
                        node={node}
                        collapsed={documentState().collapsedNodes.has(node.id)}
                        onToggle={() => toggleCollapse(node.id)}
                        onSubagentClick={onSubagentClick}
                    />
                )}
            </For>
        </div>
    );
};

AgentDocumentView.displayName = "AgentDocumentView";

// ── Node renderer ────────────────────────────────────────────────────────────

const DocumentNodeRenderer = ({
    node,
    collapsed,
    onToggle,
    onSubagentClick,
}: {
    node: DocumentNode;
    collapsed: boolean;
    onToggle: () => void;
    onSubagentClick?: (node: SubagentLinkNode) => void;
}): JSX.Element => {
    switch (node.type) {
        case "markdown":
            return <MarkdownBlock node={node} />;

        case "tool":
            return <ToolBlock node={node} collapsed={collapsed} onToggle={onToggle} />;

        case "agent_message":
            return <AgentMessageBlock node={node} collapsed={collapsed} onToggle={onToggle} />;

        case "user_message":
            return (
                <div class="agent-user-message">
                    <div class="agent-user-message-content">
                        <pre>{node.message}</pre>
                    </div>
                </div>
            );

        case "subagent_link":
            return <SubagentLinkBlock node={node} onClick={onSubagentClick ?? (() => {})} />;

        case "section":
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

        default:
            return null;
    }
};

DocumentNodeRenderer.displayName = "DocumentNodeRenderer";
