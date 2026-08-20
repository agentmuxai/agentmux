// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentDocumentView — owns the agent pane's data state interface
 * (collapsed/pinned toggles, auto-collapse policy, loading-older banner),
 * then delegates list rendering to AgentDocumentVirtualList (Phase 2 of
 * the virtualization redesign, see
 * docs/specs/SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md).
 *
 * Also exports AgentAuthPanel (the login UI), a component agent-view.tsx
 * renders separately as a bottom-docked sibling — not part of this
 * component's own scrollable output.
 *
 * All scroll behavior — stick-to-bottom, anchor capture, jump-to-node,
 * pagination restore — lives in the VirtualList. This component is
 * just glue between the existing AgentAtoms contract and that new
 * component.
 *
 * Diagnostic / launch-flow logs used to render inline at the top of this
 * scroll area, then moved to a dedicated activity-log panel docked above
 * the composer, then redirected into the shell terminal (agent-view.tsx's
 * `log`/`handleShellTermReady`) — see `agentmux-ai/AGENT_PANE_ACTIVITY_LOG_SPEC.md`.
 */

import { createMemo, Show, type Accessor, type JSX } from "solid-js";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState } from "../types";
import type { ScrollCommand } from "../hooks/useScrollToNode";
import type { LayoutView } from "@/app/store/agent-pane-layout-store";
import { AgentDocumentVirtualList } from "../virtualization/AgentDocumentVirtualList";
import { createAgentViewState } from "../virtualization/state";
import { correlateDispatchesForBlock } from "../activity/dispatch-correlation";
import { allSubagentsAtom } from "../activity/subagent-source";
import { allDispatchesAtom } from "../activity/dispatch-source";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { InAppLoginPanel } from "./InAppLoginPanel";
import { PROVIDERS } from "../providers/catalog";
import type { LaunchPhase } from "../flows/launch-phase";
import { toInAppLoginPhase } from "./to-in-app-login-phase";

interface AgentDocumentViewProps {
    documentAtom: SignalPair<DocumentNode[]>;
    documentStateAtom: SignalPair<DocumentState>;
    /** Re-run the provider login flow — forwarded to the list so an inline
     *  auth-error node can offer a "Login Again" CTA (SPEC_REAUTH_FROM_AUTH_ERROR §7). */
    onAgentErrorLogin?: () => void;
    /** Called when the user scrolls near the top — load the previous page of history. */
    onLoadOlder?: () => Promise<void>;
    /** Whether an older-history load is currently in progress. */
    loadingOlder?: Accessor<boolean>;
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
    /** AgentWorkingRow's current height — forwarded to the list's
     *  stick-to-bottom effect. See AgentDocumentVirtualListProps.workingRowHeight. */
    workingRowHeight?: Accessor<number>;
    /** Open/focus the Agent History tab — forwarded to the list so a
     *  `history_link` synthetic row can act on click. See
     *  SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.2. */
    onOpenHistory?: () => void;
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

    // Hold a tool expanded after it completes live on screen (added by
    // ToolBlock on the active→inactive transition). Stays open until its row
    // scrolls off the top, at which point the VirtualList calls releaseToolOpen.
    // Replaces the old 3 s post-completion timer — see
    // docs/specs/PLAN_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md.
    const holdToolOpen = (nodeId: string): void => {
        setDocumentState((prev) => {
            if (prev.expandedTools.has(nodeId)) return prev; // already held — no churn
            const expandedTools = new Set(prev.expandedTools);
            expandedTools.add(nodeId);
            return { ...prev, expandedTools };
        });
    };

    // Release a held tool once it has scrolled off the top (latched collapse).
    const releaseToolOpen = (nodeId: string): void => {
        setDocumentState((prev) => {
            if (!prev.expandedTools.has(nodeId)) return prev;
            const expandedTools = new Set(prev.expandedTools);
            expandedTools.delete(nodeId);
            return { ...prev, expandedTools };
        });
    };

    // Header slot: loading-older banner, rendered above the virtualizer
    // inside the scroll container. VirtualList's scrollMargin
    // (=virtualContainerRef.offsetTop) handles the offset automatically.
    //
    // The auth-URL box and auth-notice USED to render here too, but that
    // pinned the login UI to the top of the scroll area instead of near the
    // composer like AgentQuestionPanel/AgentDecisionPanel — see the #2429
    // follow-up. They're now AgentAuthPanel, rendered by agent-view.tsx as a
    // flex sibling after .agent-document-scroll-region.
    const headerSlot = (): JSX.Element => (
        <Show when={props.loadingOlder?.()}>
            <div class="agent-history-loading">Loading older messages...</div>
        </Show>
    );

    // Ordinal-matched tool_use_id -> live dispatch for this pane's
    // Agent/Task/Workflow tool calls (see activity/dispatch-correlation.ts).
    // Empty (not attempted) without a blockId — a pane with no block id has
    // no dispatches of its own to match against anyway.
    const dispatchMatches = createMemo<Map<string, AgentDispatch>>(() =>
        props.blockId
            ? correlateDispatchesForBlock(props.blockId, props.documentAtom[0](), allSubagentsAtom(), allDispatchesAtom())
            : new Map()
    );

