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

/** Recycle this window's renderer now, unless something would be lost. Resolves false when refused. */
export async function recycleWindow(reason = "command"): Promise<boolean> {
    // A second call (a double Enter in the palette) would measure and reload twice.
    if (recycling) return false;
    const blocked = recycleBlockers();
    if (blocked.length > 0) {
        refuse(blocked);
        return false;
    }
    recycling = true;
    try {
        const [{ flushPaneSnapshots }, { saveScrollMemos }, { flushComposerDrafts }] = await Promise.all([
            import("@/app/view/agent/hooks/useSnapshotPersistence"),
            import("@/app/view/agent/virtualization/scroll-memo"),
            import("@/app/view/agent/reload-persistence"),
        ]);
        await flushPaneSnapshots();
        saveScrollMemos();
        flushComposerDrafts();
    } catch (e) {
        recycling = false;
        throw e;
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
        refuse(late);
        return false;
    }
    getApi().reloadWindow();
    return true;
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
