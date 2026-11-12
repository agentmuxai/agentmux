// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Maintenance section's Microsoft Store part: "Update in the Store" (the Store's download and install, with its
// progress), and "Restart now" once the update is downloaded and the Store waits for AgentMux to be closed.

import { createSignal } from "solid-js";

import {
    acceptStoreProgress,
    recordStoreNone,
    setStoreProgress,
    setStoreStaged,
    setStoreStayed,
    storeFallbackLine,
    storeStaged,
    storeTargetVersion,
} from "@/store/app-update";
import { getApi, openLink } from "@/store/global";

// The host opens exactly this link (`external_url_allowed` in the CEF host).
export const MS_STORE_PAGE_URL = "ms-windows-store://pdp/?productid=9P9QCXNNCRK3";

/**
 * What the Store said about an update asked of it, whether it is working on one, and the Restart now that follows a
 * staged one. `release` is the update on offer; `setStartError` shows a failure in the row.
 */
export function createStoreInstall(release: () => { version: string } | null, setStartError: (e: string | null) => void) {
    const [storeNote, setStoreNote] = createSignal<string | null>(null);
    const [storeBusy, setStoreBusy] = createSignal(false);
    // Restart now after a staged update: whether the row is asking "are you sure", and whether it has started.
    const [restartAsking, setRestartAsking] = createSignal(false);
    const [restarting, setRestarting] = createSignal(false);

    // Ask the Store to install the update (the host calls its API, which only a
    // packaged install can use); when it can't, or the host has no such call,
    // open the Store page as before. A refusal says why, a cancel or "not there
    // yet" is just told.
    const updateInStore = async () => {
        if (storeBusy()) return;
        setStartError(null);
        setStoreNote(null);
        setStoreBusy(true);
        setStoreProgress(null);
        setStoreStaged(null);
        // The host reports the Store's download and install as it goes, for the row to show.
        const stopListening = await getApi()?.listen?.("store-update-progress", acceptStoreProgress).catch(() => undefined);
        try {
            setStoreStayed(null);
            const version = release()?.version || storeTargetVersion();
            const result = await getApi()?.approvals?.storeUpdate?.(version).catch(() => undefined);
            if (result?.outcome === "none") recordStoreNone();
            if (result?.outcome === "staged") {
                // Downloaded; the Store waits for AgentMux to close. The row says so and offers Restart now.
                setStoreStaged({ version });
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

    // Restart now, after a staged update: closes AgentMux and every agent in it and opens it again once the Store has
    // swapped the package. The row asks first (no window modal), and only this click reaches the host.
    const askRestart = () => setRestartAsking(true);
    const cancelAskRestart = () => setRestartAsking(false);
    const refused = (why: string) => {
        setStartError(why);
        setRestarting(false);
        setRestartAsking(false);
    };
    const restartForStore = async () => {
        if (restarting()) return;
        setStartError(null);
        setRestarting(true);
        try {
            const r = await getApi()?.approvals?.storeUpdateRestart?.();
            if (!r?.ok) refused(r?.message || "This window can't restart AgentMux for the Store.");
        } catch (e: any) {
            refused(String(e?.message ?? e));
        }
    };

    return { storeNote, storeBusy, storeStaged, restartAsking, restarting, updateInStore, askRestart, cancelAskRestart, restartForStore };
}
