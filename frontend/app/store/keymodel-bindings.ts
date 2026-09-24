// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Key descriptions for global bindings that are also shown as a shortcut label
// somewhere in the UI. keymodel.ts registers these exact strings, and the UI
// renders them with keyutil's formatKeyDescription. A binding and its label
// therefore can't drift apart.
//
// Syntax is keyutil's parseKeyDescription: "Cmd" is ⌘ on macOS and Alt on
// Windows/Linux; "Ctrl" is the Control key on every platform.
//
// A leaf module with no imports on purpose: keymodel.ts pulls in the global
// store, the layout model and the command palette, and a component that only
// needs a label shouldn't import all of that.

/** New tab. Labelled in the hamburger menu. */
export const NEW_TAB_KEY = "Cmd:t";

/** New window. Labelled in the hamburger menu. */
export const NEW_WINDOW_KEY = "Ctrl:Shift:n";

/** Open the command palette. Labelled in the hamburger menu. */
export const COMMAND_PALETTE_KEY = "Ctrl:p";
