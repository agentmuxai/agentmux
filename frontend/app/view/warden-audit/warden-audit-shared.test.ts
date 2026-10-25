// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The audit fetch returns every entry the ring buffer holds, registrations included. */

import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/auth-headers", () => ({ authHeaders: () => ({}) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://srv" }));

import { fetchWardenAudit } from "./warden-audit-shared";

afterEach(() => vi.unstubAllGlobals());

describe("fetchWardenAudit", () => {
    it("keeps register and unregister entries beside deliveries", async () => {
        const rows = [
            { event_kind: "delivery", target_agent: "a" },
            { event_kind: "register", target_agent: "b" },
            { event_kind: "unregister", target_agent: "c" },
            { target_agent: "d" },
        ];
        const fetchMock = vi.fn().mockResolvedValue({ ok: true, json: async () => rows });
        vi.stubGlobal("fetch", fetchMock);
        const entries = await fetchWardenAudit();
        expect(entries.map((e) => e.target_agent)).toEqual(["a", "b", "c", "d"]);
        expect(fetchMock.mock.calls[0][0]).toBe("http://srv/agentmux/reactive/audit?limit=50");
    });

    it("throws on a failed request", async () => {
        vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: false, status: 500 }));
        await expect(fetchWardenAudit()).rejects.toThrow("500");
    });
});
