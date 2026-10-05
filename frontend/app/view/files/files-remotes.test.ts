// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import type { RemoteRecord } from "@/app/store/rpc-api/remotes";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: vi.fn(() => () => {}) }));

import { hangarRemotes } from "./files-model";

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

describe("Hangar's Remote section", () => {
    it("lists SSH remotes in the Remotes order, with nickname and colour, without WSL or hidden ones", () => {
        const got = hangarRemotes([
            remote("web"),
            remote("me@box", { sources: ["recent"] }),
            remote("db1", { settings: { "display:pinned": true, "display:name": "prod-db", "display:color": "#0090ff" } }),
            remote("wsl://Ubuntu", { kind: "wsl", sources: ["wsl"] }),
            remote("old", { settings: { "display:hidden": true } }),
        ]);
        expect(got).toEqual([
            { name: "db1", label: "prod-db", color: "#0090ff" },
            { name: "web", label: "web", color: undefined },
            { name: "me@box", label: "me@box", color: undefined },
        ]);
    });
});
