// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Maintenance section's markup in parts (MaintenanceSection.tsx): the update row (a branch for each state
// the update is in), the lines under it, and the Check for update row. Each reads the section's state
// (maintenance-state.ts).

import { For, Show, type JSX } from "solid-js";

import { tip } from "@/app/element/tip";
import { Button } from "@/app/element/ui";
import { appUpdate, releaseNotesUrl, upgradeUnderWay, viaStore } from "@/store/app-update";
import { openLink } from "@/store/global";
import type { MaintenanceState } from "./maintenance-state";
import { StoreUpdateNotes } from "./StoreUpdateNotes";

// What to do with a Store install older than the release's upgrade floor.
const STORE_MOVE_OFF_URL = "https://docs.agentmux.ai/installation/#moving-off-the-microsoft-store-build";

type Part = { m: MaintenanceState };

/** A newer release: what is out, What's new, and the download (or the Store's update, or the floor version). */
function AvailableRow(props: Part): JSX.Element {
    const m = props.m;
    return (
        <Show when={m.updaterStatus() === "available"}>
            <span class="maintenance-icon maintenance-icon--update">↓</span>
            <span class="maintenance-row-text">
                {m.updaterVersion() ? (viaStore() ? `v${m.updaterVersion()} is in the Microsoft Store` : `v${m.updaterVersion()} is out`) : "Update available"}
            </span>
            <Button density="compact" class="maintenance-btn" onClick={() => void openLink(releaseNotesUrl(m.updaterVersion()))}>
                What's new
            </Button>
            <Show when={!m.storeBelowFloor() && m.storeOffers()}>
                <Button
                    density="compact"
                    tone="accent"
                    class="maintenance-btn maintenance-btn--primary"
                    onClick={m.handleInstallUpdate}
                >
                    {m.installFirst() ? `Get v${m.installFirst()}` : viaStore() ? "Update in the Store" : "Download"}
                </Button>
            </Show>
        </Show>
    );
}

function DownloadingRow(props: Part): JSX.Element {
    const m = props.m;
    return (
        <Show when={m.updaterStatus() === "downloading"}>
            <span class="maintenance-icon maintenance-icon--working">↓</span>
            <span class="maintenance-row-text maintenance-row-text--dim">
                {m.download()?.state === "unpacking"
                    ? `Unpacking v${m.updaterVersion()}…`
                    : m.download()
                      ? `Downloading v${m.updaterVersion()}… ${m.percent()}%`
                      : "Downloading…"}
            </span>
        </Show>
    );
}

/** Staged by srv: restart into it, once agents finish their turn. */
function StagedRow(props: Part): JSX.Element {
    const m = props.m;
    const download = m.download;
    return (
        <Show when={download()?.state === "ready" && (download()?.folder || download()?.installer)}>
            <span class="maintenance-icon maintenance-icon--update">↑</span>
            <span class="maintenance-row-text" {...tip(download()?.folder ?? download()?.installer)}>
                v{m.updaterVersion()} is ready
            </span>
            <Show when={download()?.folder}>
                <Button
                    density="compact"
                    class="maintenance-btn"
                    onClick={() => void m.reveal(download()!.folder!)}
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
                    onClick={() => void m.reveal(download()!.installer!)}
                >
                    {appUpdate()?.install === "linux-appimage" ? "Show file" : "Show installer"}
                </Button>
            </Show>
            <Button
                density="compact"
                tone="accent"
                class="maintenance-btn maintenance-btn--primary"
                onClick={() => m.runUpgrade(false)}
            >
                Upgrade &amp; restart
            </Button>
        </Show>
    );
}

function WaitingRow(props: Part): JSX.Element {
    const m = props.m;
    const download = m.download;
    return (
        <Show when={download()?.state === "waiting"}>
            <span class="maintenance-icon maintenance-icon--working">↑</span>
            <span class="maintenance-row-text maintenance-row-text--dim">
                {download()!.waiting_for > 0
                    ? `Waiting for ${download()!.waiting_for} agent${download()!.waiting_for === 1 ? "" : "s"} to finish…`
                    : "Restarting soon…"}
            </span>
            <Button density="compact" class="maintenance-btn" onClick={() => m.runUpgrade(true)}>
                Restart now
            </Button>
            <Button density="compact" class="maintenance-btn" onClick={() => m.cancelRestart()}>
                Cancel
            </Button>
        </Show>
    );
}

function RestartingRow(props: Part): JSX.Element {
    return (
        <Show when={props.m.download()?.state === "restarting"}>
            <span class="maintenance-icon maintenance-icon--working">↑</span>
            {/* Only the launcher's dialog can stop it now: No there cancels. */}
            <span class="maintenance-row-text maintenance-row-text--dim">
                Restarting: answer the AgentMux dialog (No cancels)
            </span>
        </Show>
    );
}

