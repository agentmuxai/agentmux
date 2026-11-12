// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Maintenance section's Microsoft Store part: "Update in the Store" (the Store's download, with its progress),
// and "Restart now" once the update is downloaded: the row asks, naming what a restart interrupts, and only then is
// the Store asked to install it.

import { createSignal } from "solid-js";

import {
    acceptStoreProgress,
    recordStoreNone,
    setStoreDownloaded,
    setStoreProgress,
    setStoreRestarting,
    setStoreStayed,
    storeDownloaded,
    storeFallbackLine,
    storeRestarting,
    storeTargetVersion,
} from "@/store/app-update";
import { getApi, openLink } from "@/store/global";

// The host opens exactly this link (`external_url_allowed` in the CEF host).
export const MS_STORE_PAGE_URL = "ms-windows-store://pdp/?productid=9P9QCXNNCRK3";

/** Who a restart interrupts, from srv's quit question: the agents in the middle of a turn and how many agents there are. */
export type RestartWho = { working: string[]; agents: number };

/**
 * Restart now, after a downloaded update: the row asks first (no window modal), naming the agents in the middle of a
 * turn, and only a click on Restart reaches the host. The host then asks the Store to install, which closes AgentMux
 * and every agent in it; a reopen is arranged before that. Whether the install call is open (`restarting`) is kept
 * outside this section, so the row still says so when it is opened again.
 */
function createStoreRestart(setStartError: (e: string | null) => void, setStoreNote: (n: string | null) => void) {
    const [restartAsking, setRestartAsking] = createSignal(false);
    const [restartWho, setRestartWho] = createSignal<RestartWho | null>(null);

    const askRestart = () => {
        setStartError(null);
        setRestartWho(null);
        setRestartAsking(true);
        void getApi()
            ?.approvals?.storeUpdateWorking?.()
            .then((who) => setRestartWho(who ?? null))
            .catch(() => undefined);
    };
    const cancelAskRestart = () => setRestartAsking(false);
    const refused = (why: string) => {
        setStartError(why);
        setStoreRestarting(false);
        setRestartAsking(false);
        setStoreProgress(null);
    };
    const restartForStore = async () => {
        if (storeRestarting()) return;
        setStartError(null);
        setStoreNote(null);
        setStoreProgress(null);
        setStoreRestarting(true);
        setRestartAsking(false);
        // The host says when the Store is asking the person, or installing, for as long as the install call is open.
        const stopListening = await getApi()?.listen?.("store-update-progress", acceptStoreProgress).catch(() => undefined);
        try {
            const r = await getApi()?.approvals?.storeUpdateRestart?.();
            if (r?.ok) return; // AgentMux is closing; the Store's swap and the reopen follow.
            if (r?.canceled) {
                // "Maybe later" in the Store's window: the update stays downloaded, Restart now stays available.
                setStoreNote(r.message);
                setStoreRestarting(false);
                setStoreProgress(null);
            } else {
                refused(r?.message || "This window can't restart AgentMux for the Store.");
            }
        } catch (e: any) {
            refused(String(e?.message ?? e));
        } finally {
            stopListening?.();
        }
    };

    return { restartAsking, restartWho, restarting: storeRestarting, askRestart, cancelAskRestart, restartForStore };
}

/**
 * What the Store said about an update asked of it, whether it is working on one, and the Restart now that follows a
 * downloaded one. `setStartError` shows a failure in the row.
 */
export function createStoreInstall(setStartError: (e: string | null) => void) {
    const [storeNote, setStoreNote] = createSignal<string | null>(null);
    const [storeBusy, setStoreBusy] = createSignal(false);
    // Ask the Store to download the update (the host calls its API, which only a packaged install can use); when it
    // can't, or the host has no such call, open the Store page as before. A refusal says why, a cancel or "not there
    // yet" is just told. Nothing is installed or asked of the person here: that is Restart now.
    const updateInStore = async () => {
        if (storeBusy() || storeRestarting()) return;
        setStartError(null);
        setStoreNote(null);
        setStoreBusy(true);
        setStoreProgress(null);
        setStoreDownloaded(null);
        // The host reports the Store's download as it goes, for the row to show.
        const stopListening = await getApi()?.listen?.("store-update-progress", acceptStoreProgress).catch(() => undefined);
        try {
            setStoreStayed(null);
            // For a Store install the offered release is the Store's own offer, the same source as the progress line.
            const version = storeTargetVersion();
            const result = await getApi()?.approvals?.storeUpdate?.(version).catch(() => undefined);
            if (result?.outcome === "none") recordStoreNone();
            if (result?.outcome === "downloaded") {
                // Downloaded and nothing more: the row says so and offers Restart now.
                setStoreDownloaded({ version });
            } else if (!result || result.outcome === "fallback") {
                setStoreNote(storeFallbackLine(result?.message));
                void openLink(MS_STORE_PAGE_URL);
            } else {
                setStoreNote(result.message);
            }
        } finally {
            stopListening?.();
            setStoreProgress(null);
            setStoreBusy(false);
        }
    };

    const restart = createStoreRestart(setStartError, setStoreNote);

    return {
        storeNote,
        storeBusy,
        storeDownloaded,
        updateInStore,
        ...restart,
    };
}
