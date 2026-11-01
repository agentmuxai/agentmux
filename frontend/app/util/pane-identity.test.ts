// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { holdsLiveProcess, paneIdentity } from "./pane-identity";

describe("paneIdentity", () => {
    it("is the agent of an agent pane, the file of an Editor pane and the profile of a browser pane", () => {
        expect(paneIdentity("agent", { agentId: " a1 " })).toBe("agent:a1");
        expect(paneIdentity("editor", { file: "C:/notes.md" })).toBe("file:C:/notes.md");
        expect(paneIdentity("browser", { "browser:identity": "profile:work" })).toBe("profile:work");
    });

    it("is nothing for a pane that does not name one, so it is looked for in this window only", () => {
        expect(paneIdentity("agent", {})).toBeUndefined();
        expect(paneIdentity("agent", { agentId: "  " })).toBeUndefined();
        expect(paneIdentity("editor", undefined)).toBeUndefined();
        expect(paneIdentity("browser", {})).toBeUndefined();
        expect(paneIdentity("browser", { "browser:identity": "incognito:0123456789abcdef" })).toBeUndefined();
        expect(paneIdentity("settings", { file: "x" })).toBeUndefined();
        expect(paneIdentity(undefined, { agentId: "a1" })).toBeUndefined();
    });
});

describe("holdsLiveProcess", () => {
    it("is true for terminals and agents only", () => {
        expect(["term", "agent"].map(holdsLiveProcess)).toEqual([true, true]);
        expect(["editor", "browser", "settings", undefined].map(holdsLiveProcess)).toEqual([false, false, false, false]);
    });
});
