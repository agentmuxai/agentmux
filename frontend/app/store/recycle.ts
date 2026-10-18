// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * "Reload window (free memory)": reload this window's page to give back the
 * memory its renderer keeps after a long session, without the user losing
 * anything (docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §3).
 *
 * 1. Refuse while something a reload would lose is in flight (recycle-blockers.ts).
 * 2. Write what each pane keeps for a reload: snapshots, scroll positions, drafts.
 * 3. Ask the host to measure the renderer and keep a picture of the window, then
 *    look at the blockers again: anything started meanwhile cancels it.
 * 4. Reload; the new document shows that picture while it boots (`showReloadCover`).
 */

import { getApi } from "@/app/store/global";
import { pushFlashError } from "@/app/store/flash-notifications";
import { recycleBlockers } from "@/app/store/recycle-blockers";

function ownWindowLabel(): string {
    return new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
}

function refuse(reasons: string[]): void {
    pushFlashError({
        id: "",
        icon: "triangle-exclamation",
        title: "Can't reload the window yet",
        message: `${reasons.join("; ")}. Try again when that's done.`,
        expiration: Date.now() + 8_000,
    });
}

/** Set while a recycle runs: the page goes away at its end, so it is cleared only if it fails first. */
let recycling = false;

/** Test-only: forget a recycle in flight (a real one ends with the page). */
export function resetRecycleForTests(): void {
    recycling = false;
}

/**
 * Recycle this window's renderer now, unless something would be lost.
 * Resolves false when refused. `onRefused` replaces the toast (the governor's
 * request reports to the host instead of interrupting the user).
 */
export async function recycleWindow(
    reason = "command",
    onRefused: (reasons: string[]) => void = refuse
): Promise<boolean> {
    // A second call (a double Enter in the palette) would measure and reload twice.
    if (recycling) return false;
    const blocked = recycleBlockers();
    if (blocked.length > 0) {
        onRefused(blocked);
        return false;
    }
    recycling = true;
    const unsaved: string[] = [];
    try {
        const [{ flushPaneSnapshots }, { saveScrollMemos }, { saveAllComposerDrafts }, { saveAllAttachmentDrafts }] =
            await Promise.all([
                import("@/app/view/agent/hooks/useSnapshotPersistence"),
                import("@/app/view/agent/virtualization/scroll-memo"),
                import("@/app/view/agent/reload-persistence"),
                import("@/app/view/agent/attachments/attachment-draft"),
            ]);
        if (!(await flushPaneSnapshots())) unsaved.push("a pane's collapsed rows, pins or filter couldn't be saved");
        saveScrollMemos();
        if (!saveAllComposerDrafts()) unsaved.push("a draft couldn't be saved (the browser storage is full or off)");
        if (!saveAllAttachmentDrafts()) unsaved.push("an attachment couldn't be kept (the browser storage is full or off)");
    } catch (e) {
        recycling = false;
        throw e;
    }
    // A reload now would lose what didn't save.
    if (unsaved.length > 0) {
        recycling = false;
        onRefused(unsaved);
        return false;
    }
    try {
        await getApi().windows.recycleBegin(ownWindowLabel(), reason);
    } catch (e) {
        // No measurement and no cover picture; the reload still frees the memory.
        console.warn("recycle: the host didn't record the recycle", e);
    }
    // Saving and the cover can take seconds while the window stays usable:
    // whatever started meanwhile would be lost too, so look again.
    const late = recycleBlockers();
    if (late.length > 0) {
        recycling = false;
        void getApi().windows.recycleCancel(ownWindowLabel()).catch(() => {});
        onRefused(late);
        return false;
    }
    try {
        getApi().reloadWindow();
    } catch (e) {
        abandon();
        throw e;
    }
    // The page normally goes away now; if it is still here later, the reload
    // didn't happen, so free the command again.
    setTimeout(abandon, RELOAD_GRACE_MS);
    return true;
}

/** How long a reload may take to unload the page before it counts as not happening. */
const RELOAD_GRACE_MS = 15_000;

function abandon(): void {
    if (!recycling) return;
    recycling = false;
    void getApi().windows.recycleCancel(ownWindowLabel()).catch(() => {});
}

/** How long the window must have had no input, when the host's request doesn't say (spec §4.3). */
const IDLE_MS = 10 * 60_000;

/**
 * The governor's `recycle:request` (spec §4.3): recycle this window if nobody
 * has used it for the idle time and nothing would be lost; otherwise tell the
 * host why not, so it leaves the window alone for a while. Never a toast: the
 * user didn't ask for this.
 */
export async function answerRecycleRequest(idleMs = IDLE_MS): Promise<boolean> {
    const decline = (reasons: string[]) => {
        void getApi().windows.recycleDeclined(ownWindowLabel(), reasons.join("; ")).catch(() => {});
    };
    const [{ msSinceLastInput }, { anyAgentWaitingForYou }, { modalsModel }, { isAnyUnderway, ALL_DRAG_KINDS }] =
        await Promise.all([
            import("@/app/store/last-input"),
            import("@/app/notification/waiting-for-you"),
            import("@/app/store/modalmodel"),
            import("@/app/drag/drag-session"),
        ]);
    const busy: string[] = [];
    if (msSinceLastInput() < idleMs) busy.push("the window was used recently");
    if (anyAgentWaitingForYou()) busy.push("an agent is waiting for an answer");
    if (modalsModel.hasOpenModals()) busy.push("a dialog is open");
    if (isAnyUnderway(ALL_DRAG_KINDS)) busy.push("a drag is in progress");
    if (busy.length > 0) {
        decline(busy);
        return false;
    }
    return recycleWindow("idle", decline);
}

/** Answer the governor's requests in this window (once, at startup). */
export function installRecycleRequests(): void {
    void import("@/app/store/last-input").then(({ installLastInput }) => installLastInput());
    void getApi().listen<{ idle_ms?: number }>("recycle:request", (payload) => {
        // The host's idle time: shorter under memory pressure or a lab override.
        const idleMs = typeof payload?.idle_ms === "number" ? payload.idle_ms : IDLE_MS;
        void answerRecycleRequest(idleMs);
    });
}

/**
 * While booting: if a recycle reloaded this window, show its picture in the
 * startup splash until the app is back (the tear-off snapshot's mechanism).
 */
export async function showReloadCover(): Promise<void> {
    try {
        const { jpeg_base64 } = await getApi().windows.takeReloadSnapshot(ownWindowLabel());
        if (!jpeg_base64) return;
        const { showTearOffSnapshot } = await import("@/app/init/startup-splash");
        showTearOffSnapshot(jpeg_base64);
    } catch {
        // Not a recycle, or a host without it: the ordinary splash.
    }
}
