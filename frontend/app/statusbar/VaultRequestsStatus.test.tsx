// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

type Row = { id: string; block_id: string };
const signals = vi.hoisted(() => {
    // Solid is loaded here, not at the top: the mock factory runs before the imports.
    let uses: Row[] = [];
    let requests: Row[] = [];
    const subs = new Set<() => void>();
    return {
        get uses() {
            return uses;
        },
        get requests() {
            return requests;
        },
        set(u: Row[] | null, r: Row[] | null) {
            if (u) uses = u;
            if (r) requests = r;
            subs.forEach((f) => f());
        },
        subs,
    };
});
vi.mock("@/app/view/vault/vault-store", async () => {
    const { createSignal } = await import("solid-js");
    const [tick, setTick] = createSignal(0);
    signals.subs.add(() => setTick((n) => n + 1));
    return { vaultUses: () => (tick(), signals.uses), vaultRequests: () => (tick(), signals.requests) };
});
const setUses = (u: Row[]) => signals.set(u, null);
const setRequests = (r: Row[]) => signals.set(null, r);
const openAt = vi.hoisted(() => vi.fn());
vi.mock("@/app/view/vault/open-vault", () => ({ openVaultAt: openAt }));

import { VaultRequestsStatus, vaultRequestsTip } from "./VaultRequestsStatus";

afterEach(() => {
    cleanup();
    setUses([]);
    setRequests([]);
    openAt.mockClear();
});

describe("VaultRequestsStatus", () => {
    it("shows nothing while the Vault waits on no one", () => {
        const { container } = render(() => <VaultRequestsStatus />);
        expect(container.querySelector(".status-vault-requests")).toBeNull();
    });

    it("counts every use and request, whatever pane asked: one asked in a pane, one with no pane, one whose pane closed", () => {
        setUses([{ id: "u1", block_id: "pane-on-screen" }, { id: "u2", block_id: "" }]);
        setRequests([{ id: "r1", block_id: "closed-pane" }]);
        render(() => <VaultRequestsStatus />);
        expect(screen.getByText("3 Vault requests")).toBeTruthy();
    });

    it("leaves the count at once when a request is answered, and says so in the singular for one", () => {
        setRequests([{ id: "r1", block_id: "p" }, { id: "r2", block_id: "" }]);
        const { container } = render(() => <VaultRequestsStatus />);
        expect(screen.getByText("2 Vault requests")).toBeTruthy();
        setRequests([{ id: "r2", block_id: "" }]);
        expect(screen.getByText("1 Vault request")).toBeTruthy();
        setRequests([]);
        expect(container.querySelector(".status-vault-requests")).toBeNull();
    });

    it("opens the Vault at the first request, and cannot be read or clicked by a widget", () => {
        setUses([{ id: "u1", block_id: "p" }]);
        const { container } = render(() => <VaultRequestsStatus />);
        const item = container.querySelector(".status-vault-requests") as HTMLElement;
        expect(item.hasAttribute("data-operator-only")).toBe(true);
        fireEvent.click(item);
        expect(openAt).toHaveBeenCalledWith("u1");
    });

    it("says what it counts in the tooltip", () => {
        expect(vaultRequestsTip(1)).toContain("1 Vault request waits on you");
        expect(vaultRequestsTip(4)).toContain("4 Vault requests wait on you");
    });
});
