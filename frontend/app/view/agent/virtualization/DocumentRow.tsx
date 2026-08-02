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
import { AgentMessageBlock } from "../components/AgentMessageBlock";
import { JektBubble } from "../components/JektBubble";
import { MarkdownBlock } from "../components/MarkdownBlock";
import { PersistentShellBlock } from "../components/PersistentShellBlock";
import { ToolBlock } from "../components/ToolBlock";
import { UserMessageBlock } from "../components/UserMessageBlock";
import type { DocumentNode, DocumentState, ShellNode, UserMessageNode } from "../types";
import { markRowMount } from "./perf-probe";

export interface DocumentRowProps {
    /**
     * Reactive accessor for the current node at this row's slot. Must
     * be an accessor (not a value) so streaming updates that replace
     * the node at the same id propagate without remount.
     */
    node: Accessor<DocumentNode>;
    documentState: Accessor<DocumentState>;
    highlightNodeId?: Accessor<string | null>;
    onToggleCollapse: (id: string) => void;
    onTogglePin: (id: string) => void;
    /** Hold a tool expanded after it completes live on screen (ToolBlock calls
     *  this on the active→inactive transition). */
    onHoldToolOpen?: (id: string) => void;
    /** Re-run the provider login flow. Threaded down so an `agent_error` node
     *  carrying an auth status (401/403) can offer an inline "Login Again" CTA
     *  — the same action as the failure banner. See
     *  SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20 §7. */
    onAgentErrorLogin?: () => void;
    /** Style applied to the wrapper element. Virtualized parent
     *  passes absolute positioning + translateY; streaming parent
     *  passes nothing (normal flow). */
    style?: JSX.CSSProperties;
    /** Ref forwarded to the wrapper element. Virtualized parent passes the
     *  measure-RO observer (keyed by nodeId); streaming parent omits. */
    ref?: (el: HTMLElement) => void;
}

// Kinds whose hover-strip surfaces an Expand/Collapse control.
// `user_message` was here until PR #1020 — UserMessageBlock now owns
// its own collapse state (via `isStartup` + `documentState.pinnedNodes`,
// not `collapsedNodes`), so a hover-strip toggle here would have been a
// no-op control writing dead state. Toggling pin from the strip would
// also be confusing for normal typed input (which is never collapsible
// to begin with). Codex P2 on PR #1020.
const TOGGLEABLE_KINDS: ReadonlySet<DocumentNode["type"]> = new Set([
    "tool",
    "shell",
    "agent_message",
    "jekt_message",
    "section",
]);

export function DocumentRow(props: DocumentRowProps): JSX.Element {
    // Phase 3: per-kind row mount perf probe. markRowMount returns a
    // closer that we invoke after onMount fires (i.e., after the row
    // is in the DOM and its initial paint has been queued).
    // No-op in production builds.
    const close = markRowMount(props.node().type);
    onMount(() => close());

    const isSearchMatch = (): boolean => {
        return props.highlightNodeId?.() === props.node().id;
    };

    const canExpand = (): boolean => TOGGLEABLE_KINDS.has(props.node().type);

    const onExpand = (): void => {
        const n = props.node();
        if (n.type === "tool" || n.type === "shell") props.onTogglePin(n.id);
        else props.onToggleCollapse(n.id);
    };

    const handleRowKey = (e: KeyboardEvent): void => {
        if (e.metaKey || e.ctrlKey || e.altKey) return;
        const n = props.node();
        switch (e.key.toLowerCase()) {
            case "e":
                if (canExpand()) { onExpand(); e.preventDefault(); }
                break;
            case "escape":
                (e.currentTarget as HTMLElement).blur();
                e.preventDefault();
                break;
        }
    };

    return (
        <div
            ref={props.ref}
            class="agent-document-row"
            classList={{
                "agent-node-search-match": isSearchMatch(),
            }}
            data-node-id={props.node().id}
            tabindex="0"
            onKeyDown={handleRowKey}
            style={props.style}
        >
            <DocumentNodeBody
                node={props.node}
                documentState={props.documentState}
                onToggleCollapse={props.onToggleCollapse}
                onTogglePin={props.onTogglePin}
                onHoldToolOpen={props.onHoldToolOpen}
                onAgentErrorLogin={props.onAgentErrorLogin}
            />
        </div>
    );
}

