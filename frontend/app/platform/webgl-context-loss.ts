// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a lost WebGL context belongs to a terminal.
 *
 * Every terminal's WebGL renderer holds its own context, and Chromium allows
 * a page 16: opening the 17th evicts the oldest. termwrap.ts moves that
 * terminal to the DOM renderer (`onContextLoss`). bootstrap.ts reloads the
 * page on a lost context, meant for a GPU or driver reset, and must not do it
 * for these: the reload recreates every terminal, the limit evicts the same
 * number again, and the page reloaded three times before giving up
 * (docs/analysis/RCA_TERMINAL_WEBGL_CONTEXT_CAP_RELOAD_LOOP_2026_10_13.md).
 */
export function isTerminalContext(target: EventTarget | null): boolean {
    const el = target as { closest?: (selector: string) => unknown } | null;
    return typeof el?.closest === "function" && el.closest(".xterm") != null;
}
