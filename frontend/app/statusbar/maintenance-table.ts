// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Every line the Maintenance section can show, and the buttons under it, in one place: a table of the states
// the update can be in (UPDATE_ROWS), the lines about why it cannot go ahead or what a restart does (NOTICE_ROWS),
// the Store's own lines (STORE_ROWS) and the standing line with Check for update (CHECK_ROWS). Each definition is a
// function of the section's state (maintenance-state.ts) that returns the rows to show: none, or one or more, each a
// message and its buttons. maintenance-rows.tsx draws every row the same way; nothing here knows about layout.

import { appUpdate, releaseNotesUrl, storeProgress, storeProgressLine, storeClosesLine, storeDownloadedLine, storeRestartAsk, storeStayed, storeStayedLine, storeTargetVersion, upgradeUnderWay, viaPackage, viaStore } from "@/store/app-update";
import { openLink } from "@/store/global";
import type { MaintenanceState } from "./maintenance-state";
import { restartingLine } from "./update-wait";

/** What to do with a Store install older than the release's upgrade floor. */
export const STORE_MOVE_OFF_URL = "https://docs.agentmux.ai/installation/#moving-off-the-microsoft-store-build";

/** What the row says while the Store downloads and has given no numbers yet. */
export const STORE_ASKING_LINE = "Asking the Microsoft Store to download the update…";

/** What the row says after Restart, until the host reports that the Store is asking or installing. */
export const STORE_PREPARING_LINE = "Getting the install ready…";

/** The colour of a message: the plain text colour when none. */
export type RowTone = "dim" | "warn" | "error";

/** The glyph before a message, and what it stands for. */
export type RowIcon = { glyph: string; tone: "update" | "working" | "error" };

/** A button under a message. `primary` is the one the row leads to. */
export type RowAction = { label: string; onClick: () => void; primary?: boolean; disabled?: boolean };

/** One message with its buttons (zero to three). `tip` is the full text for a hover, when the row shows less. */
export type RowSpec = { icon?: RowIcon; tone?: RowTone; text: string; tip?: string; actions: RowAction[] };

/** A definition: its name, and the rows it gives for the section's state (none when it does not apply). */
export type RowDef = { id: string; rows: (m: MaintenanceState) => RowSpec[] };

const one = (spec: RowSpec): RowSpec[] => [spec];
const none: RowSpec[] = [];

// ── The update row: one definition for each state the update is in ──────────────────────────────

/** The update row shows for any state but up to date and checking. */
const updateShown = (m: MaintenanceState): boolean => m.updaterStatus() !== "up-to-date" && m.updaterStatus() !== "checking";

const available = (m: MaintenanceState): RowSpec[] => {
    if (m.updaterStatus() !== "available") return none;
    const version = m.updaterVersion();
    const first = m.installFirst();
    const actions: RowAction[] = [{ label: "What's new", onClick: () => void openLink(releaseNotesUrl(version)) }];
    if (!m.storeBelowFloor() && m.storeOffers()) {
        actions.push({
            label: first ? `Get v${first}` : viaStore() ? "Update in the Store" : m.opensDownloadPage() ? "Open the download page" : "Download",
            onClick: m.handleInstallUpdate,
            primary: true,
        });
    }
    return one({
        icon: { glyph: "↓", tone: "update" },
        text: version ? (viaStore() ? `v${version} is in the Microsoft Store` : `v${version} is out`) : "Update available",
        actions,
    });
};

const downloading = (m: MaintenanceState): RowSpec[] => {
    if (m.updaterStatus() !== "downloading") return none;
    const state = m.download()?.state;
    return one({
        icon: { glyph: "↓", tone: "working" },
        tone: "dim",
        text: state === "unpacking" ? `Unpacking v${m.updaterVersion()}…` : m.download() ? `Downloading v${m.updaterVersion()}… ${m.percent()}%` : "Downloading…",
        actions: [],
    });
};

/** Staged by srv: restart into it, once agents finish their turn. */
const staged = (m: MaintenanceState): RowSpec[] => {
    const d = m.download();
    if (!(d?.state === "ready" && (d.folder || d.installer))) return none;
    const actions: RowAction[] = [];
    if (d.folder) actions.push({ label: "Show folder", onClick: () => void m.reveal(d.folder!) });
    // An installer (or an AppImage's next version) is shown in its folder, never opened: that would run it.
    if (d.installer) actions.push({ label: appUpdate()?.install === "linux-appimage" ? "Show file" : "Show installer", onClick: () => void m.reveal(d.installer!) });
    actions.push({ label: "Upgrade & restart", onClick: () => m.runUpgrade(false), primary: true });
    return one({ icon: { glyph: "↑", tone: "update" }, text: `v${m.updaterVersion()} is ready`, tip: d.folder ?? d.installer, actions });
};

