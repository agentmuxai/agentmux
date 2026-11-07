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

import { catchError, onMount, Show, createMemo, untrack, type Accessor, type JSX } from "solid-js";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { AgentMessageBlock } from "../components/AgentMessageBlock";
import { OriginContainer } from "../components/OriginContainer";
import { HiddenRunBlock } from "../components/HiddenRunBlock";
import { AmbientNarrationBlock } from "../components/AmbientNarrationBlock";
import { MarkdownBlock } from "../components/MarkdownBlock";
import { PersistentShellBlock } from "../components/PersistentShellBlock";
import { ToolBlock } from "../components/ToolBlock";
import { rowDisclosureIn } from "./disclosure";
import { AgentErrorRow, CliNoticeRow, CompactionStartedRow, ContextCompactedRow, DayDividerRow, HistoryLinkRow, MemoryReinjectionRow, ResumePreflightRow, SessionOutcomeRow, type RowPeek } from "./InlineNodeRows";
import { ContextDeliveryCard } from "../components/ContextDeliveryCard";
import { SetupNote } from "../components/SetupNote";
import { UserMessageBlock } from "../components/UserMessageBlock";
import { useNodePeek } from "../hooks/useNodePeek";
import type { ContextDeliveryNode, DocumentNode, DocumentState, ShellNode, UserMessageNode } from "../../../agent-model/types";
import { markRowMount } from "../../../agent-model/perf-probe";
import { useTick } from "@/app/hook/useTick";

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
    onHoldNodeOpen?: (id: string) => void;
    /** Re-run the provider login flow. Threaded down so an `agent_error` node
     *  carrying an auth status (401/403) can offer an inline "Login Again" CTA
     *  — the same action as the failure banner. See
     *  SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20 §7. */
    onAgentErrorLogin?: () => void;
    /** Open (or focus, if already open) the pane's Agent History tab —
     *  threaded down so a `history_link` synthetic row can act on click.
     *  See SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.2. */
    onOpenHistory?: () => void;
    /** Style applied to the wrapper element. Virtualized parent
     *  passes absolute positioning + translateY; streaming parent
     *  passes nothing (normal flow). */
    style?: JSX.CSSProperties;
    /** Ref forwarded to the wrapper element. Virtualized parent passes the
     *  measure-RO observer (keyed by nodeId); streaming parent omits. */
    ref?: (el: HTMLElement) => void;
    /** Ordinal-matched tool_use_id -> live dispatch, for this pane's
     *  Agent/Task/Workflow tool nodes. See `activity/dispatch-correlation.ts`. */
    dispatchMatches?: Accessor<Map<string, AgentDispatch>>;
}

/** Flip whichever set the shared rule names for this row (`pinnedNodes` or
 *  `collapsedNodes`); nothing for a fixed row. The row's click and the `e` key. */
function flipRow(
    node: DocumentNode,
    state: DocumentState,
    on: { onTogglePin: (id: string) => void; onToggleCollapse: (id: string) => void },
): void {
    const t = rowDisclosureIn(node, state).toggle;
    if (t === "pin") on.onTogglePin(node.id);
    else if (t === "collapse") on.onToggleCollapse(node.id);
}

/** The last time each distinct row error was logged, so a row that throws on
 *  every store write doesn't flood the log. */
const rowErrorLoggedAt = new Map<string, number>();
const ROW_ERROR_LOG_EVERY_MS = 10_000;

/** Log a row's render error, naming the row, at most once per message per 10 s. */
export function logRowError(kind: string, id: string, err: unknown): void {
    const message = err instanceof Error ? err.message : String(err);
    const key = `${kind}:${message}`;
    const now = Date.now();
    if (now - (rowErrorLoggedAt.get(key) ?? 0) < ROW_ERROR_LOG_EVERY_MS) return;
    rowErrorLoggedAt.set(key, now);
    console.error(`[agent-row] render error in a ${kind} row (${id}): ${message}`, err);
}

