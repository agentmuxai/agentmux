// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Maintenance section's state (MaintenanceSection.tsx): the release srv found and its download, what the
// host updater says when there is none, the Store's part, the update buttons and Check for update. The
// pieces are made in the order the section always made them.

import { createSignal, onMount } from "solid-js";

import {
    appUpdate,
    availableRelease,
    cancelUpgrade,
    checkedAt,
    checkForUpdate,
    loadAppUpdate,
    recordStoreNone,
    releaseDownload,
    setStoreStayed,
    startAppUpdateDownload,
    storeAnswer,
    storeFallbackLine,
    storePublishedNotListed,
    storeStandingLine,
    upgradeAndRestart,
    useStoreOffer,
    viaStore,
} from "@/store/app-update";
import { atoms, getApi, openLink } from "@/store/global";

// The host opens exactly this link (`external_url_allowed` in the CEF host).
const MS_STORE_PAGE_URL = "ms-windows-store://pdp/?productid=9P9QCXNNCRK3";

/** The release on offer, its download, and what to say of it: from srv's check, else the host updater's own state. */
function createUpdateView() {
    const release = () => availableRelease();
    const download = () => (release() ? releaseDownload() : null);
    const updaterStatus = () => {
        if (!release()) return atoms.updaterStatusAtom();
        const d = download()?.state;
        if (d === "downloading" || d === "unpacking") return "downloading";
        if (d === "ready" || d === "waiting" || d === "restarting") return "ready";
        if (d === "failed") return "error";
        return "available";
    };
    const percent = () => {
        const d = download();
        return d && d.total > 0 ? Math.floor((d.received * 100) / d.total) : 0;
    };
    const updaterVersion = () => release()?.version ?? atoms.updaterVersionAtom();
    // Older than the release upgrades from: its floor version first, from its page.
    const installFirst = () => (release() ? appUpdate()?.install_first : undefined);
    // A Store install below that floor has no safe route: the Store installs only
    // the latest (which can't migrate this data), and a standalone build from the
    // page would sit beside the Store copy, not upgrade it.
    const storeBelowFloor = () => viaStore() && !!installFirst();
    // A Store install is updated by the Store: the release on offer is the one it lists, so there is no button until it lists one.
    const storeOffers = () => !viaStore() || storeAnswer()?.state === "available";
    // Where this AgentMux stands, beside Check for update. A Store install says it in the Store's terms.
    const standing = () => {
        const s = appUpdate();
        if (viaStore(s)) {
            const offer = storeAnswer(s);
            const line = storeStandingLine(offer, offer && offer.checkedMs > 0 ? checkedAt(offer.checkedMs) : null, storePublishedNotListed(s));
            return line ?? `Checked ${checkedAt(offer!.checkedMs)}`;
        }
        const checked = s && s.checked_ms > 0 ? checkedAt(s.checked_ms) : null;
        if (s?.state === "local-build" && s.latest) return `Development build. The latest release is v${s.latest}.`;
        if (s?.state === "up-to-date") return `v${s.current} is up to date${checked ? ` (checked ${checked})` : ""}`;
        if (s?.state === "available" && checked) return `Checked ${checked}`;
        return "Not checked for updates yet";
    };
    const badge = (): string | null => {
        const upd = updaterStatus();
        if (upd === "available") {
            const ver = updaterVersion();
            return ver ? `↓ ${ver}` : "↓ update";
        }
        if (upd === "downloading") return "↓ …";
        if (upd === "ready") return "↑ ready";
        return null;
    };
    // A release GitHub has that the Store has not listed for this PC yet: the panel says so and offers the Store page.
    const storeLagging = () => storePublishedNotListed() !== null;
    const openStorePage = () => void openLink(MS_STORE_PAGE_URL);
    return { release, download, updaterStatus, percent, updaterVersion, installFirst, storeBelowFloor, storeOffers, storeLagging, openStorePage, standing, badge };
}

/** Check for update, and what it said when it failed. */
function createCheck() {
    const [checking, setChecking] = createSignal(false);
    const [checkError, setCheckError] = createSignal<string | null>(null);
    const handleCheck = () => {
        setChecking(true);
        setCheckError(null);
        checkForUpdate()
            .catch((e) => setCheckError(String(e?.message ?? e)))
            .finally(() => setChecking(false));
    };
    return { checking, checkError, handleCheck };
}

/** The Maintenance section's state: everything its markup reads, and what its buttons do. */
export function createMaintenance() {
    // A newer release srv found (store/app-update.ts) leads; the host
    // updater's own states are kept for a host that has one.
    onMount(() => void loadAppUpdate());
    useStoreOffer(true);
    const view = createUpdateView();
    const { release, installFirst, storeBelowFloor } = view;
    const [startError, setStartError] = createSignal<string | null>(null);
    // What the Store said about an update asked of it, and whether it is working on one.
    const [storeNote, setStoreNote] = createSignal<string | null>(null);
    const [storeBusy, setStoreBusy] = createSignal(false);
    const check = createCheck();

    // Opens a folder, or shows a file in its folder; a failure is shown, like the other actions here.
    const reveal = async (path: string) => {
        setStartError(null);
        try {
            await getApi().revealInFileExplorer(path);
        } catch (e: any) {
            setStartError(`Couldn't open ${path}: ${String(e?.message ?? e)}`);
        }
    };
    const runUpgrade = (now: boolean) => {
        setStartError(null);
        upgradeAndRestart(now).catch((e) => setStartError(String(e?.message ?? e)));
    };
    const cancelRestart = () => cancelUpgrade().catch((e) => setStartError(String(e?.message ?? e)));

    // Ask the Store to install the update (the host calls its API, which only a
    // packaged install can use); when it can't, or the host has no such call,
    // open the Store page as before. A refusal says why, a cancel or "not there
    // yet" is just told.
    const updateInStore = async () => {
        if (storeBusy()) return;
        setStartError(null);
        setStoreNote(null);
        setStoreBusy(true);
        try {
            setStoreStayed(null);
            const result = await getApi()?.approvals?.storeUpdate?.(release()?.version).catch(() => undefined);
            if (result?.outcome === "none") recordStoreNone();
            if (!result || result.outcome === "fallback") {
                setStoreNote(storeFallbackLine(result?.message));
                void openLink(MS_STORE_PAGE_URL);
            } else {
                setStoreNote(result.message);
            }
        } finally {
            setStoreBusy(false);
        }
    };
    const handleInstallUpdate = () => {
        const r = release();
        const first = installFirst();
        if (storeBelowFloor()) return;
        if (first) {
            void openLink(`https://github.com/agentmuxai/agentmux/releases/tag/v${first}`);
            return;
        }
        if (r) {
            if (viaStore()) {
                void updateInStore();
                return;
            }
            // srv downloads, verifies and unpacks what it can (a Windows
            // portable); for any other install the release page is the way.
            if (!appUpdate()?.can_download) {
                void openLink(r.url);
                return;
            }
            setStartError(null);
            startAppUpdateDownload().catch((e) => setStartError(String(e?.message ?? e)));
            return;
        }
        getApi().installAppUpdate();
    };

    return {
        ...view, ...check, startError, storeNote, storeBusy,
        reveal, runUpgrade, cancelRestart, handleInstallUpdate,
    };
}

/** What the Maintenance section's markup reads. */
export type MaintenanceState = ReturnType<typeof createMaintenance>;
