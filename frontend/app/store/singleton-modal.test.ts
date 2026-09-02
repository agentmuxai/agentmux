// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for the singleton-modal coordination layer (PR 3 of
// docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md). The side-effecting deps
// (HTTP publish, WPS subscribe, launcher events) are mocked so the
// pure acquire/release/holder decision logic is verified in isolation.

import { beforeEach, describe, expect, test, vi } from "vitest";
import { createRoot } from "solid-js";

// ── Mocks ──────────────────────────────────────────────────────────────

/** Records every claim published, so tests assert broadcast behaviour. */
const publishedClaims: Array<Record<string, unknown>> = [];

/** Captured launcher-event subscriber so tests can drive crash-release. */
let launcherCb: ((evt: { event: string; label?: string }) => void) | null = null;

/** Mockable window registry — `isWindowLive` reads this via the
 *  `@/store/global` mock. `[]` ⇒ registry treated as not-yet-populated. */
let mockWindowEntries: Array<{ label: string }> = [];

vi.mock("@/store/global", () => ({
    getApi: () => ({
        getWindowLabel: () => Promise.resolve("window-self"),
        getAuthKey: () => "test-key",
        focusWindow: () => Promise.resolve(),
    }),
    openWindowEntriesAtom: () => mockWindowEntries,
}));

vi.mock("@/util/endpoints", () => ({
    getWebServerEndpoint: () => "http://test",
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        // No persisted history by default — each test that needs replay
        // overrides this.
        EventReadHistoryCommand: vi.fn(() => Promise.resolve([])),
    },
}));

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

vi.mock("@/app/store/wps", () => ({
    // waveEventSubscribe is a no-op here — live broadcasts are simulated
    // directly via __applyClaimForTests.
    waveEventSubscribe: vi.fn(() => () => {}),
}));

vi.mock("@/util/launcher-events", () => ({
    subscribeLauncherEvent: vi.fn((cb: (evt: { event: string; label?: string }) => void) => {
        launcherCb = cb;
        return () => {};
    }),
}));

// `fetch` — capture the publish payload, succeed.
const fetchMock = vi.fn(async (_url: string, init?: { body?: string }) => {
    if (init?.body) publishedClaims.push(JSON.parse(init.body));
    return { ok: true, status: 200 } as Response;
});
vi.stubGlobal("fetch", fetchMock);

// Import AFTER mocks are registered.
import {
    acquireSingleton,
    releaseSingleton,
    holdsSingleton,
    singletonHolder,
    startSingletonCrashRelease,
    __resetSingletonForTests,
    __setMyLabelForTests,
    __applyClaimForTests,
} from "./singleton-modal";

/** Read a SolidJS signal outside a reactive root. */
function read<T>(signal: () => T): T {
    let val!: T;
    createRoot((dispose) => {
        val = signal();
        dispose();
    });
    return val;
}

const KIND = "test-kind";

beforeEach(() => {
    __resetSingletonForTests();
    __setMyLabelForTests("window-self");
    publishedClaims.length = 0;
    launcherCb = null;
    mockWindowEntries = [];
    fetchMock.mockClear();
});

// ── acquire / release ──────────────────────────────────────────────────

describe("acquireSingleton", () => {
    test("acquires a free singleton and becomes the holder", () => {
        expect(acquireSingleton(KIND)).toBe(true);
        expect(read(singletonHolder(KIND))).toBe("window-self");
        expect(holdsSingleton(KIND)).toBe(true);
    });

    test("broadcasts a claim with this window as holder", () => {
        acquireSingleton(KIND);
        expect(publishedClaims).toHaveLength(1);
        expect(publishedClaims[0].event).toBe("singleton:claim");
        expect(publishedClaims[0].persist).toBe(1);
        const data = publishedClaims[0].data as Record<string, unknown>;
        expect(data.holder).toBe("window-self");
        expect(data.kind).toBe(KIND);
    });

    test("re-acquiring a singleton this window already holds returns true", () => {
        expect(acquireSingleton(KIND)).toBe(true);
        expect(acquireSingleton(KIND)).toBe(true);
        expect(holdsSingleton(KIND)).toBe(true);
    });

    test("cannot acquire a singleton held by another window", () => {
        // Simulate a claim broadcast from a different window.
        __applyClaimForTests({ kind: KIND, holder: "window-other", epoch: 1 });
        expect(acquireSingleton(KIND)).toBe(false);
        expect(read(singletonHolder(KIND))).toBe("window-other");
        expect(holdsSingleton(KIND)).toBe(false);
    });

    test("publishes nothing when acquisition is refused", () => {
        __applyClaimForTests({ kind: KIND, holder: "window-other", epoch: 1 });
        publishedClaims.length = 0;
        acquireSingleton(KIND);
        expect(publishedClaims).toHaveLength(0);
    });

    test("returns false when this window's label is unresolved", () => {
        // Boot-instant call before getWindowLabel resolves: `true` must
        // mean "holds it NOW", which can't be honoured without a label.
        __setMyLabelForTests(null);
        expect(acquireSingleton(KIND)).toBe(false);
        expect(publishedClaims).toHaveLength(0);
    });

    test("refused when the holder is still a live window", () => {
        __applyClaimForTests({ kind: KIND, holder: "window-other", epoch: 1 });
        mockWindowEntries = [{ label: "window-self" }, { label: "window-other" }];
        expect(acquireSingleton(KIND)).toBe(false);
    });

    test("acquires over a stale holder whose window is no longer live", () => {
        // A persisted claim from a window that crashed — the launcher
        // registry no longer lists it, so it must not block forever.
        __applyClaimForTests({ kind: KIND, holder: "window-dead", epoch: 1 });
        mockWindowEntries = [{ label: "window-self" }];
        expect(acquireSingleton(KIND)).toBe(true);
        expect(read(singletonHolder(KIND))).toBe("window-self");
    });
});

