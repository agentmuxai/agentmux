// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Whether the person's Check for update goes on to download the release it found (`downloadFollowsCheck`).
import { describe, expect, it } from "vitest";
import { downloadFollowsCheck, type AppUpdateStatus } from "./app-update";

const FOUND: AppUpdateStatus = {
    state: "available",
    current: "0.60.33",
    latest: "0.60.34",
    url: "https://github.com/agentmuxai/agentmux/releases/tag/v0.60.34",
    checked_ms: 1,
    install: "windows-setup",
    can_download: true,
};
const download = (state: string) => ({ ...FOUND, download: { version: "0.60.34", state, received: 0, total: 0, waiting_for: 0, stopping: [] } }) as AppUpdateStatus;

describe("downloadFollowsCheck", () => {
    it("downloads the release a check found, for every install that can download it", () => {
        for (const install of ["windows-setup", "windows-portable", "macos-app", "linux-appimage", "linux-portable", "linux-package"]) {
            expect(downloadFollowsCheck({ ...FOUND, install }, true), install).toBe(true);
        }
    });

    it("keeps the Download button as the second step when the person turned downloads off", () => {
        expect(downloadFollowsCheck(FOUND, false)).toBe(false);
    });

    it("downloads nothing when there is no release, or it is not one this install can fetch", () => {
        expect(downloadFollowsCheck(null, true)).toBe(false);
        expect(downloadFollowsCheck({ ...FOUND, state: "up-to-date" }, true)).toBe(false);
        expect(downloadFollowsCheck({ ...FOUND, state: "local-build" }, true)).toBe(false);
        expect(downloadFollowsCheck({ ...FOUND, state: "unknown", latest: undefined, url: undefined }, true)).toBe(false);
        expect(downloadFollowsCheck({ ...FOUND, can_download: false }, true)).toBe(false);
        expect(downloadFollowsCheck({ ...FOUND, install_first: "0.55.0" }, true)).toBe(false);
    });

    it("leaves a Microsoft Store install to the Store", () => {
        expect(downloadFollowsCheck({ ...FOUND, install: "windows-store" }, true)).toBe(false);
    });

    it("starts no second download, and does not retry one that failed or is staged", () => {
        for (const state of ["downloading", "unpacking", "ready", "waiting", "restarting", "failed"]) {
            expect(downloadFollowsCheck(download(state), true), state).toBe(false);
        }
    });

    it("does not count a download of another release", () => {
        const older = { ...FOUND, download: { version: "0.60.33", state: "ready", received: 0, total: 0, waiting_for: 0, stopping: [] } } as AppUpdateStatus;
        expect(downloadFollowsCheck(older, true)).toBe(true);
    });
});