interface DocumentNodeBodyProps {
    node: Accessor<DocumentNode>;
    documentState: Accessor<DocumentState>;
    onToggleCollapse: (id: string) => void;
    onTogglePin: (id: string) => void;
    onHoldToolOpen?: (id: string) => void;
    /** Re-run the provider login flow — drives the inline auth-error CTA. */
    onAgentErrorLogin?: () => void;
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
                    heldOpen={props.documentState().expandedTools.has(props.node().id)}
                    onTogglePin={() => props.onTogglePin(props.node().id)}
                    onHoldOpen={() => props.onHoldToolOpen?.(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "agent_message"}>
                <AgentMessageBlock
                    node={props.node() as Extract<DocumentNode, { type: "agent_message" }>}
                    collapsed={props.documentState().collapsedNodes.has(props.node().id)}
                    onToggle={() => props.onToggleCollapse(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "jekt_message"}>
                <JektBubble
                    node={props.node() as Extract<DocumentNode, { type: "jekt_message" }>}
                    collapsed={props.documentState().collapsedNodes.has(props.node().id)}
                    onToggle={() => props.onToggleCollapse(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "user_message"}>
                <UserMessageBlock
                    node={props.node() as UserMessageNode}
                    pinned={props.documentState().pinnedNodes.has(props.node().id)}
                    onTogglePin={() => props.onTogglePin(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "shell"}>
                <PersistentShellBlock
                    node={props.node() as ShellNode}
                    pinned={props.documentState().pinnedNodes.has(props.node().id)}
                    onTogglePin={() => props.onTogglePin(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "section"}>
                {/* Section header toggles its own collapse on click (the
                    expand affordance the removed hover strip used to provide);
                    keyboard "e" on the focused row still toggles too. */}
                <div
                    class={`agent-section agent-section--toggle level-${(props.node() as Extract<DocumentNode, { type: "section" }>).level}`}
                    onClick={() => props.onToggleCollapse(props.node().id)}
                >
                    <Show when={(props.node() as Extract<DocumentNode, { type: "section" }>).level === 1}>
                        <h1>{(props.node() as Extract<DocumentNode, { type: "section" }>).title}</h1>
                    </Show>
                    <Show when={(props.node() as Extract<DocumentNode, { type: "section" }>).level === 2}>
                        <h2>{(props.node() as Extract<DocumentNode, { type: "section" }>).title}</h2>
                    </Show>
                    <Show when={(props.node() as Extract<DocumentNode, { type: "section" }>).level === 3}>
                        <h3>{(props.node() as Extract<DocumentNode, { type: "section" }>).title}</h3>
                    </Show>
                </div>
            </Show>
            <Show when={props.node() && props.node().type === "agent_error"}>
                <div class="agent-error-block">
                    <span class="agent-error-code">
                        {(() => {
                            const n = props.node() as Extract<DocumentNode, { type: "agent_error" }>;
                            // code=0 is a sentinel for non-HTTP errors (network/CLI); don't show "HTTP 0"
                            return n.code > 0 ? `HTTP ${n.code}` : "Error";
                        })()}
                    </span>
                    <span class="agent-error-message">
                        {(props.node() as Extract<DocumentNode, { type: "agent_error" }>).message}
                    </span>
                    {/* Auth errors (401 Unauthorized / 403 Forbidden) are recoverable
                        by re-running the provider login — surface an inline CTA that
                        drives the same flow as the failure banner. Other codes have no
                        in-place fix, so no button. SPEC_REAUTH_FROM_AUTH_ERROR §7. */}
                    <Show when={
                        props.onAgentErrorLogin
                        && [401, 403].includes((props.node() as Extract<DocumentNode, { type: "agent_error" }>).code)
                    }>
                        <button
                            type="button"
                            class="agent-error-login-btn"
                            onClick={() => props.onAgentErrorLogin?.()}
                        >
                            Login Again →
                        </button>
                    </Show>
                </div>
            </Show>
            <Show when={props.node() && props.node().type === "context_compacted"}>
                {(() => {
                    const n = props.node() as Extract<DocumentNode, { type: "context_compacted" }>;
                    const fmt = (tok: number) => tok >= 1000 ? `${Math.round(tok / 1000)}k` : String(tok);
                    // Real events (backend `CompactionBoundary`) carry a real
                    // trigger + duration; the heuristic fallback (other
                    // providers, or a missed real event) has neither. See
                    // docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md §4.3.
                    const triggerLabel = n.trigger === "manual"
                        ? "you ran /compact"
                        : n.trigger === "auto"
                            ? "auto-compacted"
                            : null;
                    const durationLabel = n.durationMs != null
                        ? ` · took ${(n.durationMs / 1000).toFixed(1)}s`
                        : "";
                    return (
                        <div class="agent-context-compacted">
                            <div class="agent-context-compacted-label">
                                context compacted{triggerLabel ? ` — ${triggerLabel}` : ""}
                            </div>
                            <div class="agent-context-compacted-detail">
                                Earlier history summarized · {fmt(n.tokensBefore)} → {fmt(n.tokensAfter)} tokens{durationLabel}
                            </div>
                        </div>
                    );
                })()}
            </Show>
            <Show when={props.node() && props.node().type === "compaction_started"}>
                {(() => {
                    const n = props.node() as Extract<DocumentNode, { type: "compaction_started" }>;
                    const triggerLabel = n.trigger === "manual" ? "you ran /compact" : "context filled up";
                    return (
                        <div class="agent-compaction-started">
                            <div class="agent-compaction-started-label">
                                Compacting conversation…
                            </div>
                            <div class="agent-compaction-started-detail">
                                {triggerLabel}
                            </div>
                        </div>
                    );
                })()}
            </Show>
        </>
    );
}
