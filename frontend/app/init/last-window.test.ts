// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A new window opened while no window is (AgentMux stayed in the tray after
// its last window closed) brings that window back (SPEC_WORKSPACES_2026_10_23.md §2).

import { describe, expect, it } from "vitest";
import { restoresLastWindow } from "./last-window";

const plain = { windowIds: [] as string[], workspaceId: "", view: null, reattached: false };

describe("restoresLastWindow", () => {
    it("restores when no window is open", () => {
        expect(restoresLastWindow(plain)).toBe(true);
        expect(restoresLastWindow({ ...plain, windowIds: undefined })).toBe(true);
    });

    it("opens the default panes beside an open window, as New Window does", () => {
        expect(restoresLastWindow({ ...plain, windowIds: ["w1"] })).toBe(false);
        expect(restoresLastWindow({ ...plain, windowIds: ["w1"], otherLiveWindows: 1 })).toBe(false);
    });

    it("goes by the host's open windows: srv can still list the one just closed", () => {
        expect(restoresLastWindow({ ...plain, windowIds: ["closing"], otherLiveWindows: 0 })).toBe(true);
        expect(restoresLastWindow({ ...plain, windowIds: ["closing"], otherLiveWindows: null })).toBe(false);
    });

    it("opens what it was asked for: given contents, or a pane of its own", () => {
        expect(restoresLastWindow({ ...plain, workspaceId: "ws-1" })).toBe(false);
        expect(restoresLastWindow({ ...plain, view: "sysinfo" })).toBe(false);
        expect(restoresLastWindow({ ...plain, view: "credential-approval" })).toBe(false);
    });

    it("keeps its own window on a reload", () => {
        expect(restoresLastWindow({ ...plain, reattached: true })).toBe(false);
    });
});
