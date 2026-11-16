// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "Show message": the one way every "take me to that message" control gets there (the sensitive-message card, the
 * panel's "Needs you" strip and its rows). Where the message is decides what showing it means, and each answer
 * is something the person can see:
 *   - the conversation has no row for it (the panel holds it, or no row of the pane's view does): the panel opens on
 *     it, scrolled to it and open;
 *   - the conversation has its row: the panel, which lies over the conversation, closes, and the row is scrolled
 *     into view and marked, so a row already on screen (nothing to scroll) still answers the click.
 */

import type { DocumentNode } from "@/app/agent-model/node-types";
import type { MessagesPanelControls } from "./messages-state";

export interface ShowMessageDeps {
    /** The pane's panel; absent outside the pane's layout. */
    panel: MessagesPanelControls | undefined;
    /** The whole document, with the messages the panel holds. */
    nodes: () => readonly DocumentNode[];
    /** Whether the conversation draws something for the message: its row, or the hidden run that holds it. */
    hasRow: (nodeId: string) => boolean;
    /** Scroll the conversation to the message (a hidden run that holds it opens). */
    jump: (nodeId: string) => void;
    /** Mark the message's row for a moment. */
    mark: (nodeId: string) => void;
}

/** Where the message was shown: in the panel, or as a row of the conversation. */
export type ShownIn = "panel" | "row";

export function showMessage(nodeId: string, d: ShowMessageDeps): ShownIn {
    const node = d.nodes().find((n) => n.id === nodeId);
    const panelOnly = !!d.panel && !!node && d.panel.hidden(node);
    if (d.panel && (panelOnly || !d.hasRow(nodeId))) {
        d.panel.showInPanel(nodeId);
        return "panel";
    }
    // The panel lies over the conversation: a jump behind it would show nothing.
    if (d.panel?.state().open) d.panel.patch({ open: false });
    d.jump(nodeId);
    d.mark(nodeId);
    return "row";
}