/**
 * A row whose computations throw while the document store updates (Solid's
 * "Stale read from <Show>." is one) must not take the rest of the update with
 * it: an error nothing handles aborts the whole reactive flush, and every
 * effect queued in it, in any pane of the window, is left stale and never
 * runs again (a frozen window tab, a bound style that stops following).
 * The handler scopes it to the row and says which row.
 */
export function DocumentRow(props: DocumentRowProps): JSX.Element {
    return catchError(
        () => DocumentRowBody(props),
        (err) => {
            let kind = "unknown";
            let id = "?";
            try {
                const node = untrack(props.node);
                kind = node.type;
                id = node.id;
            } catch {
                // The row's own node accessor is what threw.
            }
            logRowError(kind, id, err);
        }
    );
}

function DocumentRowBody(props: DocumentRowProps): JSX.Element {
    // Phase 3: per-kind row mount perf probe. markRowMount returns a
    // closer that we invoke after onMount fires (i.e., after the row
    // is in the DOM and its initial paint has been queued).
    // No-op in production builds.
    const close = markRowMount(props.node().type);
    onMount(() => close());

    const isSearchMatch = (): boolean => {
        return props.highlightNodeId?.() === props.node().id;
    };

    // The `e` key flips the same set the row's own header click does: the
    // shared rule says which (virtualization/disclosure.ts), or that the row
    // isn't toggleable at all (normal user input, fixed rows).
    const canExpand = (): boolean => rowDisclosureIn(props.node(), props.documentState()).toggle !== null;
    const onExpand = (): void => flipRow(props.node(), props.documentState(), props);

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
            // The kind of node, for the stylesheet (the user message has its own
            // enter motion) and for code that must read it from the DOM, without
            // a reactive read inside a ResizeObserver callback.
            data-node-type={props.node().type}
            tabindex="0"
            onKeyDown={handleRowKey}
            style={props.style}
        >
            <DocumentNodeBody
                node={props.node}
                documentState={props.documentState}
                onToggleCollapse={props.onToggleCollapse}
                onTogglePin={props.onTogglePin}
                onHoldNodeOpen={props.onHoldNodeOpen}
                onAgentErrorLogin={props.onAgentErrorLogin}
                onOpenHistory={props.onOpenHistory}
                dispatchMatches={props.dispatchMatches}
            />
        </div>
    );
}

interface DocumentNodeBodyProps {
    node: Accessor<DocumentNode>;
    documentState: Accessor<DocumentState>;
    onToggleCollapse: (id: string) => void;
    onTogglePin: (id: string) => void;
    onHoldNodeOpen?: (id: string) => void;
    /** Re-run the provider login flow — drives the inline auth-error CTA. */
    onAgentErrorLogin?: () => void;
    /** Open/focus the Agent History tab — drives the `history_link` row's click. */
    onOpenHistory?: () => void;
    /** Ordinal-matched tool_use_id -> live dispatch, for this pane's
     *  Agent/Task/Workflow tool nodes. See `activity/dispatch-correlation.ts`. */
    dispatchMatches?: Accessor<Map<string, AgentDispatch>>;
}

/** What the node body and the functions that serve it share. */
interface NodeBodyState {
    props: DocumentNodeBodyProps;
    disclosure: () => ReturnType<typeof rowDisclosureIn>;
    onExpand: () => void;
    peek: RowPeek;
    dispatchMatch: Accessor<AgentDispatch | undefined>;
}