describe("releaseSingleton", () => {
    test("the holder can release; holder becomes null", () => {
        acquireSingleton(KIND);
        releaseSingleton(KIND);
        expect(read(singletonHolder(KIND))).toBeNull();
        expect(holdsSingleton(KIND)).toBe(false);
    });

    test("release broadcasts a null-holder claim", () => {
        acquireSingleton(KIND);
        publishedClaims.length = 0;
        releaseSingleton(KIND);
        expect(publishedClaims).toHaveLength(1);
        const data = publishedClaims[0].data as Record<string, unknown>;
        expect(data.holder).toBeNull();
    });

    test("a non-holder release is a no-op (does not steal the claim)", () => {
        __applyClaimForTests({ kind: KIND, holder: "window-other", epoch: 1 });
        publishedClaims.length = 0;
        releaseSingleton(KIND);
        expect(read(singletonHolder(KIND))).toBe("window-other");
        expect(publishedClaims).toHaveLength(0);
    });

    test("after release the singleton can be acquired again", () => {
        acquireSingleton(KIND);
        releaseSingleton(KIND);
        expect(acquireSingleton(KIND)).toBe(true);
    });
});

// ── observability ──────────────────────────────────────────────────────

describe("singletonHolder", () => {
    test("starts null for an unknown kind", () => {
        expect(read(singletonHolder("never-claimed"))).toBeNull();
    });

    test("reflects an inbound claim broadcast from another window", () => {
        __applyClaimForTests({ kind: KIND, holder: "window-3", epoch: 5 });
        expect(read(singletonHolder(KIND))).toBe("window-3");
    });

    test("a release broadcast clears the holder", () => {
        __applyClaimForTests({ kind: KIND, holder: "window-3", epoch: 5 });
        __applyClaimForTests({ kind: KIND, holder: null, epoch: 6 });
        expect(read(singletonHolder(KIND))).toBeNull();
    });
});

// ── crash release ──────────────────────────────────────────────────────

describe("startSingletonCrashRelease", () => {
    test("releases a holder's claim when its window closes", () => {
        startSingletonCrashRelease();
        __applyClaimForTests({ kind: KIND, holder: "window-other", epoch: 1 });
        expect(launcherCb).not.toBeNull();

        // Launcher reports that the holding window exited.
        launcherCb!({ event: "window_closed", label: "window-other" });

        expect(read(singletonHolder(KIND))).toBeNull();
        // A release was broadcast on the dead holder's behalf.
        const releases = publishedClaims.filter(
            (c) => (c.data as Record<string, unknown>).holder === null,
        );
        expect(releases.length).toBeGreaterThanOrEqual(1);
    });

    test("window_instance_released also triggers crash release", () => {
        startSingletonCrashRelease();
        __applyClaimForTests({ kind: KIND, holder: "window-other", epoch: 1 });
        launcherCb!({ event: "window_instance_released", label: "window-other" });
        expect(read(singletonHolder(KIND))).toBeNull();
    });

    test("a window closing that does NOT hold the singleton leaves it intact", () => {
        startSingletonCrashRelease();
        __applyClaimForTests({ kind: KIND, holder: "window-other", epoch: 1 });
        launcherCb!({ event: "window_closed", label: "window-unrelated" });
        expect(read(singletonHolder(KIND))).toBe("window-other");
    });

    test("a non-exit launcher event does not release the singleton", () => {
        startSingletonCrashRelease();
        __applyClaimForTests({ kind: KIND, holder: "window-other", epoch: 1 });
        launcherCb!({ event: "window_opened", label: "window-other" });
        expect(read(singletonHolder(KIND))).toBe("window-other");
    });

    test("is idempotent — a second call does not re-subscribe", async () => {
        const mod = vi.mocked(
            (await import("@/util/launcher-events")).subscribeLauncherEvent,
        );
        startSingletonCrashRelease();
        const callsAfterFirst = mod.mock.calls.length;
        startSingletonCrashRelease();
        expect(mod.mock.calls.length).toBe(callsAfterFirst);
    });
});
