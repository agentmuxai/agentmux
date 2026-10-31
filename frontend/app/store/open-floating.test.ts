// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const paneOpen = vi.fn((..._args: unknown[]) => Promise.resolve({ block_id: "float-1" }));
vi.mock("./rpc-api", () => ({ RpcApi: { PaneOpenCommand: (...a: unknown[]) => paneOpen(...a) } }));
vi.mock("./rpc-util", () => ({ TabRpcClient: "client" }));
vi.mock("./global", () => ({ atoms: { activeTabId: () => "tab-7" } }));

import { openBlockFloating } from "./open-floating";

beforeEach(() => paneOpen.mockClear());

describe("openBlockFloating", () => {
    it("opens the view floating, in this window's tab, with its meta", async () => {
        const meta = { view: "kanban", "kanban:open": "12" };
        expect(await openBlockFloating({ meta } as BlockDef)).toBe("float-1");
        expect(paneOpen).toHaveBeenCalledWith("client", { view: "kanban", meta, floating: true, tab_id: "tab-7" }, {});
    });

    it("leaves a block it can't build from its meta alone to the docked path", async () => {
        expect(await openBlockFloating({ meta: {} } as BlockDef)).toBeNull();
        expect(await openBlockFloating({ meta: { view: "term" }, files: { "a.txt": {} } } as unknown as BlockDef)).toBeNull();
        expect(paneOpen).not.toHaveBeenCalled();
    });
});
