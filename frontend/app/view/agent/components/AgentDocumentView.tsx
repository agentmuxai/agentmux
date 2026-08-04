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
 * scroll area, then moved to a dedicated activity-log panel docked above
 * the composer, then redirected into the shell terminal (agent-view.tsx's
 * `log`/`handleShellTermReady`) — see `agentmux-ai/AGENT_PANE_ACTIVITY_LOG_SPEC.md`.
 */

import { createSignal, onMount, Show, type Accessor, type JSX } from "solid-js";
import { Button } from "@/element/button";
import type { SignalPair } from "../state";
import type { DocumentNode, DocumentState } from "../types";
import type { ScrollCommand } from "../hooks/useScrollToNode";
import type { LayoutView } from "@/app/store/agent-pane-layout-store";
import { AgentDocumentVirtualList } from "../virtualization/AgentDocumentVirtualList";
import { createAgentViewState } from "../virtualization/state";

interface AgentDocumentViewProps {
    documentAtom: SignalPair<DocumentNode[]>;
    documentStateAtom: SignalPair<DocumentState>;
    authUrl?: Accessor<string | null>;
    /**
     * User-visible auth-recovery error (e.g. "Login Again" couldn't open a
     * browser). Rendered as an error box in the same header slot as the
     * auth-URL box, with a dismiss button. Never fail silently — see
     * retro-agent-auth-relogin-noop-2026-07-01 §5.1.
     */
    authNotice?: Accessor<string | null>;
    /** Dismiss handler for the auth notice (clears the signal). */
    onDismissAuthNotice?: () => void;
    /** Provider ID for the active auth flow — used when submitting a pasted auth code. */
    authProviderId?: string;
    /** Cancel the in-flight login (kills the host CLI child) — shown as a
     *  button on the auth-URL box so a stuck login has an exit besides
     *  closing the pane. Wired to useAgentControllerStatus's cancelLogin. */
    onCancelLogin?: () => void;
    /** Explicit "Use terminal instead" fallback on the auth-URL box — mirrors
     *  PreLaunchAuthPanel's identical secondary action
     *  (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 2). Wired to
     *  useAgentControllerStatus's loginViaTerminal, which cancels this
     *  session's own login child itself (tier 1 starting a second one would
     *  race against the one still open in this box). */
    onUseTerminal?: () => void;
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
                {(url) => (
                    <AuthUrlBox
                        url={url()}
                        authProviderId={props.authProviderId}
                        onCancel={props.onCancelLogin}
                        onUseTerminal={props.onUseTerminal}
                    />
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
            headerSlot={headerSlot()}
        />
    );
};

AgentDocumentView.displayName = "AgentDocumentView";

// ── Auth URL box ────────────────────────────────────────────────────────────

interface AuthUrlBoxProps {
    url: string;
    authProviderId?: string;
    onCancel?: () => void;
    onUseTerminal?: () => void;
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
    const [switchingToTerminal, setSwitchingToTerminal] = createSignal(false);
    let inputRef: HTMLInputElement | undefined;

    // Grab focus when the box appears so the user's pasted code lands HERE,
    // not in the main agent message input (which otherwise holds focus and
    // silently swallows the paste — the login then stalls waiting on stdin).
    // Deferred past the current focus cycle (the pane's focus-reclaim runs
    // synchronously on render) so this wins the race.
    onMount(() => {
        requestAnimationFrame(() => inputRef?.focus());
    });

    const submitCode = async (explicit?: string): Promise<void> => {
        // Read from the live input element as a fallback. If focus desynced and
        // the controlled signal missed the paste, the DOM value still holds it —
        // otherwise a pasted-then-submitted code can be silently dropped.
        const code = (explicit ?? inputRef?.value ?? pasteCode()).trim();
        if (!code) return;
        setPasting(true);
        setPasteResult(null);
        try {
            const { getApi } = await import("@/app/store/global");
            await getApi().setProviderAuth(props.authProviderId ?? "claude", code);
            setPasteResult("Code accepted — signing you in…");
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
            <div class="agent-auth-title">Sign in to Claude</div>

            <div class="agent-auth-url-label">1 · Authorize in your browser</div>
            <div class="agent-auth-hint">
                Your browser should have opened. If it didn't, open this link:
            </div>
            <div class="agent-auth-url-row">
                <span class="agent-auth-url-text">{props.url}</span>
                <button
                    class="agent-auth-url-copy"
                    title="Open this URL in your browser"
                    onClick={() => {
                        void import("../flows/open-oauth-pane").then(m => m.openOAuthBrowserPane(props.url));
                    }}
                >
                    Open
                </button>
                <button
                    class="agent-auth-url-copy"
                    onClick={() => { import("@/util/clipboard").then(c => c.writeText(props.url)); }}
                    title="Copy URL"
                >
                    Copy
                </button>
            </div>

            <div class="agent-auth-url-label agent-auth-step-2">2 · Paste the code from that page</div>
            <div class="agent-auth-hint">
                After you authorize, the page shows an <strong>authorization code</strong> — copy it and paste it here.
            </div>
            <div class="agent-auth-paste-row">
                <input
                    ref={inputRef}
                    class="agent-auth-paste-input"
                    type="text"
                    placeholder="Paste the authorization code…"
                    value={pasteCode()}
                    onInput={(e) => setPasteCode((e.target as HTMLInputElement).value)}
                    onKeyDown={(e) => { if (e.key === "Enter") void submitCode(); }}
                    onPaste={(e) => {
                        // Auto-submit on paste — pasting the code IS the intent to
                        // submit, so the user doesn't have to also click a button.
                        const text = (e.clipboardData?.getData("text") ?? "").trim();
                        if (text) { setPasteCode(text); void submitCode(text); }
                    }}
                />
                <button
                    class="agent-auth-url-copy"
                    title="Paste from clipboard and submit"
                    onClick={() => {
                        import("@/util/clipboard").then(c => c.readText()).then(text => {
                            const trimmed = (text ?? "").trim();
                            if (trimmed) { setPasteCode(trimmed); void submitCode(trimmed); }
                        }).catch(() => {
                            setPasteResult("Could not read clipboard — paste manually");
                        });
                    }}
                >
                    Paste &amp; submit
                </button>
                <button
                    class="agent-auth-url-copy"
                    onClick={() => { void submitCode(); }}
                    disabled={pasting()}
                >
                    {pasting() ? "…" : "Submit"}
                </button>
            </div>
            <Show when={pasteResult()}>
                <div class="agent-auth-paste-result">{pasteResult()}</div>
            </Show>
            <div class="agent-auth-actions-row">
                <Show when={props.onCancel}>
                    <Button onClick={() => props.onCancel?.()}>Cancel login</Button>
                </Show>
                <Show when={props.onUseTerminal}>
                    {/* Secondary fallback alongside Cancel — the URL/paste flow
                        above is the default now (spec §3.2), but a browser that
                        can't reach it (remote desktop, sandboxed host) still
                        needs a way out besides giving up. Mirrors
                        PreLaunchAuthPanel's identical secondary action. */}
                    <Button
                        className="grey"
                        disabled={switchingToTerminal()}
                        onClick={async () => {
                            setSwitchingToTerminal(true);
                            try {
                                await props.onUseTerminal?.();
                            } finally {
                                setSwitchingToTerminal(false);
                            }
                        }}
                    >
                        {switchingToTerminal() ? "Switching…" : "Use terminal instead"}
                    </Button>
                </Show>
            </div>
        </div>
    );
}
