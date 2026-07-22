// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for seedGlobalLogin's defensive error handling (reagent P0 on
 * #2255): `seed_provider_auth_from_global` hard-rejects (throws) for every
 * provider except claude — this function must never let that throw
 * propagate as an unhandled rejection, since it's a shared utility called
 * by more than one caller.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    seedProviderAuthFromGlobal: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    getApi: () => ({ seedProviderAuthFromGlobal: hub.seedProviderAuthFromGlobal }),
}));

import { seedGlobalLogin } from "./seed-global-login";

beforeEach(() => {
    hub.seedProviderAuthFromGlobal.mockReset();
});
afterEach(() => {
    vi.clearAllMocks();
});

describe("seedGlobalLogin", () => {
    it("returns true on a successful seed", async () => {
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: true });
        const result = await seedGlobalLogin("claude", vi.fn(), "/some/dir");
        expect(result).toBe(true);
    });

    it("returns false (not throw) when the host reports no valid global login", async () => {
        hub.seedProviderAuthFromGlobal.mockResolvedValue({ seeded: false, status: "missing" });
        const result = await seedGlobalLogin("claude", vi.fn(), "/some/dir");
        expect(result).toBe(false);
    });

    it("returns false (not throw) when the host command rejects — e.g. a non-claude provider, which seed_provider_auth_from_global hard-rejects server-side", async () => {
        hub.seedProviderAuthFromGlobal.mockRejectedValue(
            new Error("seed_provider_auth_from_global: only supported for claude"),
        );
        const log = vi.fn();
        await expect(seedGlobalLogin("codex", log, "/some/dir")).resolves.toBe(false);
        expect(log).toHaveBeenCalledWith("auth", expect.stringMatching(/seed-from-global failed/i), "warn");
    });
});