function createNodeBodyState(props: DocumentNodeBodyProps): NodeBodyState {
    // The message rows (agent message, jekt, context delivery) take their open
    // state and their click from the shared rule, as the `e` key does.
    const disclosure = () => rowDisclosureIn(props.node(), props.documentState());
    const onExpand = (): void => flipRow(props.node(), props.documentState(), props);
    // Peek tooltip for the node kinds still rendered INLINE below
    // (section/agent_error/context_compacted/compaction_started/
    // day_divider/session_outcome) — SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25.
    // markdown/tool/agent_message/jekt_message/user_message/shell own their
    // own peek state inside their dedicated components; these six don't
    // have one, so a SINGLE shared instance here covers all of them — only
    // one `<Show>` branch (and hence only one of these six anchors) is ever
    // actually mounted for a given node, so sharing is safe.
    const peekTick = useTick(1000);
    const { isPeeking, panelVisible: peekPanelVisible, rowEl: peekRowEl, setRowEl: setPeekRowEl, handlePeekEnter, handlePeekLeave } = useNodePeek();

    const peek: RowPeek = { isPeeking, peekPanelVisible, peekRowEl, setPeekRowEl, handlePeekEnter, handlePeekLeave, peekTick };

    // This row's dispatch match, equality-gated. `dispatchMatches` is a memo
    // over the WHOLE document array (AgentDocumentView.tsx), so it produces a
    // new Map identity on every stream flush. Passing `.get(id)` as an inline
    // prop getter subscribed every consumer down the chain (ToolBlock →
    // ToolBlockOverlay → ToolOverlayLog → ToolOverlayResult's `<Show>`
    // children memo) to that Map identity — so `renderToolResultBody` re-ran
    // and rebuilt the entire result subtree for EVERY finished tool in the
    // streaming buffer, EVERY flush, while the value it read was unchanged
    // (`undefined` for anything but Agent/Task/Workflow). For `.md` Read
    // previews that meant a full markdown parse plus an OverlayScrollbars
    // construction — 46% + 38% of `flushPendingNodes` under 4-pane load.
    // A memo compares the resolved value (`===`), so identity churn on the
    // Map stops here. See ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md §2.
    const dispatchMatch = createMemo<AgentDispatch | undefined>(() => {
        const n = props.node();
        return n ? props.dispatchMatches?.().get(n.id) : undefined;
    });

    return { props, disclosure, onExpand, peek, dispatchMatch };
}

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

