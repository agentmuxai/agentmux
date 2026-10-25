// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The page's address with the host's IPC token replaced, for anything that is
 * written to a log. The host hands the token to a window in its address; the
 * frontend removes it once read (cef-init.ts), but whatever logs the address
 * before that, or after a reload, must not keep it.
 */
export function redactIpcToken(href: string): string {
    return href.replace(/([?&#]ipc_token=)[^&#]*/g, "$1<redacted>");
}
