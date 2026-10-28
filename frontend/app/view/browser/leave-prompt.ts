// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Closing a browser pane whose page would ask "Leave site?" (a `beforeunload`
 * handler on a page the person has used) asks the person first, in AgentMux's
 * own dialog naming the site, instead of Chrome's native box over a pane that
 * is already gone. Only the person's own close asks: an agent's Stop already
 * gave the person its countdown, and AgentMux quitting asks for the whole app.
 * The host answers the page's own prompt Leave once the close is decided
 * (crates/cef/src/browser_pane/leaving.rs).
 */

export interface LeavePrompt {
    blockId: string;
    /** The page's host name, e.g. "portal.azure.com". */
    site: string;
}

/** How to look a block up. Injected so the logic is testable without the host. */
export interface LeaveProbe {
    isBrowser(blockId: string): boolean;
    /** The host's check: this page would show "Leave site?" now. */
    asks(blockId: string): Promise<boolean>;
    site(blockId: string): string;
}

/**
 * The pages that would prompt, among `blockIds`. A check that fails counts as
 * "would not": a broken lookup must never make a pane impossible to close.
 */
export async function leavePrompts(blockIds: string[], probe: LeaveProbe): Promise<LeavePrompt[]> {
    const asked = await Promise.all(
        blockIds
            .filter((blockId) => probe.isBrowser(blockId))
            .map(async (blockId) => ((await probe.asks(blockId).catch(() => false)) ? { blockId, site: probe.site(blockId) } : null))
    );
    return asked.filter((p): p is LeavePrompt => p !== null);
}

/** The host name of an address, or "this page" when it has none. */
export function siteOf(url: string | undefined): string {
    try {
        return new URL(url ?? "").hostname || "this page";
    } catch {
        return "this page";
    }
}

/** "portal.azure.com may have unsaved changes." */
export function describeLeave(prompts: LeavePrompt[]): string {
    const sites = [...new Set(prompts.map((p) => p.site))];
    const named = sites.length <= 1 ? (sites[0] ?? "This page") : `${sites.slice(0, -1).join(", ")} and ${sites[sites.length - 1]}`;
    return `${named} may have unsaved changes.`;
}

// The blocks the person chose to close, until the window tells the host: the
// host's log says who closed a pane.
const personCloses = new Set<string>();

export function notePersonClose(blockIds: string[]): void {
    for (const blockId of blockIds) personCloses.add(blockId);
}

/** Whether the person chose to close `blockId`; asked once, as the pane closes. */
export function takePersonClose(blockId: string): boolean {
    return personCloses.delete(blockId);
}
