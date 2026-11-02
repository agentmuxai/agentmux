// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { holdsLiveProcess, paneIdentity, staysInItsWindow } from "./pane-identity";

describe("paneIdentity", () => {
    it("is the agent of an agent pane, the file of an Editor pane and the profile of a browser pane", () => {
        expect(paneIdentity("agent", { agentId: " a1 " })).toBe("agent:a1");
        expect(paneIdentity("editor", { file: "C:/notes.md" })).toBe("file:c:/notes.md");
        expect(paneIdentity("browser", { "browser:identity": "profile:work" })).toBe("profile:work");
    });

    it("is the document in front, not the last one opened, once tabs are switched or closed", () => {
        const doctabs = (active: number, ...paths: string[]) => ({ v: 1, active, tabs: paths.map((path) => ({ key: path, title: path, state: { path } })) });
        expect(paneIdentity("editor", { file: "/b", doctabs: doctabs(0, "/a", "/b") })).toBe("file:/a");
        expect(paneIdentity("editor", { file: "/b", doctabs: doctabs(1, "/a", "/b") })).toBe("file:/b");
        expect(paneIdentity("editor", { file: "/b", doctabs: doctabs(0, "/a") })).toBe("file:/a");
        expect(paneIdentity("editor", { file: "/b", doctabs: { v: 1, active: 0, tabs: [{ key: "s", title: "s", state: {} }] } })).toBeUndefined();
        expect(paneIdentity("editor", { file: "C:\\Repo\\a.ts", doctabs: undefined })).toBe("file:c:/Repo/a.ts");
    });

    it("tells the same path on another host apart, and counts local as no host", () => {
        const id = (connection: unknown) => paneIdentity("editor", { file: "/etc/hosts", connection });
        expect(id("user@box")).toBe("file@user@box:/etc/hosts");
        expect(id("user@box")).not.toBe(id(undefined));
        expect([id(""), id("local"), id("wsl://Ubuntu")]).toEqual(["file:/etc/hosts", "file:/etc/hosts", "file:/etc/hosts"]);
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

describe("staysInItsWindow", () => {
    it("is true for a terminal, an agent and an Editor, which a move would cut off or lose text from", () => {
        expect(["term", "agent", "editor"].map(staysInItsWindow)).toEqual([true, true, true]);
        expect(["browser", "settings", undefined].map(staysInItsWindow)).toEqual([false, false, false]);
    });
});
