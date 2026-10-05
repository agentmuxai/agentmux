// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import { remoteSuggestionScopes } from "./conn-remote-items";

function remote(name: string, over: Partial<RemoteRecord> = {}): RemoteRecord {
    return {
        name,
        kind: "ssh",
        sources: ["ssh_config"],
        status: { state: "disconnected", error: "" },
        platform: null,
        helper: { state: "absent", version: "", installed: false },
        sessions: null,
        agents: [],
        settings: {},
        last_used_ms: null,
        ...over,
    };
}

const records = [
    remote("db1", {
        settings: { "display:name": "prod-db", "display:color": "#e5484d", "display:pinned": true },
        status: { state: "connected", error: "" },
    }),
    remote("web", { platform: { os: "linux", arch: "x86_64" } }),
    remote("me@box", { sources: ["recent"], last_used_ms: 3 }),
    remote("wsl://Ubuntu", { kind: "wsl", sources: ["wsl"] }),
    remote("old", { settings: { "display:hidden": true } }),
];

describe("the picker's remote sections", () => {
    it("are the Remotes pane's sections, without Hidden", () => {
        const scopes = remoteSuggestionScopes(records, "", undefined, () => 2);
        expect(scopes.map((s) => [s.headerText, s.items.map((i) => i.value)])).toEqual([
            ["Pinned", ["db1"]],
            ["SSH hosts", ["web"]],
            ["Recent", ["me@box"]],
            ["WSL", ["wsl://Ubuntu"]],
        ]);
    });

    it("show the nickname, the real name, the colour and the status", () => {
        const [pinned, ssh] = remoteSuggestionScopes(records, "", "db1", () => 2);
        const db1 = pinned.items[0];
        expect(db1.label).toBe("prod-db");
        expect(db1.detail).toBe("db1");
        expect(db1.swatchColor).toBe("#e5484d");
        expect(db1.iconColor).toBe("var(--conn-icon-color-2)");
        expect(db1.current).toBe(true);
        expect(ssh.items[0].detail).toBe("Linux · x86_64");
        expect(ssh.items[0].iconColor).toBe("var(--grey-text-color)");
    });

    it("filter on what is typed, and keep a hidden remote the pane is on", () => {
        expect(remoteSuggestionScopes(records, "prod", undefined, () => 1).map((s) => s.headerText)).toEqual(["Pinned"]);
        const onHidden = remoteSuggestionScopes(records, "", "old", () => 1);
        expect(onHidden.at(-1)).toMatchObject({ headerText: "Hidden", items: [{ value: "old", current: true }] });
        // A hidden remote's name typed in full reaches it; a part of it doesn't list it.
        expect(remoteSuggestionScopes(records, "old", undefined, () => 1)).toMatchObject([
            { headerText: "Hidden", items: [{ value: "old" }] },
        ]);
        expect(remoteSuggestionScopes(records, "ol", undefined, () => 1)).toEqual([]);
    });
});
