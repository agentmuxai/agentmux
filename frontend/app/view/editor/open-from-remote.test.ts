// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";

const openInPane = vi.hoisted(() => vi.fn(async () => {}));
vi.mock("@/app/view/files/files-open", () => ({ openInPane }));

import { editorHostScopes, openRemoteFile } from "./open-from-remote";

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
    remote("db1", { settings: { "display:name": "prod-db", "display:color": "#e5484d" } }),
    remote("web"),
    remote("wsl://Ubuntu", { kind: "wsl", sources: ["wsl"] }),
];
const values = (scopes: SuggestionConnectionScope[]) => scopes.flatMap((s) => s.items.map((i) => i.value));

describe("editorHostScopes", () => {
    it("offers the SSH hosts, in the picker's sections, and leaves WSL out", () => {
        const scopes = editorHostScopes(records, "", undefined, () => 1);
        expect(values(scopes)).toEqual(["db1", "web"]);
        expect(scopes[0].items[0].label).toBe("prod-db");
        expect(scopes[0].items[0].swatchColor).toBe("#e5484d");
    });

    it("offers a typed user@host that isn't a remote yet, and nothing extra for a listed one", () => {
        const typed = editorHostScopes(records, "me@box", undefined, () => 1);
        expect(typed.at(-1)).toMatchObject({ headerText: "New host", items: [{ value: "me@box", label: "me@box" }] });
        expect(values(editorHostScopes(records, "web", undefined, () => 1))).toEqual(["web"]);
        expect(values(editorHostScopes(records, "wsl://Ubuntu", undefined, () => 1))).toEqual([]);
    });
});

describe("openRemoteFile", () => {
    const editor = (connection?: string) => ({
        blockId: "ed-1",
        connection: () => connection,
        openFile: vi.fn(async () => {}),
    });
    beforeEach(() => openInPane.mockClear());

    it("opens in this editor when it is already on that host", async () => {
        const e = editor("db1");
        await openRemoteFile(e, "db1", " ~/.bashrc ");
        expect(e.openFile).toHaveBeenCalledWith("~/.bashrc");
        expect(openInPane).not.toHaveBeenCalled();
    });

    it("opens a new editor on the host beside this one otherwise", async () => {
        const e = editor(undefined);
        await openRemoteFile(e, "db1", "/etc/hosts");
        expect(openInPane).toHaveBeenCalledWith("editor", "/etc/hosts", "ed-1", "db1");
        expect(e.openFile).not.toHaveBeenCalled();
    });

    it("refuses an empty path", async () => {
        await expect(openRemoteFile(editor(), "db1", "  ")).rejects.toThrow(/path/);
        expect(openInPane).not.toHaveBeenCalled();
    });
});
