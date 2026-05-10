// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DocumentRow — single row in the agent document list. Used by both
 * the virtualized region and the unvirtualized streaming buffer in
 * AgentDocumentVirtualList.
 *
 * Caller controls positioning (style + ref) so the same row works in
 * absolute-positioned virtualizer slots and in the normal-flow
 * streaming buffer without the row knowing the difference.
 *
 * Phase 2 of the virtualization redesign — see
 * docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md.
 */

import { onMount, Show, type Accessor, type JSX } from "solid-js";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { AgentMessageBlock } from "../components/AgentMessageBlock";
import { MarkdownBlock } from "../components/MarkdownBlock";
import { NodeHoverStrip } from "../components/NodeHoverStrip";
import { SubagentLinkBlock } from "../components/SubagentLinkBlock";
import { ToolBlock } from "../components/ToolBlock";
import type { DocumentNode, DocumentState, SubagentLinkNode } from "../types";
import { markRowMount } from "./perf-probe";

export interface DocumentRowProps {
    /**
     * Reactive accessor for the current node at this row's slot. Must
     * be an accessor (not a value) so streaming updates that replace
     * the node at the same id propagate without remount.
     */
    node: Accessor<DocumentNode>;
    documentState: Accessor<DocumentState>;
    bookmarkedNodeIds?: Accessor<Set<string>>;
    onBookmark?: (node: DocumentNode) => void;
    onSubagentClick?: (node: SubagentLinkNode) => void;
    highlightNodeId?: Accessor<string | null>;
    onToggleCollapse: (id: string) => void;
    onTogglePin: (id: string) => void;
    /** Style applied to the wrapper element. Virtualized parent
     *  passes absolute positioning + translateY; streaming parent
     *  passes nothing (normal flow). */
    style?: JSX.CSSProperties;
    /** Ref forwarded to the wrapper element. Virtualized parent
     *  passes virtualizer.measureElement; streaming parent omits. */
    ref?: (el: HTMLElement) => void;
    /**
     * Virtual item index — set as `data-index` on the wrapper. TanStack
     * Virtual's measureElement requires this attribute to match
     * measurements back to virtual items. Streaming parent omits.
     * (codex P1 on #784.)
     */
    dataIndex?: number;
}

const TOGGLEABLE_KINDS: ReadonlySet<DocumentNode["type"]> = new Set([
    "tool",
    "agent_message",
    "user_message",
    "section",
]);