function renderMessageRows(s: NodeBodyState): JSX.Element {
    const { props } = s;
    return (
        <>
            <Show when={props.node() && props.node().type === "markdown"}>
                <MarkdownBlock
                    node={props.node() as Extract<DocumentNode, { type: "markdown" }>}
                    pinned={props.documentState().pinnedNodes.has(props.node().id)}
                    onTogglePin={() => props.onTogglePin(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "hidden_run"}>
                <HiddenRunBlock
                    node={props.node() as Extract<DocumentNode, { type: "hidden_run" }>}
                    onToggle={() => props.onTogglePin(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "ambient_narration"}>
                <AmbientNarrationBlock node={props.node() as Extract<DocumentNode, { type: "ambient_narration" }>} />
            </Show>
            <Show when={props.node() && props.node().type === "tool"}>
                <ToolBlock
                    node={props.node() as Extract<DocumentNode, { type: "tool" }>}
                    pinned={props.documentState().pinnedNodes.has(props.node().id)}
                    heldOpen={props.documentState().heldOpenNodes.has(props.node().id)}
                    onTogglePin={() => props.onTogglePin(props.node().id)}
                    onHoldOpen={() => props.onHoldNodeOpen?.(props.node().id)}
                    userCollapsed={props.documentState().collapsedNodes.has(props.node().id)}
                    onToggleCollapse={() => props.onToggleCollapse(props.node().id)}
                    dispatchMatch={s.dispatchMatch()}
                />
            </Show>
            <Show when={props.node() && props.node().type === "agent_message"}>
                <AgentMessageBlock
                    node={props.node() as Extract<DocumentNode, { type: "agent_message" }>}
                    collapsed={!s.disclosure().open}
                    onToggle={s.onExpand}
                />
            </Show>
            <Show when={props.node() && props.node().type === "jekt_message"}>
                <OriginContainer
                    node={props.node() as Extract<DocumentNode, { type: "jekt_message" }>}
                    collapsed={!s.disclosure().open}
                    onToggle={s.onExpand}
                    onHoldOpen={() => props.onHoldNodeOpen?.(props.node().id)}
                />
            </Show>
            <Show when={props.node() && props.node().type === "system_note"}>
                <OriginContainer node={props.node() as Extract<DocumentNode, { type: "system_note" }>} />
            </Show>
            <Show when={props.node() && props.node().type === "context_delivery"}>
                <ContextDeliveryCard
                    node={props.node() as ContextDeliveryNode}
                    collapsed={!s.disclosure().open}
                    onToggle={s.onExpand}
                />
            </Show>
            <Show when={props.node() && props.node().type === "user_message"}>
                {/* AgentMux's own setup prompts are a quiet line, never the human's turn. */}
                <Show when={(props.node() as UserMessageNode).isStartup} fallback={<UserMessageBlock node={props.node() as UserMessageNode} />}>
                    <SetupNote
                        node={props.node() as UserMessageNode}
                        pinned={props.documentState().pinnedNodes.has(props.node().id)}
                        onTogglePin={() => props.onTogglePin(props.node().id)}
                    />
                </Show>
            </Show>
            <Show when={props.node() && props.node().type === "shell"}>
                <PersistentShellBlock
                    node={props.node() as ShellNode}
                    pinned={props.documentState().pinnedNodes.has(props.node().id)}
                    onTogglePin={() => props.onTogglePin(props.node().id)}
                />
            </Show>
        </>
    );
}

function renderInlineRows(s: NodeBodyState): JSX.Element {
    const { props } = s;
    return (
        <>
            <Show when={props.node() && props.node().type === "agent_error"}>
                <AgentErrorRow node={props.node} peek={s.peek} onAgentErrorLogin={props.onAgentErrorLogin} />
            </Show>
            <Show when={props.node() && props.node().type === "context_compacted"}>
                <ContextCompactedRow node={props.node} peek={s.peek} />
            </Show>
            <Show when={props.node() && props.node().type === "compaction_started"}>
                <CompactionStartedRow node={props.node} peek={s.peek} />
            </Show>
            <Show when={props.node() && props.node().type === "memory_reinjection"}>
                <MemoryReinjectionRow node={props.node} peek={s.peek} />
            </Show>
            <Show when={props.node() && props.node().type === "day_divider"}>
                <DayDividerRow node={props.node} peek={s.peek} />
            </Show>
            {/* history_link intentionally has no peek — a render-time synthetic
                CTA row (id "history-link" at the top of the feed, or
                "history-gap:<next id>" between kept turns — spec §6.9) with no
                timestamp/content field
                at all, and its full text is already fully visible without
                hovering. SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25
                treats this as the one deliberate exception to "always fires":
                there is no data here a peek could add. */}
            <Show when={props.node() && props.node().type === "history_link"}>
                <HistoryLinkRow node={props.node} onOpenHistory={props.onOpenHistory} />
            </Show>
            {/* resume_preflight — like history_link, a render-time synthetic
                with a fixed id, no timestamp and no hidden content, so it has
                no peek either (same deliberate exception). */}
            <Show when={props.node() && props.node().type === "resume_preflight"}>
                <ResumePreflightRow node={props.node} onOpenHistory={props.onOpenHistory} />
            </Show>
            <Show when={props.node() && props.node().type === "session_outcome"}>
                <SessionOutcomeRow node={props.node} peek={s.peek} />
            </Show>
            <Show when={props.node() && props.node().type === "cli_notice"}>
                <CliNoticeRow node={props.node} />
            </Show>
        </>
    );
}

/**
 * Routes a DocumentNode to the right block component. Each branch
 * accesses props.node() reactively so streaming updates and state-set
 * toggles propagate without remount.
 *
 * Reactivity discipline (carried over from the original
 * DocumentNodeRenderer comment): never destructure props in this
 * function. The pinned state (`documentState().pinnedNodes`) lives in
 * documentState (separate from the document array that the parent <For>
 * keys off), so a destructured pinned value would stay stale forever even
 * though ToolBlock uses props.pinned correctly. See PR #346.
 */
function DocumentNodeBody(props: DocumentNodeBodyProps): JSX.Element {
    const s = createNodeBodyState(props);

    return (
        <>
            {renderMessageRows(s)}
            {renderInlineRows(s)}
        </>
    );
}
