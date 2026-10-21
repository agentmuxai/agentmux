// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Vault's list opens the Vault key and decrypts every entry, which can make the OS ask for it, so
 *  it is read only while a Vault view is on screen (a hidden Warden section, a closed pane, never). */

import { createRoot } from "solid-js";
import { beforeEach, describe, expect, it, vi } from "vitest";

let handler: ((data: unknown) => void) | null = null;
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_event: string, f: (data: unknown) => void) => {
        handler = f;
        return () => {};
    },
}));
const vault = vi.fn(async (op: string): Promise<unknown> => {
    if (op === "list") return { entries: [], problem: null, requests: [], uses: [], backend: "keychain" };
    return {};
});
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

import { followVault, holdVaultShown } from "./vault-store";

const lists = () => vault.mock.calls.filter((c) => c[0] === "list").length;
const event = () => handler?.({ requests: [], uses: [], changed_ms: Date.now() });
const settle = () => new Promise((r) => setTimeout(r, 10));

describe("when the Vault's list is read", () => {
    beforeEach(() => vault.mockClear());

    it("reads it only while a Vault view is on screen, and again when one next appears", async () => {
        followVault();
        // Never shown: an agent's request or use reads nothing, and decrypts nothing.
        event();
        await settle();
        expect(lists()).toBe(0);

        // Shown: read at once, then again on every change.
        const release = createRoot((dispose) => {
            const r = holdVaultShown();
            return () => {
                r();
                dispose();
            };
        });
        await settle();
        expect(lists()).toBe(1);
        event();
        await settle();
        expect(lists()).toBe(2);

        // Hidden (another Warden section, or closed): changes are not read.
        release();
        event();
        event();
        await settle();
        expect(lists()).toBe(2);

        // Shown again: read once, so it is current, then kept current.
        const again = holdVaultShown();
        await settle();
        expect(lists()).toBe(3);
        // A second view on screen at once doesn't read it again.
        const second = holdVaultShown();
        await settle();
        expect(lists()).toBe(3);
        second();
        again();
    });

    it("releases once however often it is called", async () => {
        const a = holdVaultShown();
        const b = holdVaultShown();
        a();
        a();
        event();
        await settle();
        const before = lists();
        event();
        await settle();
        expect(lists()).toBe(before + 1); // b still holds it
        b();
        event();
        await settle();
        expect(lists()).toBe(before + 1);
    });
});
