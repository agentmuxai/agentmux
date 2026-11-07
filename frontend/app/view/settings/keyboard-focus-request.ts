// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// "Show me this command in Keyboard shortcuts": Help's Customize and Set links ask, and the section (once it is
// on screen, which may be after Settings has to open) scrolls to the line and marks it. A signal, because the
// section may not exist yet when the link is clicked.

import { createSignal } from "solid-js";

const [request, setRequest] = createSignal<{ command: string | null } | null>(null);

export const keyboardRowRequest = request;

/** Ask the Keyboard shortcuts section to show `command`, or just open when none is given. */
export const requestKeyboardRow = (command: string | null): void => void setRequest(() => ({ command }));

export const clearKeyboardRowRequest = (): void => void setRequest(() => null);
