// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared async helpers — consolidates 12 inlined copies of the same
 * `new Promise<void>((r) => setTimeout(r, ms))` sleep expression found
 * scattered across poll loops, retry loops, drag monitors, and tests.
 *
 * See docs/specs/SPEC_TOKEN_STATS_NUMBER_FORMATTING_2026_08_02.md §7.3.
 */

/** Resolve after `ms` milliseconds. */
export const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));