export function DocumentRow(props: DocumentRowProps): JSX.Element {
    // Phase 3: per-kind row mount perf probe. markRowMount returns a
    // closer that we invoke after onMount fires (i.e., after the row
    // is in the DOM and its initial paint has been queued).
    // No-op in production builds.
    const close = markRowMount(props.node().type);
    onMount(() => close());

    const isBookmarked = (): boolean => {
        const n = props.node();
        return props.bookmarkedNodeIds?.().has(n.id) ?? false;
    };

    const isSearchMatch = (): boolean => {
        return props.highlightNodeId?.() === props.node().id;
    };

    const canExpand = (): boolean => TOGGLEABLE_KINDS.has(props.node().type);

    const isExpanded = (): boolean => {
        const n = props.node();
        const state = props.documentState();
        if (n.type === "tool") return state.pinnedNodes.has(n.id);
        return !state.collapsedNodes.has(n.id);
    };

    const onExpand = (): void => {
        const n = props.node();
        if (n.type === "tool") props.onTogglePin(n.id);
        else props.onToggleCollapse(n.id);
    };

    // Phase 6 placeholders — match existing AgentDocumentView contract.
    const onOpenInNewPane = (): void => {
        if (props.node().type === "tool") {
            console.warn("[hover-strip] open in new pane — not yet implemented");
        }
    };
    const onOpenInNewWindow = (): void =>
        console.warn("[hover-strip] open in new window — not yet implemented");
    const onNewAgentFromHere = (): void =>
        console.warn("[hover-strip] new agent from here — not yet implemented");

    const handleRowKey = (e: KeyboardEvent): void => {
        if (e.metaKey || e.ctrlKey || e.altKey) return;
        const n = props.node();
        switch (e.key.toLowerCase()) {
            case "e":
                if (canExpand()) { onExpand(); e.preventDefault(); }
                break;
            case "b":
                if (props.onBookmark != null) { props.onBookmark(n); e.preventDefault(); }
                break;
            case "p":
                if (n.type === "tool") { onOpenInNewPane(); e.preventDefault(); }
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

    const handleContextMenu = (e: MouseEvent): void => {
        if (!props.onBookmark) return;
        // Don't shadow text-selection menus — let the parent pane handle those.
        const sel = window.getSelection()?.toString();
        if (sel) return;
        e.preventDefault();
        e.stopPropagation();
        const n = props.node();
        ContextMenuModel.showContextMenu(
            [
                {
                    label: isBookmarked() ? "Remove bookmark" : "Bookmark this message",
                    click: () => props.onBookmark?.(n),
                },
            ],
            e,
        );
    };

    return (
        <div
            ref={props.ref}
            class="hover-strip-host agent-document-row"
            classList={{
                "agent-node-bookmarked": isBookmarked(),
                "agent-node-search-match": isSearchMatch(),
            }}
            data-node-id={props.node().id}
            data-index={props.dataIndex}
            tabindex="0"
            onKeyDown={handleRowKey}
            onContextMenu={handleContextMenu}
            style={props.style}
        >
            <DocumentNodeBody
                node={props.node}
                documentState={props.documentState}
                onToggleCollapse={props.onToggleCollapse}
                onTogglePin={props.onTogglePin}
                onSubagentClick={props.onSubagentClick}
            />
            <NodeHoverStrip
                timestamp={(props.node() as { timestamp?: number }).timestamp}
                nodeId={props.node().id}
                isBookmarked={isBookmarked()}
                onBookmark={props.onBookmark != null ? () => props.onBookmark!(props.node()) : undefined}
                canExpand={canExpand()}
                isExpanded={isExpanded()}
                onExpand={onExpand}
                onOpenInNewPane={props.node().type === "tool" ? onOpenInNewPane : undefined}
                onOpenInNewWindow={onOpenInNewWindow}
                onNewAgentFromHere={onNewAgentFromHere}
            />
        </div>
    );
}

interface DocumentNodeBodyProps {
    node: Accessor<DocumentNode>;
    documentState: Accessor<DocumentState>;
    onToggleCollapse: (id: string) => void;
    onTogglePin: (id: string) => void;
    onSubagentClick?: (node: SubagentLinkNode) => void;
}

/**
 * Routes a DocumentNode to the right block component. Each branch
 * accesses props.node() reactively so streaming updates and state-set
 * toggles propagate without remount.
 *
 * Reactivity discipline (carried over from the original
 * DocumentNodeRenderer comment): never destructure props in this
 * function. The toolPinned state lives in documentState (separate from
 * the document array that the parent <For> keys off), so a
 * destructured `toolPinned` would stay stale forever even though
 * ToolBlock uses props.pinned correctly. See PR #346 reagent.
 */
function DocumentNodeBody(props: DocumentNodeBodyProps): JSX.Element {
    // SolidJS <Show> with keyed:false (the default) calls the child
    // factory ONCE when `when` first becomes truthy and never re-runs
    // it. The previous version of this function was:
    //
    //   <Show when={props.node()}>{(n) => { const node = n(); switch ...}}
    //
    // which captured a one-time `node = n()` snapshot. Streaming token
    // updates and tool transitions (running → success, etc.) silently
    // failed to propagate to ToolBlock / AgentMessageBlock until the
    // row remounted — defeating the whole streaming buffer.
    //
    // Use separate <Show when={node.type === "X"}> branches per kind.
    // Each branch reads props.node() reactively at every prop site, so
    // the matching branch's children update when the underlying node
    // changes. This was committed on the Phase 2 PR but landed after
    // the squash-merge cutoff — adding back as a follow-up.
    return (
        <>
            <Show when={props.node() && props.node().type === "markdown"}>
                <MarkdownBlock node={props.node() as Extract<DocumentNode, { type: "markdown" }>} />
            </Show>
            <Show when={props.node() && props.node().type === "tool"}>
                <ToolBlock
                    node={props.node() as Extract<DocumentNode, { type: "tool" }>}
                    pinned={props.documentState().pinnedNodes.has(props.node().id)}
                    onTogglePin={() => props.onTogglePin(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "agent_message"}>
                <AgentMessageBlock
                    node={props.node() as Extract<DocumentNode, { type: "agent_message" }>}
                    collapsed={props.documentState().collapsedNodes.has(props.node().id)}
                    onToggle={() => props.onToggleCollapse(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "user_message"}>
                <div
                    class="agent-user-message"
                    classList={{
                        "agent-user-message--collapsed":
                            props.documentState().collapsedNodes.has(props.node().id),
                    }}
                >
                    <div class="agent-user-message-content">
                        <pre>{(props.node() as Extract<DocumentNode, { type: "user_message" }>).message}</pre>
                    </div>
                </div>
            </Show>
            <Show when={props.node() && props.node().type === "subagent_link"}>
                <SubagentLinkBlock
                    node={props.node() as Extract<DocumentNode, { type: "subagent_link" }>}
                    onClick={props.onSubagentClick ?? (() => { })}
                />
            </Show>
            <Show when={props.node() && props.node().type === "section"}>
                {(() => {
                    const sec = props.node() as Extract<DocumentNode, { type: "section" }>;
                    return (
                        <div class={`agent-section level-${sec.level}`}>
                            <Show when={sec.level === 1}><h1>{sec.title}</h1></Show>
                            <Show when={sec.level === 2}><h2>{sec.title}</h2></Show>
                            <Show when={sec.level === 3}><h3>{sec.title}</h3></Show>
                        </div>
                    );
                })()}
            </Show>
        </>
    );
}
