// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pure parts of Settings, Agents on sites: how a row says what it
// covers, and which powers a row's checkboxes offer.

import { LATE_PROMPT_LIMIT, siteLabel } from "@/app/view/browser/bypass";
import type { BrowserGrantsResult } from "@/types/rpc/BrowserGrantsResult";

export { LATE_PROMPT_LIMIT };

/**
 * srv's list with every field the screen reads present. An srv older than this build (an upgrade in
 * progress, a packaged backend under a newer frontend) leaves out what it did not have yet, and a
 * missing list must show as an empty one, not take the whole Settings page down. `pause_all` is
 * passed through as it came: this screen does not read it (the pause switch has its own store).
 */
export function completeGrants(raw: Partial<BrowserGrantsResult> | null | undefined): BrowserGrantsResult {
    const g = raw ?? {};
    return {
        ...g,
        rows: (g.rows ?? []).map((r) => ({ ...r, origins: r.origins ?? [], powers: r.powers ?? [] })),
        presets: (g.presets ?? []).map((p) => ({ ...p, sites: p.sites ?? [] })),
        powers: g.powers ?? [],
        permission_kinds: g.permission_kinds ?? [],
        permissions: (g.permissions ?? []).map((p) => ({ ...p, kinds: p.kinds ?? [] })),
    } as BrowserGrantsResult;
}

/** What a power is called beside its checkbox. */
export const POWER_LABELS: Record<string, string> = {
    terminal: "Terminal: type and run commands",
    terminal_ask: "Terminal: ask me for each line",
    enter_space: "Enter and Space: press them as the click they are",
    uploads: "Uploads: fill a page's file chooser from its workspace",
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
