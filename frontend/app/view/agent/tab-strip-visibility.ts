// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether an agent pane should render its tab strip at all.
 *
 * Until now the strip always rendered: with a single conversation open it
 * collapsed to just the 28×28px "+" box, floating over the content
 * (SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md,
 * SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md). That "+"
 * is dead weight on a **fresh** pane, though — one that hasn't launched an
 * agent yet and is still showing the picker ("My Agents"). There is nothing
 * to switch between, and adding a second blank tab before choosing a first
 * agent isn't a real flow. So a fresh pane now renders no strip at all, and
 * the "+" reappears the moment the pane actually has an agent.
 *
 * Kept as a pure predicate (rather than inline JSX) so it can be tested
 * without mounting `agent-view.tsx` — the same extract-and-test pattern the
 * rest of this directory already uses.
 */
export interface TabStripVisibilityInput {
    /** `visibleTabs().length` — already 0 for a lone tab, since a single
     *  conversation shows no pill for itself. */
    visibleTabCount: number;
    /** This pane has launched an agent (block meta `agentId` is set). */
    hasAgent: boolean;
    /** A read-only history-reader tab — never a "fresh" pane, even though
     *  it may not carry a live `agentId` of its own. */
    isHistoryTab: boolean;
}

export function shouldShowTabStrip(input: TabStripVisibilityInput): boolean {
    // Real pills to switch between — always show, regardless of whether
    // the *active* tab happens to be a blank picker (adding a 2nd tab to a
    // launched pane leaves exactly that state, and hiding the strip there
    // would strand the user with no way back to the first tab).
    if (input.visibleTabCount > 0) return true;
    // Lone tab: the strip is just the "+". Worth showing only once this
    // pane is actually a conversation.
    return input.hasAgent || input.isHistoryTab;
}
