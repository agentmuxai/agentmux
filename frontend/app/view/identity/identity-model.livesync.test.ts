// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Tests for the account-cache live sync: primeAccountCache() must install
// exactly one app-lifetime subscription to the backend's
// `identityaccounts:changed` broadcast, and that subscription's handler
// must refresh the cache — so accounts created by backend-originated flows
// (in-app OAuth login persist, API-key verify, upserts from another tab)
// appear in the Armory / launch modal / pickers without a reload.

import { beforeEach, describe, expect, it, vi } from "vitest";

const subscribeCalls: Array<{ eventType: string; handler: () => void }> = [];
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: () => void }) => {
        subscribeCalls.push(sub);
        return () => {};
    }),
}));

const listAccountsMock = vi.fn(async () => [] as unknown[]);
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListIdentityAccountsCommand: () => listAccountsMock(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { primeAccountCache, loadAccounts, subscribeAccountChanges } from "./identity-model";

describe("account cache live sync (identityaccounts:changed)", () => {
    beforeEach(() => {
        listAccountsMock.mockClear();
    });

    it("primeAccountCache installs exactly one identityaccounts:changed subscription, even when called twice", () => {
        primeAccountCache();
        primeAccountCache();
        const accountSubs = subscribeCalls.filter((s) => s.eventType === "identityaccounts:changed");
        expect(accountSubs.length).toBe(1);
    });

    it("the broadcast handler refreshes the cache and notifies cache listeners", async () => {
        primeAccountCache();
        const sub = subscribeCalls.find((s) => s.eventType === "identityaccounts:changed");
        expect(sub).toBeDefined();

        listAccountsMock.mockResolvedValueOnce([
            {
                id: "550e8400-e29b-41d4-a716-446655440000",
                name: "new-account",
                provider: "claude",
                kind: "oauth",
                display_name: "",
                secret_ref: { oauth_config_dir: { dir: "/tmp/x" } },
                context: {},
                status: "valid",
                created_at: 0,
                updated_at: 0,
            },
        ]);

        const seen: unknown[][] = [];
        const unsub = subscribeAccountChanges((accounts) => seen.push(accounts as unknown[]));

        listAccountsMock.mockClear();
        sub!.handler();
        // refreshAccountCache is async — give the microtask queue a tick.
        await vi.waitFor(() => expect(listAccountsMock).toHaveBeenCalledTimes(1));
        await vi.waitFor(() => expect(seen.length).toBeGreaterThan(0));
        expect(loadAccounts().some((a) => a.name === "new-account")).toBe(true);
        unsub();
    });

    it("a superseded refresh does not overwrite a newer one (latest-call-wins, codex P2 on #2474)", async () => {
        const { refreshAccountCache } = await import("./identity-model");
        const mkRow = (name: string) => ({
            id: "550e8400-e29b-41d4-a716-446655440001",
            name,
            provider: "claude",
            kind: "oauth",
            display_name: "",
            secret_ref: { oauth_config_dir: { dir: "/tmp/x" } },
            context: {},
            status: "valid",
            created_at: 0,
            updated_at: 0,
        });

        // First (older) request resolves LAST — after the second (newer)
        // request already landed. Without the seq guard, "older" would
        // overwrite "newer" in the cache.
        let releaseOlder!: (rows: unknown[]) => void;
        const olderGate = new Promise<unknown[]>((res) => (releaseOlder = res));
        listAccountsMock.mockImplementationOnce(() => olderGate);
        listAccountsMock.mockResolvedValueOnce([mkRow("newer")]);

        const older = refreshAccountCache();
        const newer = refreshAccountCache();
        await newer;
        expect(loadAccounts().some((a) => a.name === "newer")).toBe(true);

        releaseOlder([mkRow("older")]);
        const olderResult = await older;
        // Cache keeps the newer snapshot; the superseded call returns the
        // live (newer) cache rather than its own stale fetch.
        expect(loadAccounts().some((a) => a.name === "newer")).toBe(true);
        expect(loadAccounts().some((a) => a.name === "older")).toBe(false);
        expect(olderResult.some((a) => a.name === "newer")).toBe(true);
    });
});
