// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the store layer asks of the view layer without importing it: the commands that act on a
 * view's own state. `app-init.ts` registers each one once, loading the view module when the
 * action first runs, so the store keeps no import of a view (scripts/RATCHETS.md). An action
 * not registered does nothing and says so on the console.
 */

export interface ViewActions {
    /** Puts the browser panes that aren't visible to sleep. */
    hibernateHiddenBrowserPanes(): Promise<unknown>;
    /** Wakes every sleeping browser pane. */
    wakeAllBrowserPanes(): Promise<unknown>;
    /** Redocks this floating pane into the main window (a no-op outside a floater). */
    redockThisFloater(): Promise<unknown>;
}

const actions: Partial<ViewActions> = {};

export function registerViewActions(impl: ViewActions): void {
    Object.assign(actions, impl);
}

export function runViewAction(name: keyof ViewActions): void {
    const run = actions[name];
    if (!run) {
        console.warn(`view action "${name}" is not registered`);
        return;
    }
    void run().catch(console.error);
}

let basicTermCount: () => number = () => 0;

/** The terminal view says how many of its panes run a shell; the keymodel reads it for multi-input. */
export function setBasicTermCount(count: () => number): void {
    basicTermCount = count;
}

export function countBasicTerms(): number {
    return basicTermCount();
}
