// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

const env = vi.hoisted(() => ({ backend: "views" as string | undefined }));
vi.mock("@/util/app-api", () => ({
    getApi: () => ({ getAboutModalDetails: () => ({ paneBackend: env.backend }) }),
}));

import { isViewsPanesOnWindows } from "./pane-backend";

const WINDOWS = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";
const LINUX = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36";

function at(userAgent: string, search: string): void {
    vi.spyOn(navigator, "userAgent", "get").mockReturnValue(userAgent);
    window.history.replaceState(null, "", `/${search}`);
}

afterEach(() => {
    vi.restoreAllMocks();
    window.history.replaceState(null, "", "/");
    env.backend = "views";
});

describe("isViewsPanesOnWindows", () => {
    it("is the host's pane kind in an app window on Windows", () => {
        at(WINDOWS, "");
        expect(isViewsPanesOnWindows()).toBe(true);
        at(WINDOWS, "?windowLabel=window-pool-1");
        expect(isViewsPanesOnWindows()).toBe(true);
        env.backend = "hwnd";
        expect(isViewsPanesOnWindows()).toBe(false);
        env.backend = undefined;
        expect(isViewsPanesOnWindows()).toBe(false);
    });

    it("is never true in a floating pane's window, whose panes stay child windows", () => {
        at(WINDOWS, "?windowLabel=floating-1f0c");
        expect(isViewsPanesOnWindows()).toBe(false);
        at(WINDOWS, "?windowLabel=floating-pool-77aa");
        expect(isViewsPanesOnWindows()).toBe(false);
    });

    it("is never true off Windows", () => {
        at(LINUX, "");
        expect(isViewsPanesOnWindows()).toBe(false);
    });
});
