// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `revealOrOpen`: the one entry point for "show the pane of this view, or open one", with a typed
 * outcome (docs/reports/REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md §4).
 *
 * The result of a person's action must appear in the window they acted in, so a pane of `view` is
 * looked for in this window only (`scope: "window"`, the default): the shown tab's layout first, then
 * this window's other tabs, and when none is there one is opened here. The outcome says which of those
 * happened, so a caller (and the next steps: a cross-window reveal with a verified raise, an agent's
 * quiet notice) never has to guess where the pane went.
 */

import { isFailure, type SelectionResult } from "@/app/selection/selection-machine";

/** Where a pane of the view is looked for: this window only, or (`app`) any window of the app too. */
export type RevealScope = "window" | "app";

export interface RevealRequest {
    view: string;
    /** What a new pane is made from; defaults to `{ meta: { view } }`. */
    blockDef?: BlockDef;
    /** Merged into the pane's meta before it is shown (a section to show, say). */
    focusMeta?: MetaType;
    /** Who asked, for the selection trace: "widget", "palette", "settings". */
    source: string;
    scope?: RevealScope;
    /** Open a new pane even when one of the view is already open. */
    forceNew?: boolean;
}

/** What `revealOrOpen` did. `blockId` is the pane the person now has. */
export type RevealOutcome =
    | { kind: "shown_here"; blockId: string }
    | { kind: "opened_here"; blockId: string }
    | { kind: "went_to"; blockId: string; windowId: string }
    | { kind: "moved_here"; blockId: string }
    | { kind: "opened_in_other"; blockId: string; windowId: string }
    | { kind: "unverified"; blockId: string; windowId?: string }
    | { kind: "failed"; reason: string; blockId?: string };

/** The pane an outcome leaves the person with, if any. */
export function outcomeBlockId(outcome: RevealOutcome): string | undefined {
    return outcome.blockId;
}

/** What the host said about raising the window that holds a pane (`getApi().focusWindow`). */
export type RaiseVerdict = "verified" | "refused" | "unknown";

/**
 * `verified`: the window was raised and is the foreground one when read back. `refused`: the host
 * answered and it is not (the OS kept it behind, or there is no such window). `unknown`: no answer,
 * or one that timed out, so nothing is known.
 */
export function raiseVerdict(raise: FocusWindowResult | undefined): RaiseVerdict {
    if (!raise || raise.timed_out) return "unknown";
    return raise.found && raise.raised && raise.foreground ? "verified" : "refused";
}

/** A pane of the same view and identity in another window of the app. */
export interface PaneElsewhere {
    blockId: string;
    windowId: string;
    tabId: string;
    workspaceId: string;
    /** For the notice: "Settings is open in another window". */
    name: string;
    /** A terminal or an agent: it can be gone to but never moved, which would cut its session off. */
    liveProcess: boolean;
}

/** What going to a pane in another window did: its window's answer, and what the host said of the raise. */
export interface GoResult {
    shown: "revealed" | "failed" | "timeout";
    reason?: string;
    raise?: FocusWindowResult;
}

/** What an `app` scope reveal reads and does beyond this window. */
export interface AppRevealDeps {
    findInOtherWindow(def: BlockDef): Promise<PaneElsewhere | null>;
    /** Show the pane in the window that holds it, and raise that window. */
    goTo(found: PaneElsewhere): Promise<GoResult>;
    /** Tell the person where the pane is, since going there was not confirmed. */
    offer(found: PaneElsewhere, why: "refused" | "unknown"): void;
}

/** What `revealOrOpen` reads and does, so the decision can be tested without a window. */
export interface RevealDeps {
    recordOpened(def: BlockDef): void;
    /** A pane of `view` mounted in the shown tab's layout (with the way to show and focus it), or the id of one mounted in another tab. */
    findMounted(view: string): { here?: { blockId: string; showAndFocus(): void }; elsewhere?: string };
    /** A pane of `view` in any tab of this window, mounted or not. */
    findInWindow(view: string): Promise<string | null>;
    setMeta(blockId: string, meta: MetaType): Promise<void>;
    /** Go to the pane in this window: switch tab, show, focus, read back. */
    revealInWindow(blockId: string): Promise<SelectionResult>;
    create(def: BlockDef): Promise<string>;
    /** Present where `scope: "app"` is honoured. */
    app?: AppRevealDeps;
}

/** Why a reveal that did not show the pane means "there is no such pane": it is not there or just closed. */
const absent = (r: SelectionResult): boolean => isFailure(r) && (r.reason === "not-found" || r.reason === "gone");

export async function revealOrOpen(req: RevealRequest, deps: RevealDeps): Promise<RevealOutcome> {
    const def: BlockDef = req.blockDef ?? { meta: { view: req.view } };
    // Showing the pane that is already open is a use of it too (the Shortcuts pane's last-used order).
    deps.recordOpened(def);
    if (!req.forceNew) {
        const { here, elsewhere: mountedElsewhere } = deps.findMounted(req.view);
        if (here) {
            if (req.focusMeta != null) await deps.setMeta(here.blockId, req.focusMeta);
            here.showAndFocus();
            return { kind: "shown_here", blockId: here.blockId };
        }
        // An inactive tab of a pane's stack is not mounted: look in the window's tabs too.
        const elsewhere = mountedElsewhere ?? (await deps.findInWindow(req.view)) ?? undefined;
        if (elsewhere) {
            if (req.focusMeta != null) await deps.setMeta(elsewhere, req.focusMeta);
            const shown = await deps.revealInWindow(elsewhere);
            if (!isFailure(shown)) return { kind: "shown_here", blockId: elsewhere };
            // Only a pane that is not there (never found, or closed meanwhile) is opened afresh: a newer click
            // taking over, or a tab that was slow, leaves the pane where it is and a second one would be a copy.
            if (!absent(shown)) return { kind: "failed", reason: shown.reason, blockId: elsewhere };
        }
    }
    if (!req.forceNew && req.scope === "app" && deps.app) {
        const found = await deps.app.findInOtherWindow(def);
        const went = found && (await goToElsewhere(req, found, deps, deps.app));
        if (went) return went;
    }
    return { kind: "opened_here", blockId: await deps.create(def) };
}

/**
 * Go to a pane another window holds. Confirmed (it showed the pane and its window is the foreground one) it
 * is `went_to`; otherwise the person is told where it is, with a way to go there again or, where the pane can
 * move, to show it here. `null`: the pane is not there any more, so a new one is opened.
 */
async function goToElsewhere(req: RevealRequest, found: PaneElsewhere, deps: RevealDeps, app: AppRevealDeps): Promise<RevealOutcome | null> {
    if (req.focusMeta != null) await deps.setMeta(found.blockId, req.focusMeta);
    const go = await app.goTo(found);
    const { blockId, windowId } = found;
    if (go.shown === "failed") {
        if (go.reason === "not-found" || go.reason === "gone") return null;
        app.offer(found, "unknown");
        return { kind: "failed", reason: go.reason ?? "the window could not show it", blockId };
    }
    const verdict = raiseVerdict(go.raise);
    if (go.shown === "revealed" && verdict === "verified") return { kind: "went_to", blockId, windowId };
    app.offer(found, verdict === "refused" ? "refused" : "unknown");
    return { kind: "unverified", blockId, windowId };
}
