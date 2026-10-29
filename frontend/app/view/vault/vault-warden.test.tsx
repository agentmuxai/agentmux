// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Vault's oversight half in Warden's Audit section: what agents did with
 *  secrets, where each may use one without asking, and a way to stop it
 *  (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.6). */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VaultActivity } from "@/types/rpc/VaultActivity";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
const openOrFocus = vi.fn(async () => {});
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: (...a: unknown[]) => openOrFocus(...(a as [])) }));

let entries: VaultEntryInfo[] = [];
let activity: VaultActivity[] = [];
const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "grants") return { granted: entries.filter((e) => e.grants.length > 0).map((e) => ({ id: e.id, label: e.label, kind: e.kind, grants: e.grants })) };
    if (op === "activity") return { activity };
    return {};
});
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

import { WardenSecrets } from "./vault-warden";

const entry = (over: Partial<VaultEntryInfo> = {}): VaultEntryInfo => ({
    id: "api-key-stripe",
    kind: "api-key",
    section: "api-keys",
    label: "Stripe",
    fields: {},
    tags: [],
    parts_set: ["value"],
    grants: ["https://dashboard.stripe.com", "command:Agent1:value"],
    tail: "1234",
    created_at: 1,
    updated_at: 1,
    last_used_at: 0,
    ...over,
});

const row = (over: Partial<VaultActivity>): VaultActivity => ({ at: Date.now(), entry_id: "api-key-stripe", entry_label: "Stripe", actor: "Agent1", action: "ran a command", target: "stripe balance retrieve", outcome: "done", ...over });

beforeEach(() => {
    vault.mockClear();
    openOrFocus.mockClear();
    entries = [entry(), entry({ id: "ssn-alex", label: "Alex", grants: [] })];
    activity = [
        row({}),
        row({ actor: "you", action: "revealed", target: "value" }),
        row({ actor: "Agent2", action: "filled a web form", target: "https://irs.gov (SSN)", outcome: "denied" }),
        row({ actor: "Agent3", action: "requested an entry", target: "Stripe test key", outcome: "pending" }),
        row({ actor: "Agent4", action: "generated", entry_label: "Webhook secret", target: "sign test webhooks" }),
    ];
});
afterEach(cleanup);

describe("Secrets in Warden's Audit section", () => {
    it("lists what agents did, not the person's own doings, with the outcome", async () => {
        render(() => <WardenSecrets />);
        const table = await screen.findByRole("table", { name: "Agents' use of secrets" });
        expect(table.textContent).toContain("Agent1");
        expect(table.textContent).toContain("stripe balance retrieve");
        expect(table.textContent).toContain("Agent2");
        expect(table.textContent).toContain("denied");
        expect(table.textContent).not.toContain("revealed");
        // A request for an entry isn't a use of one; an entry an agent had the Vault make is listed.
        expect(table.textContent).not.toContain("Agent3");
        expect(table.textContent).toContain("Agent4");
        expect(table.textContent).toContain("sign test webhooks");
    });

    it("loads nothing from the Vault until it is shown", async () => {
        const [shown, setShown] = createSignal(false);
        render(() => <WardenSecrets visible={shown()} />);
        await new Promise((r) => setTimeout(r, 20));
        expect(vault).not.toHaveBeenCalled();
        setShown(true);
        await waitFor(() => expect(vault.mock.calls.some((c) => c[0] === "grants")).toBe(true));
        // Never the decrypting list, which can make the OS ask for the Vault key.
        expect(vault.mock.calls.some((c) => c[0] === "list")).toBe(false);
        expect(await screen.findByRole("table", { name: "Agents' use of secrets" })).toBeInTheDocument();
    });

    it("lists the standing permissions, sites and commands apart, and stops one", async () => {
        render(() => <WardenSecrets />);
        const table = await screen.findByRole("table", { name: "Standing permissions" });
        expect(table.textContent).toContain("dashboard.stripe.com");
        expect(table.textContent).toContain("runs commands: Agent1 (value)");
        expect(table.textContent).not.toContain("Alex");
        fireEvent.click(within(table).getByRole("button", { name: "Stop runs commands: Agent1 (value) for Stripe" }));
        await waitFor(() => expect(vault.mock.calls.filter((c) => c[0] === "revoke_grant").map((c) => c[1])).toEqual([{ entry_id: "api-key-stripe", origin: "command:Agent1:value" }]));
    });

    it("links to the Vault's activity", async () => {
        render(() => <WardenSecrets />);
        fireEvent.click(await screen.findByRole("button", { name: "Open the Vault" }));
        await waitFor(() => expect(openOrFocus).toHaveBeenCalled());
        const args = openOrFocus.mock.calls[0] as unknown as [string, { meta: Record<string, string> }, Record<string, string>];
        // The Vault is Warden's Vault section: the same Warden switches there.
        expect(args[0]).toBe("warden");
        expect(args[2]).toEqual({ "warden:section": "vault", "vault:section": "activity" });
    });

    it("says so when no agent has used a secret", async () => {
        activity = [row({ actor: "you" })];
        entries = [];
        render(() => <WardenSecrets />);
        expect(await screen.findByText("No agent has used a secret yet.")).toBeInTheDocument();
        expect(screen.queryByRole("table", { name: "Standing permissions" })).toBeNull();
    });
});