const waiting = (m: MaintenanceState): RowSpec[] => {
    const d = m.download();
    if (d?.state !== "waiting") return none;
    return one({
        icon: { glyph: "↑", tone: "working" },
        tone: "dim",
        text: d.waiting_for > 0 ? `Waiting for ${d.waiting_for} agent${d.waiting_for === 1 ? "" : "s"} to finish…` : "Restarting soon…",
        actions: [
            { label: "Restart now", onClick: () => m.runUpgrade(true) },
            { label: "Cancel", onClick: () => void m.cancelRestart() },
        ],
    });
};

/**
 * A restart that is decided is sent to the launcher as confirmed, and the launcher restarts without asking again, so no
 * dialog of ours opens at this step and nothing here can cancel it. The one thing that can still be asked is on a
 * `.deb` or `.rpm` install: the system's own permission prompt for the install, which refusing (or leaving
 * unanswered) turns into a restart that did not happen.
 */
const restarting = (m: MaintenanceState): RowSpec[] => {
    if (m.download()?.state !== "restarting") return none;
    return one({
        icon: { glyph: "↑", tone: "working" },
        tone: "dim",
        text: restartingLine(m.updaterVersion(), viaPackage()),
        actions: [],
    });
};

/** The host updater's own "ready" state, when srv has no download of it. */
const hostReady = (m: MaintenanceState): RowSpec[] =>
    m.updaterStatus() === "ready" && !m.download()
        ? one({
              icon: { glyph: "↑", tone: "update" },
              text: m.updaterVersion() ? `v${m.updaterVersion()} ready` : "Ready to install",
              actions: [{ label: "Restart", onClick: m.handleInstallUpdate, primary: true }],
          })
        : none;

const failed = (m: MaintenanceState): RowSpec[] => {
    if (m.updaterStatus() !== "error") return none;
    const d = m.download();
    return one({
        icon: { glyph: "✗", tone: "error" },
        tone: "error",
        text: d?.error ? `Download failed: ${d.error}` : "Update failed",
        tip: d?.error,
        actions: d
            ? [
                  { label: "Try again", onClick: m.handleInstallUpdate },
                  { label: "Release page", onClick: () => void openLink(m.release()!.url) },
              ]
            : [],
    });
};

/** The update row's states, in the order they are tried; at most one applies at a time. */
export const UPDATE_ROWS: RowDef[] = [
    { id: "available", rows: available },
    { id: "downloading", rows: downloading },
    { id: "staged", rows: staged },
    { id: "waiting", rows: waiting },
    { id: "restarting", rows: restarting },
    { id: "host-ready", rows: hostReady },
    { id: "failed", rows: failed },
].map((def) => ({ ...def, rows: (m: MaintenanceState) => (updateShown(m) ? def.rows(m) : none) }));

// ── The lines under the update row ──────────────────────────────────────────────────────────────

/** What agents left running that the restart stops (srv's background_work): a heading, then a line each. */
const stopping = (m: MaintenanceState): RowSpec[] => {
    const d = m.download();
    const lines = d?.stopping ?? [];
    if (!((d?.state === "waiting" || d?.state === "restarting") && lines.length > 0)) return none;
    return [
        { tone: "warn", text: "The restart also stops what agents left running:", actions: [] },
        ...lines.map((line): RowSpec => ({ tone: "dim", text: line, actions: [] })),
    ];
};

const readyError = (m: MaintenanceState): RowSpec[] => {
    const d = m.download();
    return d?.state === "ready" && d.error ? one({ tone: "error", text: d.error, actions: [] }) : none;
};

const installFirst = (m: MaintenanceState): RowSpec[] =>
    m.installFirst() && !viaStore()
        ? one({ tone: "warn", text: `This version is older than v${m.updaterVersion()} upgrades from: install v${m.installFirst()} first, then upgrade.`, actions: [] })
        : none;

const storeBelowFloor = (m: MaintenanceState): RowSpec[] =>
    m.storeBelowFloor()
        ? one({
              tone: "warn",
              text: `This version is older than v${m.updaterVersion()} upgrades from (v${m.installFirst()} or later). The Microsoft Store can only install the latest version, so AgentMux does not offer the update here. To update, move to the GitHub build: install v${m.installFirst()} from GitHub first, then the latest.`,
              actions: [{ label: "How to move off the Store build", onClick: () => void openLink(STORE_MOVE_OFF_URL) }],
          })
        : none;

