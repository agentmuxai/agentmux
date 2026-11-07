// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Show me this command in Keyboard shortcuts": Help's Customize and Set links ask, and the section (once it is
// on screen, which may be after Settings has to open) scrolls to the line and marks it. A signal, because the
// section may not exist yet when the link is clicked.

import { createSignal } from "solid-js";

import { openOrFocusPaneIdByView } from "./block-component-registry";

/** `blockId`: the Settings pane that was opened or focused for it; only that pane's section takes the request
 *  (several Settings panes can be open). Absent, whichever section sees it first. */
interface KeyboardRowRequest {
    command: string | null;
    blockId?: string;
}

const [request, setRequest] = createSignal<KeyboardRowRequest | null>(null);

export const keyboardRowRequest = request;

/** Ask the Keyboard shortcuts section to show `command`, or just open when none is given. */
export const requestKeyboardRow = (command: string | null, blockId?: string): void => void setRequest(() => ({ command, blockId }));

export const clearKeyboardRowRequest = (): void => void setRequest(() => null);

/** Open Settings at Keyboard shortcuts, scrolled to `command`'s line when one is given. */
export async function openKeyboardSettings(command?: string): Promise<void> {
    const blockId = await openOrFocusPaneIdByView("settings", { meta: { view: "settings", "settings:section": "keyboard" } }, { "settings:section": "keyboard" });
    requestKeyboardRow(command ?? null, blockId);
}