    return (
        <AgentDocumentVirtualList
            viewState={viewState}
            documentState={documentState}
            onAgentErrorLogin={props.onAgentErrorLogin}
            onLoadOlder={props.onLoadOlder}
            loadingOlder={props.loadingOlder}
            highlightNodeId={props.highlightNodeId}
            scrollCommand={props.scrollCommand}
            scrollToBottomRef={props.scrollToBottomRef}
            onToggleCollapse={toggleCollapse}
            onTogglePin={togglePin}
            onHoldToolOpen={holdToolOpen}
            onReleaseToolOpen={releaseToolOpen}
            zoomFactor={props.zoomFactor}
            blockId={props.blockId}
            layoutView={props.layoutView}
            workingRowHeight={props.workingRowHeight}
            onOpenHistory={props.onOpenHistory}
            headerSlot={headerSlot()}
            dispatchMatches={dispatchMatches}
        />
    );
};

AgentDocumentView.displayName = "AgentDocumentView";

// ── Auth panel (bottom-docked, sibling of AgentQuestionPanel) ──────────────

export interface AgentAuthPanelProps {
    authUrl?: Accessor<string | null>;
    /**
     * User-visible auth-recovery error (e.g. "Login Again" couldn't open a
     * browser). Rendered as an error box below the login panel, with a
     * dismiss button. Never fail silently — see
     * retro-agent-auth-relogin-noop-2026-07-01 §5.1.
     */
    authNotice?: Accessor<string | null>;
    /** Dismiss handler for the auth notice (clears the signal). */
    onDismissAuthNotice?: () => void;
    /** Provider ID for the active auth flow — used when submitting a pasted auth code. */
    authProviderId?: string;
    /** Drives the in-app login panel's phase line — see `toInAppLoginPhase`. */
    launchPhase?: Accessor<LaunchPhase | null>;
    /** Cancel the in-flight login (kills the host CLI child) — shown as a
     *  button on the login panel so a stuck login has an exit besides
     *  closing the pane. Wired to useAgentControllerStatus's cancelLogin. */
    onCancelLogin?: () => void;
    /** Explicit "Use terminal instead" fallback on the login panel — mirrors
     *  PreLaunchAuthPanel's identical secondary action
     *  (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 2). Wired to
     *  useAgentControllerStatus's loginViaTerminal, which cancels this
     *  session's own login child itself (tier 1 starting a second one would
     *  race against the one still open in this box). */
    onUseTerminal?: () => void;
}

/**
 * Bottom-docked login UI. Rendered by agent-view.tsx as a flex sibling
 * after .agent-document-scroll-region, in the same slot band as
 * AgentDecisionPanel/AgentQuestionPanel — NOT inside AgentDocumentView's
 * scrollable header slot (that pinned it to the top of the scroll area,
 * #2429 follow-up) and NOT a Modal (a floating dialog would dim/block the
 * transcript, which the bottom-docked AgentQuestionPanel/AgentDecisionPanel
 * pattern deliberately avoids — chosen over the modal for consistency).
 *
 * Renders the same InAppLoginPanel the Armory/Stash surfaces use (Fix 3a,
 * PR #2423) instead of a hand-rolled copy — one fewer auth UI in the
 * codebase — just without the Modal wrapper those surfaces use.
 */
export function AgentAuthPanel(props: AgentAuthPanelProps): JSX.Element {
    return (
        <>
            <Show when={props.authUrl?.()}>
                {(url) => (
                    // AgentQuestionPanel-style card chrome (border/bg/shadow) —
                    // InAppLoginPanel itself ships with none, since its other
                    // two callers (PreLaunchAuthPanel, the Armory/Stash Modal)
                    // already provide their own box. This is the one context
                    // that renders it bare in normal document flow, so it
                    // needs its own card here to read as a distinct prompt
                    // instead of loose, unbordered text.
                    <div class="agent-auth-panel-card">
                        <InAppLoginPanel
                            providerId={props.authProviderId ?? ""}
                            providerLabel={PROVIDERS[props.authProviderId ?? ""]?.displayName ?? props.authProviderId ?? "provider"}
                            authUrl={url()}
                            phase={toInAppLoginPhase(url(), props.launchPhase?.() ?? null)}
                            onCancel={() => props.onCancelLogin?.()}
                            onUseTerminal={() => props.onUseTerminal?.()}
                        />
                    </div>
                )}
            </Show>
            <Show when={props.authNotice?.()}>
                {(notice) => (
                    <div class="agent-auth-notice" role="alert">
                        <span class="agent-auth-notice-text">{notice()}</span>
                        <button
                            class="agent-auth-notice-dismiss"
                            title="Dismiss"
                            onClick={() => props.onDismissAuthNotice?.()}
                        >
                            ✕
                        </button>
                    </div>
                )}
            </Show>
        </>
    );
}
