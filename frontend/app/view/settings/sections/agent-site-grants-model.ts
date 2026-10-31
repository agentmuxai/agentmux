// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pure parts of Settings, Agents on sites: how a row says what it
// covers, and which powers a row's checkboxes offer.

import { LATE_PROMPT_LIMIT, siteLabel } from "@/app/view/browser/bypass";

export { LATE_PROMPT_LIMIT };

/** What a power is called beside its checkbox. */
export const POWER_LABELS: Record<string, string> = {
    terminal: "Terminal: type and run commands",
};

/** A row's sites for display: the one it is named by, and how many frames of other sites belong to it. */
export function sitesLabel(origins: readonly string[]): string {
    const [first, ...rest] = origins;
    if (!first) return "";
    return rest.length ? `${siteLabel(first)} (and ${rest.map(siteLabel).join(", ")})` : siteLabel(first);
}

/** One checkbox per power a row can carry beyond committing clicks, on when the row has it. */
export function powerChecklist(offered: readonly string[], has: readonly string[]): { power: string; on: boolean }[] {
    return offered.map((power) => ({ power, on: has.includes(power) }));
}
