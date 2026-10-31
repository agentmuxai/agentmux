// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent pane's right-click menu (`AgentViewModel.getBodyContextMenuItems`):
 * Agent History, Quick Fork and View for the pane's conversation, then Seat…,
 * Bundles… and Personal memory for the agent itself, and Shut down, the one act that ends it
 * (closing the pane keeps it running). The pane header has no
 * button of its own (SPEC_DROP_THE_AGENT_STASH_2026_10_24.md, D6).
 */

import type { ModalLayerRequest } from "@/app/element/modal-layer";
import { MENU_ICON } from "@/app/util/menu-icons";
import { openMemory } from "@/app/view/section-pane/panes";
import { HISTORY_TAB_FOR_META_KEY, openOrFocusHistoryTab } from "./open-history-tab";

export interface AgentBodyMenuHost {
    blockId: string;
    meta: Record<string, unknown> | undefined;
    /** The pane's agent has a saved definition; a quick-launch pane (its id is a provider key) has none. */
    hasDefinition: boolean;
    /** Opens a modal in the pane's own layer; null while the pane's view isn't mounted. */
    openModal: ((req: ModalLayerRequest) => void) | null;
    quickFork: () => void;
    /** Opens the View popover (what the pane shows). */
    openView: () => void;
    /** Shuts the pane's agent down (asks first when something would be lost). */
    shutDown: () => void;
}

export function agentBodyMenuItems(host: AgentBodyMenuHost): ContextMenuItem[] {
    const agentId = host.meta?.["agentId"] as string | undefined;
    // No entry on the history tab itself (nothing to open, and no live agent to
    // edit) and none before an agent is loaded.
    if (!agentId || host.meta?.[HISTORY_TAB_FOR_META_KEY]) return [];
    const items: ContextMenuItem[] = [
        {
            label: "Agent History",
            icon: MENU_ICON.history,
            click: () => void openOrFocusHistoryTab({ currentBlockId: host.blockId, agentId }),
        },
        { label: "Quick Fork", icon: MENU_ICON.fork, click: host.quickFork },
        { label: "View…", icon: MENU_ICON.show, click: host.openView },
    ];
    // Seat, bundles and memory all hang off the saved definition.
    if (host.hasDefinition) {
        const agentName = (host.meta?.["agentName"] as string | undefined) || agentId;
        items.push(
            { type: "separator" },
            {
                label: "Seat…",
                icon: MENU_ICON.seat,
                click: () => host.openModal?.({ kind: "agent-seat", agentId, agentName }),
            },
            ...(host.meta?.["agentMode"] === "container"
                ? [
                      {
                          label: "Container limits…",
                          icon: MENU_ICON.seat,
                          click: () => host.openModal?.({ kind: "agent-container-limits", agentId, agentName }),
                      },
                  ]
                : []),
            {
                label: "Bundles…",
                icon: MENU_ICON.bundles,
                click: () => host.openModal?.({ kind: "agent-bundles", agentId, agentName }),
            },
            { label: "Personal memory", icon: MENU_ICON.memory, click: () => void openMemory("personal", { agentId }) }
        );
    }
    items.push({ type: "separator" }, { label: "Shut down", icon: MENU_ICON.shutdown, click: host.shutDown }, { type: "separator" });
    return items;
}
