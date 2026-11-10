// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The profile list holds the user's profiles and each agent's own (made by AgentMux the first time that
 * agent opens a browser). The user's menu and Settings list show them apart.
 */
import { createRoot } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const list = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { ListBrowserProfilesCommand: (...a: unknown[]) => list(...a) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/theme/palette", () => ({ farthestFreeSlot: () => ({ hex: "#d33" }) }));
const handlers = new Map<string, () => void>();
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (name: string, handler: () => void) => {
        handlers.set(name, handler);
        return () => handlers.delete(name);
    },
}));
vi.mock("@/app/store/ws", () => ({ addWSReconnectHandler: () => {} }));

import { agentProfiles, followBrowserProfiles, loadBrowserProfiles, profileById, userProfiles } from "./browser-profiles";

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

    describe("following the list", () => {
        afterEach(() => vi.useRealTimers());

        it("shows an agent's own profile as soon as srv says the list changed, with no menu opened", async () => {
            vi.useFakeTimers();
            list.mockResolvedValue({ profiles: [profile("p-work", "Work")], signins: {} });
            let dispose = () => {};
            createRoot((d) => {
                dispose = d;
                followBrowserProfiles();
            });
            await vi.advanceTimersByTimeAsync(0);
            expect(profileById("agent-camper-2")).toBeUndefined();
            // The agent opens its first browser: srv makes its profile and publishes the change.
            list.mockResolvedValue({
                profiles: [profile("p-work", "Work"), { ...profile("agent-camper-2", "camper-2", "camper-2"), color: "#e11" }],
                signins: {},
            });
            handlers.get("browserprofiles:changed")!();
            await vi.advanceTimersByTimeAsync(400);
            expect(profileById("agent-camper-2")?.color).toBe("#e11");
            dispose();
            expect(handlers.has("browserprofiles:changed")).toBe(false);
        });
    });
});
