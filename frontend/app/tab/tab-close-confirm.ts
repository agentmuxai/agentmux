// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Whether closing a window tab asks first. It's opt-in: only when the user
// has turned on Settings → Window & Panes → "Confirm before closing a tab"
// (SPEC_TAB_CLOSE_CONFIRM_OFF_BY_DEFAULT_2026_10_10.md). The old
// `tab:skipcloseconfirm` is no longer read.

/** The setting: ask before closing a tab when `true`; unset is off. */
export const TAB_CONFIRM_CLOSE = "tab:confirmclose";

/** Whether a tab close opens the confirmation first. */
export function asksBeforeClosingTab(settings: Record<string, unknown> | null | undefined): boolean {
    return settings?.[TAB_CONFIRM_CLOSE] === true;
}

/** What the modal's "Don't ask again" writes. */
export const DONT_ASK_AGAIN = { [TAB_CONFIRM_CLOSE]: false } as const;
