// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
vi.mock("@/app/store/block-layout-actions", () => ({ createBlock: vi.fn() }));

import { buildHostSessionsMenu } from "./hostSessions";

describe("buildHostSessionsMenu", () => {
    const actions = () => ({ open: vi.fn(), end: vi.fn() });

    it("says so when there are none, or the host could not be asked", () => {
        expect(buildHostSessionsMenu("box", [], "b1", actions())).toEqual([
            { label: "No durable sessions on box", enabled: false },
        ]);
        const [err] = buildHostSessionsMenu("box", "Permission denied (publickey).", "b1", actions());
        expect(err.label).toContain("Permission denied");
        expect(err.enabled).toBe(false);
    });

    it("offers an orphan to open or end, and a held session only to end", () => {
        const a = actions();
        const menu = buildHostSessionsMenu(
            "box",
            [
                { id: "amx-orphan", bytes: 2048 },
                { id: "amx-mine", bytes: 10, blockid: "b1" },
                { id: "amx-theirs", bytes: 10, blockid: "b2" },
                { id: "amx-done", bytes: 10, exited: 0 },
            ],
            "b1",
            a
        );
        expect(menu.map((m) => m.label?.split(" · ")[2])).toEqual([
            "no pane",
            "this pane",
            "another pane",
            "ended (exit 0)",
        ]);
        expect(menu[0].submenu?.map((m) => m.label)).toEqual(["Open in a New Pane", "End Session (stops its shell)"]);
        menu[0].submenu?.[0].click?.();
        expect(a.open).toHaveBeenCalledWith("amx-orphan");
        expect(menu[1].submenu?.map((m) => m.label)).toEqual(["End Session (stops its shell)"]);
        menu[2].submenu?.[0].click?.();
        expect(a.end).toHaveBeenCalledWith("amx-theirs");
        // An exited session is shown, not offered.
        expect(menu[3].enabled).toBe(false);
        expect(menu[3].submenu).toBeUndefined();
    });
});
