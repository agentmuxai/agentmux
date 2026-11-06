// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The profile list holds the user's profiles and each agent's own (made by AgentMux the first time that
 * agent opens a browser). The user's menu and Settings list show them apart.
 */
import { describe, expect, it, vi } from "vitest";

const list = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListBrowserProfilesCommand: (...a: unknown[]) => list(...a) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/theme/palette", () => ({ farthestFreeSlot: () => ({ hex: "#d33" }) }));

import { agentProfiles, loadBrowserProfiles, userProfiles } from "./browser-profiles";

const profile = (id: string, name: string, owner_agent?: string) => ({
    id,
    name,
    color: "#3b82f6",
    created_at: 0,
    agents_allowed: false,
    ...(owner_agent ? { owner_agent } : {}),
});

describe("browser profiles", () => {
    it("keeps the user's profiles and the agents' own apart", async () => {
        list.mockResolvedValue({
            profiles: [profile("p-work", "Work"), profile("agent-camper-2", "camper-2", "camper-2"), profile("p-bank", "Bank")],
            signins: {},
        });
        await loadBrowserProfiles();
        expect(userProfiles().map((p) => p.name)).toEqual(["Work", "Bank"]);
        expect(agentProfiles().map((p) => p.name)).toEqual(["camper-2"]);
    });
});