const downloadBlocked = (m: MaintenanceState): RowSpec[] => (m.downloadBlocked() ? one({ tone: "dim", text: m.downloadBlocked()!, actions: [] }) : none);

const startError = (m: MaintenanceState): RowSpec[] => (m.startError() ? one({ tone: "error", text: m.startError()!, actions: [] }) : none);

/** Why the update can't go ahead, a failed action, what a restart does. */
export const NOTICE_ROWS: RowDef[] = [
    { id: "stopping", rows: stopping },
    { id: "ready-error", rows: readyError },
    { id: "install-first", rows: installFirst },
    { id: "store-below-floor", rows: storeBelowFloor },
    { id: "download-blocked", rows: downloadBlocked },
    { id: "start-error", rows: startError },
];

// ── The Store's own lines (what "Update in the Store" and its Restart now say) ───────────────────

/** The version on offer ("" when the Store named none), or null when nothing is offered. */
const closesFor = (m: MaintenanceState): string | null =>
    viaStore() && !m.storeBelowFloor() && m.storeOffers() && m.updaterStatus() === "available" ? (m.updaterVersion() ?? "") : null;

const storeLine = (m: MaintenanceState): RowSpec[] => {
    const rows: RowSpec[] = [];
    const staged = m.storeDownloaded();
    const progressLine = (fallback: string) => storeProgressLine(storeProgress(), storeTargetVersion()) ?? fallback;
    const closes = closesFor(m);
    if (closes !== null && !staged) rows.push({ tone: "dim", text: storeClosesLine(closes), actions: [] });
    if (storeStayed()) rows.push({ tone: "error", text: storeStayedLine(storeStayed()!), actions: [] });
    if (m.storeBusy() && !m.restarting()) rows.push({ tone: "dim", text: progressLine(STORE_ASKING_LINE), actions: [] });
    if (staged && !m.storeBusy()) {
        const downloaded: RowSpec = { tone: "dim", text: storeDownloadedLine(staged.version), actions: [] };
        if (!m.restarting() && !m.restartAsking()) downloaded.actions.push({ label: "Restart now", onClick: m.askRestart });
        rows.push(downloaded);
        if (m.restartAsking() && !m.restarting()) {
            rows.push({
                tone: "warn",
                text: storeRestartAsk(m.restartWho()?.working ?? [], m.restartWho()?.agents ?? 0),
                actions: [
                    { label: "Restart", onClick: () => void m.restartForStore(), primary: true },
                    { label: "Cancel", onClick: m.cancelAskRestart },
                ],
            });
        }
        if (m.restarting()) rows.push({ tone: "dim", text: progressLine(STORE_PREPARING_LINE), actions: [] });
    }
    if (m.storeNote()) rows.push({ tone: "dim", text: m.storeNote()!, actions: [] });
    return rows;
};

export const STORE_ROWS: RowDef[] = [{ id: "store", rows: storeLine }];

// ── The standing line and Check for update, always ───────────────────────────────────────────────

/**
 * Check for update, always: srv also checks on its own every few hours, and a check here replaces the release on
 * offer (not while a download or restart into it is under way). A release it finds is downloaded at once when this
 * install can (the Download button is then the way back after a failure, or when `update:autodownload` is off); the
 * restart that installs it stays a click. A `task dev` build, which doesn't upgrade, learns which release is latest.
 */
const check = (m: MaintenanceState): RowSpec[] => {
    const actions: RowAction[] = [];
    if (m.storeLagging()) actions.push({ label: "Open Store page", onClick: m.openStorePage });
    const local = appUpdate();
    if (local?.state === "local-build" && local.url) actions.push({ label: "View", onClick: () => void openLink(local.url!) });
    actions.push({
        label: m.checking() ? "Checking…" : m.release() ? "Check again" : "Check for update",
        onClick: m.handleCheck,
        disabled: m.checking() || upgradeUnderWay(),
    });
    const rows: RowSpec[] = [{ tone: "dim", text: m.standing(), actions }];
    if (m.checkError()) rows.push({ tone: "error", text: m.checkError()!, actions: [] });
    return rows;
};

export const CHECK_ROWS: RowDef[] = [{ id: "check", rows: check }];

/** Every definition, in the order the section shows them. */
export const ALL_ROWS: RowDef[] = [...UPDATE_ROWS, ...NOTICE_ROWS, ...STORE_ROWS, ...CHECK_ROWS];
