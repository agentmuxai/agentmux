// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    appUpdate,
    availableRelease,
    cancelUpgrade,
    checkedAt,
    checkForUpdate,
    loadAppUpdate,
    releaseDownload,
    startAppUpdateDownload,
    upgradeAndRestart,
    upgradeUnderWay,
} from "@/store/app-update";
import { atoms, getApi, openLink } from "@/store/global";
import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import "./_maintenance-section.scss";
import { Button } from "@/app/element/ui";

// The host opens exactly this link (`external_url_allowed` in the CEF host).
const MS_STORE_PAGE_URL = "ms-windows-store://pdp/?productid=9P9QCXNNCRK3";

// ── Component ────────────────────────────────────────────────────────────────

export const MaintenanceSection = (): JSX.Element => {
    // A newer release srv found (store/app-update.ts) leads; the host
    // updater's own states are kept for a host that has one.
    onMount(() => void loadAppUpdate());
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
    const [startError, setStartError] = createSignal<string | null>(null);
    const [checking, setChecking] = createSignal(false);
    const [checkError, setCheckError] = createSignal<string | null>(null);
    const handleCheck = () => {
        setChecking(true);
        setCheckError(null);
        checkForUpdate()
            .catch((e) => setCheckError(String(e?.message ?? e)))
            .finally(() => setChecking(false));
    };
    // Where this AgentMux stands, beside Check for update. Nothing checks on
    // its own, so before the first check it says so.
    const standing = () => {
        const s = appUpdate();
        const checked = s && s.checked_ms > 0 ? checkedAt(s.checked_ms) : null;
        if (s?.state === "local-build" && s.latest) return `Development build. The latest release is v${s.latest}.`;
        if (s?.state === "up-to-date") return `v${s.current} is up to date${checked ? ` (checked ${checked})` : ""}`;
        if (s?.state === "available" && checked) return `Checked ${checked}`;
        return "Not checked for updates yet";
    };
    const updaterVersion = () => release()?.version ?? atoms.updaterVersionAtom();
    const [showNotes, setShowNotes] = createSignal(false);

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

    // A Microsoft Store install is updated by the Store, not by AgentMux.
    const viaStore = () => appUpdate()?.install === "windows-store";
    // Older than the release upgrades from: its floor version first, from its page.
    const installFirst = () => (release() ? appUpdate()?.install_first : undefined);
    // A Store install below that floor has no safe route: the Store installs only
    // the latest (which can't migrate this data), and a standalone build from the
    // page would sit beside the Store copy, not upgrade it.
    const storeBelowFloor = () => viaStore() && !!installFirst();
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
                void openLink(MS_STORE_PAGE_URL);
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

    // ── Derived ───────────────────────────────────────────────────────────────

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

    // ── Render ────────────────────────────────────────────────────────────────

    return (
        <>
            <div class="instance-panel-section-title">
                Maintenance
                <Show when={badge() != null}>
                    <span class="maintenance-badge">{badge()}</span>
                </Show>
            </div>

            {/* ── Update row ─────────────────────────────────────────────── */}
            <Show when={updaterStatus() !== "up-to-date" && updaterStatus() !== "checking"}>
                <div class="maintenance-update-row">
                    <Show when={updaterStatus() === "available"}>
                        <span class="maintenance-icon maintenance-icon--update">↓</span>
                        <span class="maintenance-row-text">
                            {updaterVersion() ? `v${updaterVersion()} is out` : "Update available"}
                        </span>
                        <Show when={release() && appUpdate()?.notes}>
                            <Button
                                density="compact"
                                class="maintenance-btn"
                                aria-expanded={showNotes()}
                                onClick={() => setShowNotes(!showNotes())}
                            >
                                What's new
                            </Button>
                        </Show>
                        <Show when={!storeBelowFloor()}>
                            <Button
                                density="compact"
                                tone="accent"
                                class="maintenance-btn maintenance-btn--primary"
                                onClick={handleInstallUpdate}
                            >
                                {installFirst() ? `Get v${installFirst()}` : viaStore() ? "Update in the Store" : "Download"}
                            </Button>
                        </Show>
                    </Show>
                    <Show when={updaterStatus() === "downloading"}>
                        <span class="maintenance-icon maintenance-icon--working">↓</span>
                        <span class="maintenance-row-text maintenance-row-text--dim">
                            {download()?.state === "unpacking"
                                ? `Unpacking v${updaterVersion()}…`
                                : download()
                                  ? `Downloading v${updaterVersion()}… ${percent()}%`
                                  : "Downloading…"}
                        </span>
                    </Show>
                    {/* Staged by srv: restart into it, once agents finish their turn. */}
                    <Show when={download()?.state === "ready" && (download()?.folder || download()?.installer)}>
                        <span class="maintenance-icon maintenance-icon--update">↑</span>
                        <span class="maintenance-row-text" title={download()?.folder ?? download()?.installer}>
                            v{updaterVersion()} is ready
                        </span>
                        <Show when={download()?.folder}>
                            <Button
                                density="compact"
                                class="maintenance-btn"
                                onClick={() => void reveal(download()!.folder!)}
                            >
                                Show folder
                            </Button>
                        </Show>
                        {/* An installer (or an AppImage's next version) is shown in its folder,
                            never opened: that would run it. */}
                        <Show when={download()?.installer}>
                            <Button
                                density="compact"
                                class="maintenance-btn"
                                onClick={() => void reveal(download()!.installer!)}
                            >
                                {appUpdate()?.install === "linux-appimage" ? "Show file" : "Show installer"}
                            </Button>
                        </Show>
                        <Button
                            density="compact"
                            tone="accent"
                            class="maintenance-btn maintenance-btn--primary"
                            onClick={() => runUpgrade(false)}
                        >
                            Upgrade &amp; restart
                        </Button>
                    </Show>
                    <Show when={download()?.state === "waiting"}>
                        <span class="maintenance-icon maintenance-icon--working">↑</span>
                        <span class="maintenance-row-text maintenance-row-text--dim">
                            {download()!.waiting_for > 0
                                ? `Waiting for ${download()!.waiting_for} agent${download()!.waiting_for === 1 ? "" : "s"} to finish…`
                                : "Restarting soon…"}
                        </span>
                        <Button density="compact" class="maintenance-btn" onClick={() => runUpgrade(true)}>
                            Restart now
                        </Button>
                        <Button density="compact" class="maintenance-btn" onClick={() => cancelUpgrade().catch((e) => setStartError(String(e?.message ?? e)))}>
                            Cancel
                        </Button>
                    </Show>
                    <Show when={download()?.state === "restarting"}>
                        <span class="maintenance-icon maintenance-icon--working">↑</span>
                        {/* Only the launcher's dialog can stop it now: No there cancels. */}
                        <span class="maintenance-row-text maintenance-row-text--dim">
                            Restarting: answer the AgentMux dialog (No cancels)
                        </span>
                    </Show>
                    <Show when={updaterStatus() === "ready" && !download()}>
                        <span class="maintenance-icon maintenance-icon--update">↑</span>
                        <span class="maintenance-row-text">
                            {updaterVersion() ? `v${updaterVersion()} ready` : "Ready to install"}
                        </span>
                        <Button
                            density="compact"
                            tone="accent"
                            class="maintenance-btn maintenance-btn--primary"
                            onClick={handleInstallUpdate}
                        >
                            Restart
                        </Button>
                    </Show>
                    <Show when={updaterStatus() === "error"}>
                        <span class="maintenance-icon maintenance-icon--error">✗</span>
                        <span class="maintenance-row-text maintenance-row-text--error" title={download()?.error}>
                            {download()?.error ? `Download failed: ${download()!.error}` : "Update failed"}
                        </span>
                        <Show when={download()}>
                            <Button density="compact" class="maintenance-btn" onClick={handleInstallUpdate}>
                                Try again
                            </Button>
                            <Button density="compact" class="maintenance-btn" onClick={() => void openLink(release()!.url)}>
                                Release page
                            </Button>
                        </Show>
                    </Show>
                </div>
            </Show>

            {/* What agents left running that the restart stops (srv's background_work). */}
            <Show when={(download()?.state === "waiting" || download()?.state === "restarting") && (download()?.stopping?.length ?? 0) > 0}>
                <div class="maintenance-row-text maintenance-row-text--warn">The restart also stops what agents left running:</div>
                <For each={download()!.stopping}>
                    {(line) => <div class="maintenance-row-text maintenance-row-text--dim">{line}</div>}
                </For>
            </Show>
            <Show when={download()?.state === "ready" && download()?.error}>
                <div class="maintenance-row-text maintenance-row-text--error">{download()!.error}</div>
            </Show>
            <Show when={updaterStatus() === "available" && viaStore() && !storeBelowFloor()}>
                <div class="maintenance-row-text maintenance-row-text--dim">
                    {`The Microsoft Store updates this install. It gets each release after GitHub does, so v${updaterVersion()} may not be there yet.`}
                </div>
            </Show>
            <Show when={installFirst() && !viaStore()}>
                <div class="maintenance-row-text maintenance-row-text--warn">
                    This version is older than v{updaterVersion()} upgrades from: install v{installFirst()} first, then upgrade.
                </div>
            </Show>
            <Show when={storeBelowFloor()}>
                <div class="maintenance-row-text maintenance-row-text--warn">
                    {`This version is older than v${updaterVersion()} upgrades from (v${installFirst()} or later). The Microsoft Store can only install the latest version, so AgentMux does not offer the update here.`}
                </div>
            </Show>
            <Show when={startError()}>
                <div class="maintenance-row-text maintenance-row-text--error">{startError()}</div>
            </Show>
            <Show when={showNotes() && release() && appUpdate()?.notes}>
                <pre class="maintenance-release-notes">{appUpdate()?.notes}</pre>
            </Show>
            {/* Check for update, always: nothing checks on its own, and a new
                check replaces the release on offer (not while a download or
                restart into it is under way). A `task dev` build, which
                doesn't upgrade, learns which release is latest. */}
            <div class="maintenance-update-row">
                <span class="maintenance-row-text maintenance-row-text--dim maintenance-row-text--wrap">{standing()}</span>
                <Show when={appUpdate()?.state === "local-build" && appUpdate()?.url}>
                    <Button density="compact" class="maintenance-btn" onClick={() => void openLink(appUpdate()!.url!)}>
                        View
                    </Button>
                </Show>
                <Button
                    density="compact"
                    class="maintenance-btn"
                    disabled={checking() || upgradeUnderWay()}
                    onClick={handleCheck}
                >
                    {checking() ? "Checking…" : release() ? "Check again" : "Check for update"}
                </Button>
            </div>
            <Show when={checkError()}>
                <div class="maintenance-row-text maintenance-row-text--error">{checkError()}</div>
            </Show>
        </>
    );
};

MaintenanceSection.displayName = "MaintenanceSection";
