// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import {
    displayName,
    groupRemotes,
    helperLabel,
    platformLabel,
    plural,
    remoteColor,
    sectionOf,
    statusLabel,
} from "./remotes-sections";

function remote(name: string, over: Partial<RemoteRecord> = {}): RemoteRecord {
    return {
        name,
        kind: "ssh",
        sources: ["ssh_config"],
        status: { state: "disconnected", error: "" },
        platform: null,
        helper: { state: "absent", version: "" },
        sessions: null,
        agents: [],
        settings: {},
        last_used_ms: null,
        ...over,
    };
}

describe("sectionOf", () => {
    it("puts each remote in exactly one section: pinned wins, then hidden, then kind and source", () => {
        expect(sectionOf(remote("a", { settings: { "display:pinned": true, "display:hidden": true } }))).toBe("pinned");
        expect(sectionOf(remote("a", { settings: { "display:hidden": true } }))).toBe("hidden");
        expect(sectionOf(remote("Ubuntu", { kind: "wsl", sources: ["wsl"] }))).toBe("wsl");
        expect(sectionOf(remote("db1", { sources: ["ssh_config", "recent"] }))).toBe("ssh");
        expect(sectionOf(remote("db2", { sources: ["settings"] }))).toBe("ssh");
        expect(sectionOf(remote("me@box", { sources: ["recent"] }))).toBe("recent");
    });
});

describe("groupRemotes", () => {
    const records = [
        remote("zeta"),
        remote("alpha"),
        remote("me@old", { sources: ["recent"], last_used_ms: 1 }),
        remote("me@new", { sources: ["recent"], last_used_ms: 9 }),
        remote("p2", { settings: { "display:pinned": true, "display:order": 2 } }),
        remote("p1", { settings: { "display:pinned": true, "display:order": 1 } }),
        remote("Ubuntu", { kind: "wsl", sources: ["wsl"] }),
        remote("gone", { settings: { "display:hidden": true } }),
    ];

    it("orders sections and sorts within each", () => {
        const groups = groupRemotes(records);
        expect(groups.map((g) => g.section)).toEqual(["pinned", "ssh", "recent", "wsl", "hidden"]);
        expect(groups.map((g) => g.records.map((r) => r.name))).toEqual([
            ["p1", "p2"],
            ["alpha", "zeta"],
            ["me@new", "me@old"],
            ["Ubuntu"],
            ["gone"],
        ]);
    });

    it("leaves out empty sections and filters by name or nickname", () => {
        const withNick = [...records, remote("db1", { settings: { "display:name": "prod-db" } })];
        expect(groupRemotes(withNick, "PROD").map((g) => [g.section, g.records.map((r) => r.name)])).toEqual([
            ["ssh", ["db1"]],
        ]);
        expect(groupRemotes(withNick, "me@").map((g) => g.section)).toEqual(["recent"]);
        expect(groupRemotes(withNick, "nothing")).toEqual([]);
    });
});

describe("labels", () => {
    it("uses the nickname when set", () => {
        expect(displayName(remote("db1", { settings: { "display:name": " prod-db " } }))).toBe("prod-db");
        expect(displayName(remote("db1", { settings: { "display:name": "  " } }))).toBe("db1");
    });

    it("accepts only #rrggbb colours", () => {
        expect(remoteColor(remote("a", { settings: { "display:color": "#e5484d" } }))).toBe("#e5484d");
        expect(remoteColor(remote("a", { settings: { "display:color": "red;background:url(x)" } }))).toBeUndefined();
    });

    it("describes platform, helper and status", () => {
        expect(platformLabel({ os: "linux", arch: "x86_64" })).toBe("Linux · x86_64");
        expect(platformLabel({ os: "macos", arch: "arm64" })).toBe("macOS · arm64");
        expect(platformLabel(null)).toBe("");
        expect(helperLabel({ state: "installed", version: "0.59.9" })).toBe("Helper 0.59.9");
        expect(helperLabel({ state: "never", version: "" })).toBe("Helper: never");
        expect(helperLabel({ state: "none", version: "" })).toBe("");
        expect(statusLabel({ state: "error", error: "timed out" })).toBe("Error: timed out");
        expect(statusLabel({ state: "connected", error: "" })).toBe("Connected");
        expect(plural(1, "session")).toBe("1 session");
        expect(plural(2, "agent")).toBe("2 agents");
    });
});
