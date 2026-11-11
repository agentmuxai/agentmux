// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { fitToWidth } from "@/app/element/width-fit";

/** A slot's fit tier: 0 is the first, before anything else of the bar is shed (an item that comes and goes). */
export type DropAt = 0 | 1 | 2 | 3 | 4;

/** The agent entry's id in the status bar registry. */
export const AGENT_COUNTS_ID = "agentmux:agent-counts";

/**
 * The fit tier at which a status bar slot goes in a narrow bar. The agent entry is where a request that waits on the
 * person is kept once its toast has gone, so while anything waits it is never dropped (every other item that can go
 * has gone before the last tier); the others keep their own.
 */
export function slotDropAt(id: string, base: DropAt | undefined, waiting: boolean): DropAt | undefined {
    return id === AGENT_COUNTS_ID && waiting ? undefined : base;
}

/** What a narrow bar sheds, tier by tier, until it fits on one line. Tier 0 hides only the slots that drop at 0. */
export const FIT_TIERS = ["0", "1", "2", "3", "4"] as const;

/**
 * Fits the bar again once its items have been laid out. The fit runs when the bar's own parts change size, and an
 * item that shows or goes away while its slot is hidden changes none: a transient item that pushed the bar to a higher
 * tier would otherwise leave it there until the window is resized.
 */
export function refitStatusBar(): void {
    const bar = document.querySelector<HTMLElement>(".status-bar");
    if (bar) requestAnimationFrame(() => fitToWidth(bar, FIT_TIERS));
}