/** The host updater's own "ready" state, when srv has no download of it. */
function HostReadyRow(props: Part): JSX.Element {
    const m = props.m;
    return (
        <Show when={m.updaterStatus() === "ready" && !m.download()}>
            <span class="maintenance-icon maintenance-icon--update">↑</span>
            <span class="maintenance-row-text">
                {m.updaterVersion() ? `v${m.updaterVersion()} ready` : "Ready to install"}
            </span>
            <Button
                density="compact"
                tone="accent"
                class="maintenance-btn maintenance-btn--primary"
                onClick={m.handleInstallUpdate}
            >
                Restart
            </Button>
        </Show>
    );
}

function FailedRow(props: Part): JSX.Element {
    const m = props.m;
    const download = m.download;
    return (
        <Show when={m.updaterStatus() === "error"}>
            <span class="maintenance-icon maintenance-icon--error">✗</span>
            <span class="maintenance-row-text maintenance-row-text--error" {...tip(download()?.error)}>
                {download()?.error ? `Download failed: ${download()!.error}` : "Update failed"}
            </span>
            <Show when={download()}>
                <Button density="compact" class="maintenance-btn" onClick={m.handleInstallUpdate}>
                    Try again
                </Button>
                <Button density="compact" class="maintenance-btn" onClick={() => void openLink(m.release()!.url)}>
                    Release page
                </Button>
            </Show>
        </Show>
    );
}

/** The update row: one branch for each state the update is in. */
export function UpdateRow(props: Part): JSX.Element {
    const m = props.m;
    return (
        <Show when={m.updaterStatus() !== "up-to-date" && m.updaterStatus() !== "checking"}>
            <div class="maintenance-update-row">
                <AvailableRow m={m} />
                <DownloadingRow m={m} />
                <StagedRow m={m} />
                <WaitingRow m={m} />
                <RestartingRow m={m} />
                <HostReadyRow m={m} />
                <FailedRow m={m} />
            </div>
        </Show>
    );
}

/** The lines under the update row: what the restart stops, why the update can't go ahead, a failed action, the Store's notes. */
export function Notices(props: Part): JSX.Element {
    const m = props.m;
    const download = m.download;
    return (
        <>
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
            <Show when={m.installFirst() && !viaStore()}>
                <div class="maintenance-row-text maintenance-row-text--warn">
                    This version is older than v{m.updaterVersion()} upgrades from: install v{m.installFirst()} first, then upgrade.
                </div>
            </Show>
            <Show when={m.storeBelowFloor()}>
                <div class="maintenance-row-text maintenance-row-text--warn">
                    {`This version is older than v${m.updaterVersion()} upgrades from (v${m.installFirst()} or later). The Microsoft Store can only install the latest version, so AgentMux does not offer the update here. To update, move to the GitHub build: install v${m.installFirst()} from GitHub first, then the latest.`}
                </div>
                <Button density="compact" class="maintenance-btn" onClick={() => void openLink(STORE_MOVE_OFF_URL)}>
                    How to move off the Store build
                </Button>
            </Show>
            <Show when={m.startError()}>
                <div class="maintenance-row-text maintenance-row-text--error">{m.startError()}</div>
            </Show>
            <StoreUpdateNotes
                closesFor={viaStore() && !m.storeBelowFloor() && m.storeOffers() && m.updaterStatus() === "available" ? m.updaterVersion() : null}
                busy={m.storeBusy()}
                note={m.storeNote()}
            />
        </>
    );
}

/**
 * Check for update, always: nothing checks on its own, and a new check replaces the release on offer (not
 * while a download or restart into it is under way). A `task dev` build, which doesn't upgrade, learns which
 * release is latest.
 */
export function CheckRow(props: Part): JSX.Element {
    const m = props.m;
    return (
        <>
            <div class="maintenance-update-row">
                <span class="maintenance-row-text maintenance-row-text--dim">{m.standing()}</span>
                <Show when={m.storeLagging()}>
                    <Button density="compact" class="maintenance-btn" onClick={m.openStorePage}>
                        Open Store page
                    </Button>
                </Show>
                <Show when={appUpdate()?.state === "local-build" && appUpdate()?.url}>
                    <Button density="compact" class="maintenance-btn" onClick={() => void openLink(appUpdate()!.url!)}>
                        View
                    </Button>
                </Show>
                <Button
                    density="compact"
                    class="maintenance-btn"
                    disabled={m.checking() || upgradeUnderWay()}
                    onClick={m.handleCheck}
                >
                    {m.checking() ? "Checking…" : m.release() ? "Check again" : "Check for update"}
                </Button>
            </div>
            <Show when={m.checkError()}>
                <div class="maintenance-row-text maintenance-row-text--error">{m.checkError()}</div>
            </Show>
        </>
    );
}
