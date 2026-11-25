// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane type's own actions (the agent's Find and Messages, the terminal's Start and Restart, a widget's header
 * actions) as items of the pane's context menu: the header's and each tab's. A pane tab shows its icon, its name and
 * its close button and nothing else: every button a view model declares through `endIconButtons` is a row here, with its
 * icon, its words and whether it is enabled, and runs what the button ran (a toggle flips).
 * They come first, above the generic pane items, with a separator under them.
 */

import { messagesHeaderButtons } from "@/app/view/agent/messages/messages-state";

/** Whether the pane in block `blockId` has an action with something for the person (Messages with unread): its tab shows a dot. Reactive. */
export function tabWantsYou(blockId: string): boolean {
    return messagesHeaderButtons(blockId).some((button) => button.attention?.() ?? false);
}

/** The rows for `viewModel`'s declared buttons, or none. `e` is the event that opened the menu, handed to a button's click. */
export function paneActionMenuItems(viewModel: ViewModel | null | undefined, e: MouseEvent): ContextMenuItem[] {
    const buttons = viewModel?.endIconButtons?.() ?? [];
    const items: ContextMenuItem[] = [];
    for (const button of buttons) {
        const label = button.title?.trim();
        // A button with no words has nothing to name its row.
        if (!label || button.noAction) continue;
        // A button whose face is a node (the Messages envelope with its count) names its menu icon.
        const icon = button.menuIcon ?? (typeof button.icon === "string" ? button.icon : undefined);
        if (button.elemtype === "toggleiconbutton") {
            // A plain row, not a check row: a check takes the icon's place, and the pane shows the state (the find bar or
            // the panel is open or it is not).
            items.push({ label, icon, enabled: !button.disabled, click: () => button.active._set(!button.active()) });
        } else {
            items.push({ label, icon, enabled: !button.disabled, click: () => button.click?.(e) });
        }
    }
    return items.length > 0 ? [...items, { type: "separator" }] : [];
}
