// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { MuxObjUpdate: "waveobj:update" } }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://localhost" }));
vi.mock("@/util/fetchutil", () => ({ fetch: () => new Promise(() => {}) }));
vi.mock("@/app/store/services", () => ({ ObjectService: { UpdateObject: async () => {} } }));
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
// block-component-registry.ts pulls in @/layout/index transitively, whose
// module graph reaches global.ts's full re-export surface — partial-mock via
// importOriginal rather than hand-listing every atom global.ts happens to
// re-export today (brittle: would need updating every time that list grows).
vi.mock("@/app/store/config-signals", async (importOriginal) => {
    const actual = await importOriginal<typeof import("@/app/store/config-signals")>();
    return { ...actual, fullConfigAtom: () => null, settingsAtom: () => null };
});

import { collectAtomCacheDiagnostics, logAtomCacheDiagnostics } from "./atom-cache-diagnostic";

describe("atom-cache-diagnostic", () => {
    it("collectAtomCacheDiagnostics returns the shape of all three underlying stats", () => {
        const snapshot = collectAtomCacheDiagnostics();
        expect(snapshot.muxObjectCache).toEqual(
            expect.objectContaining({
                totalEntries: expect.any(Number),
                pinnedEntries: expect.any(Number),
                totalRefCount: expect.any(Number),
            }),
        );
        expect(snapshot.blockAtomCache).toEqual(
            expect.objectContaining({
                cachedBlockCount: expect.any(Number),
                totalMemoCount: expect.any(Number),
            }),
        );
        expect(snapshot.registry).toEqual(
            expect.objectContaining({
                registeredCount: expect.any(Number),
                dormantCount: expect.any(Number),
            }),
        );
    });

    // Regression: the first shipped version used console.debug, which
    // log-pipe.ts forwards to the Rust host as tracing::debug! — below the
    // host's default "info" EnvFilter, so it never appeared in a real log
    // despite firing correctly (confirmed live via CDP against a running
    // build). console.log maps to tracing::info!, matching mem_attribution's
    // own level. Pinned here so a future edit can't silently regress back to
    // a level that's invisible by default.
    it("logAtomCacheDiagnostics uses console.log (not console.debug), so it survives the host's default info-level filter", () => {
        const debugSpy = vi.spyOn(console, "debug").mockImplementation(() => {});
        const logSpy = vi.spyOn(console, "log").mockImplementation(() => {});
        expect(() => logAtomCacheDiagnostics()).not.toThrow();

        expect(debugSpy).not.toHaveBeenCalled();
        expect(logSpy).toHaveBeenCalledTimes(1);
        const [tag, payload] = logSpy.mock.calls[0];
        expect(tag).toBe("[atom-cache-diag]");
        expect(() => JSON.parse(payload as string)).not.toThrow();

        debugSpy.mockRestore();
        logSpy.mockRestore();
    });
});
