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
const defaultVault = async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "grants") return { granted: entries.filter((e) => e.grants.length > 0).map((e) => ({ id: e.id, label: e.label, kind: e.kind, grants: e.grants })) };
    if (op === "activity") return { activity };
    return {};
};
const vault = vi.fn(defaultVault);
vi.mock("@/util/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

import { VaultSecrets } from "./vault-secrets";

const entry = (over: Partial<VaultEntryInfo> = {}): VaultEntryInfo => ({
    id: "api-key-stripe",
    kind: "api-key",
    section: "api-keys",
    label: "Stripe",
    vault_id: "personal",
    fields: {},
    tags: [],
    parts_set: ["value"],
    grants: ["https://dashboard.stripe.com", "command:Agent1:value"],
    tail: "1234",
    versions: 0,
    created_at: 1,
    updated_at: 1,
    last_used_at: 0,
    ...over,
});

const row = (over: Partial<VaultActivity>): VaultActivity => ({ at: Date.now(), entry_id: "api-key-stripe", entry_label: "Stripe", actor: "Agent1", action: "ran a command", target: "stripe balance retrieve", outcome: "done", seq: 1, version: 0, actor_kind: "agent", host: "", reason: "", refs: [], is_use: true, ...over });

beforeEach(() => {
    vault.mockReset();
    vault.mockImplementation(defaultVault);
    openOrFocus.mockClear();
    entries = [entry(), entry({ id: "ssn-alex", label: "Alex", grants: [] })];
    activity = [
        row({}),
        row({ actor: "you", actor_kind: "person", action: "revealed", target: "value" }),
        row({ actor: "Agent2", action: "filled a web form", target: "https://irs.gov (SSN)", outcome: "denied" }),
        row({ actor: "Agent3", action: "requested an entry", target: "Stripe test key", outcome: "pending" }),
        row({ actor: "Agent4", action: "generated", entry_label: "Webhook secret", target: "sign test webhooks" }),
    ];
});

/** `n` lines by agents, newest first. */
const agentLines = (n: number, from = 1): VaultActivity[] => Array.from({ length: n }, (_, i) => row({ actor: `Bot${from + i}`, seq: from + i, target: `job ${from + i}` }));
afterEach(cleanup);

describe("Secrets in the Vault's Audit section", () => {
    it("lists what agents did, not the person's own doings, with the outcome", async () => {
        render(() => <VaultSecrets />);
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

    it("lists every kind of thing an agent did with an entry, and none of the app's or the person's", async () => {
        activity = [
            row({ actor: "Agent5", action: "signed in", target: "https://dash.example (alex)" }),
            row({ actor: "Agent6", action: "rotated", target: "new value" }),
            row({ actor: "Agent7", action: "captured", target: "aws iam create-access-key" }),
            row({ actor: "Agent8", action: "imported a batch", target: "3 keys" }),
            row({ actor: "Agent9", action: "shared", target: "profile of Agent9" }),
            row({ actor: "Agent10", action: "asked to be given it", target: "Stripe" }),
            row({ actor: "AgentMux", actor_kind: "srv", action: "reminded", target: "expires in 7 days" }),
            row({ actor: "you", actor_kind: "person", action: "copied", target: "value" }),
            // A line made before the Vault kept who did it: known by its old kinds of use only.
            row({ actor: "Old1", actor_kind: "", action: "ran a command", target: "old command" }),
            row({ actor: "Old2", actor_kind: "", action: "rotated", target: "old rotation" }),
        ];
        render(() => <VaultSecrets />);
        const text = (await screen.findByRole("table", { name: "Agents' use of secrets" })).textContent ?? "";
        for (const who of ["Agent5", "Agent6", "Agent7", "Agent8", "Agent9", "Old1"]) expect(text).toContain(who);
        for (const not of ["Agent10", "AgentMux", "copied", "Old2"]) expect(text).not.toContain(not);
    });

    it("shows fifty at a time, says how much it covers, and reads further back when asked", async () => {
        const all = agentLines(120);
        vault.mockImplementation(async (op: string, args?: Record<string, unknown>) => {
            if (op === "grants") return { granted: [] };
            if (op === "activity") return { activity: all.slice(0, Math.min(Number(args?.limit), all.length)) };
            return {};
        });
        render(() => <VaultSecrets />);
        const table = await screen.findByRole("table", { name: "Agents' use of secrets" });
        expect(within(table).getAllByRole("row")).toHaveLength(51);
        // Fewer lines than the read asked for: nothing older exists, so no claim that there is.
        // ...and so no "newest N lines": the whole activity was read.
        expect(screen.getByText(/Showing 50 of the 120 things agents did with secrets in the Vault's activity\./)).toBeInTheDocument();
        expect(screen.queryByText(/newest/)).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Show more" }));
        await waitFor(() => expect(within(table).getAllByRole("row")).toHaveLength(101));
        fireEvent.click(screen.getByRole("button", { name: "Show more" }));
        await waitFor(() => expect(within(table).getAllByRole("row")).toHaveLength(121));
        expect(screen.queryByRole("button", { name: "Show more" })).toBeNull();
        expect(screen.queryByText(/Older lines are in the Vault's Activity/)).toBeNull();
        expect(vault.mock.calls.filter((c) => c[0] === "activity").map((c) => (c[1] as { limit: number }).limit)).toEqual([500]);
    });

    it("when the read came back full, reads further back, to the host's limit of 1000, then says older lines are not read", async () => {
        const all = agentLines(20000);
        vault.mockImplementation(async (op: string, args?: Record<string, unknown>) => {
            if (op === "grants") return { granted: [] };
            // The host answers at most 1000 lines to one read.
            if (op === "activity") return { activity: all.slice(0, Math.min(Number(args?.limit), 1000)) };
            return {};
        });
        render(() => <VaultSecrets />);
        await screen.findByRole("table", { name: "Agents' use of secrets" });
        const limits = () => vault.mock.calls.filter((c) => c[0] === "activity").map((c) => (c[1] as { limit: number }).limit);
        // 500 lines read, 500 are agents': show 50 at a time until all of them are on screen, then read further.
        for (let i = 0; i < 9; i++) fireEvent.click(screen.getByRole("button", { name: "Show more" }));
        await waitFor(() => expect(screen.getByText(/Showing 500 of the 500/)).toBeInTheDocument());
        fireEvent.click(screen.getByRole("button", { name: "Show more" }));
        await waitFor(() => expect(limits()).toEqual([500, 1000]));
        // The count may carry the language's thousands separator.
        await waitFor(() => expect(screen.getByText(/Showing 550 of the 1\D?000 things/)).toBeInTheDocument());
        // The rest of the 1000 on screen: nothing more to read, and it says older lines exist.
        for (let i = 0; i < 9; i++) fireEvent.click(screen.getByRole("button", { name: "Show more" }));
        await waitFor(() => expect(screen.getByText(/Showing 1\D?000 of the 1\D?000 things/)).toBeInTheDocument());
        expect(screen.getByText(/Older lines exist; this list reads no further back/)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Show more" })).toBeNull();
        expect(limits()).toEqual([500, 1000]);
    });

    it("with no agent doing among a full read, says how far back it looked and reads further, never 'none yet'", async () => {
        // 500 lines of the person's own doings, then agents' doings further back (the host answers at most 1000).
        const mine = Array.from({ length: 700 }, (_, i) => row({ actor: "you", actor_kind: "person", action: "copied", seq: i + 1 }));
        const all = [...mine, ...agentLines(30, 701)];
        vault.mockImplementation(async (op: string, args?: Record<string, unknown>) => {
            if (op === "grants") return { granted: [] };
            if (op === "activity") return { activity: all.slice(0, Math.min(Number(args?.limit), 1000)) };
            return {};
        });
        render(() => <VaultSecrets />);
        expect(await screen.findByText(/No agent has used a secret in the newest 500 lines of the Vault's activity/)).toBeInTheDocument();
        expect(screen.queryByText("No agent has used a secret yet.")).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Show more" }));
        const table = await screen.findByRole("table", { name: "Agents' use of secrets" });
        expect(within(table).getAllByRole("row")).toHaveLength(31);
        // 730 lines came back for a ask of 1000: the whole activity was read.
        expect(screen.getByText(/Showing 30 of the 30 things agents did with secrets in the Vault's activity\./)).toBeInTheDocument();
    });

    it("with no agent doing among a full read at the host's limit, says older lines exist", async () => {
        const all = Array.from({ length: 1000 }, (_, i) => row({ actor: "you", actor_kind: "person", action: "copied", seq: i + 1 }));
        vault.mockImplementation(async (op: string, args?: Record<string, unknown>) => {
            if (op === "grants") return { granted: [] };
            if (op === "activity") return { activity: all.slice(0, Math.min(Number(args?.limit), 1000)) };
            return {};
        });
        render(() => <VaultSecrets />);
        fireEvent.click(await screen.findByRole("button", { name: "Show more" }));
        expect(await screen.findByText(/in the newest 1\D?000 lines of the Vault's activity\./)).toBeInTheDocument();
        expect(screen.getByText(/Older lines exist; this list reads no further back/)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Show more" })).toBeNull();
    });

    it("loads nothing from the Vault until it is shown", async () => {
        const [shown, setShown] = createSignal(false);
        render(() => <VaultSecrets visible={shown()} />);
        await new Promise((r) => setTimeout(r, 20));
        expect(vault).not.toHaveBeenCalled();
        setShown(true);
        await waitFor(() => expect(vault.mock.calls.some((c) => c[0] === "grants")).toBe(true));
        // Never the decrypting list, which can make the OS ask for the Vault key.
        expect(vault.mock.calls.some((c) => c[0] === "list")).toBe(false);
        expect(await screen.findByRole("table", { name: "Agents' use of secrets" })).toBeInTheDocument();
    });

    it("lists the standing permissions, sites and commands apart, and stops one", async () => {
        render(() => <VaultSecrets />);
        const table = await screen.findByRole("table", { name: "Standing permissions" });
        expect(table.textContent).toContain("dashboard.stripe.com");
        expect(table.textContent).toContain("runs commands: Agent1 (value)");
        expect(table.textContent).not.toContain("Alex");
        fireEvent.click(within(table).getByRole("button", { name: "Stop runs commands: Agent1 (value) for Stripe" }));
        await waitFor(() => expect(vault.mock.calls.filter((c) => c[0] === "revoke_grant").map((c) => c[1])).toEqual([{ entry_id: "api-key-stripe", origin: "command:Agent1:value" }]));
    });

    it("links to the Vault's activity", async () => {
        render(() => <VaultSecrets />);
        fireEvent.click(await screen.findByRole("button", { name: "Open the Vault" }));
        await waitFor(() => expect(openOrFocus).toHaveBeenCalled());
        const args = openOrFocus.mock.calls[0] as unknown as [string, { meta: Record<string, string> }, Record<string, string>];
        expect(args[0]).toBe("vault");
        expect(args[2]).toEqual({ "vault:section": "activity" });
    });

    it("says so when no agent has used a secret", async () => {
        activity = [row({ actor: "you", actor_kind: "person" })];
        entries = [];
        render(() => <VaultSecrets />);
        expect(await screen.findByText("No agent has used a secret yet.")).toBeInTheDocument();
        expect(screen.queryByRole("table", { name: "Standing permissions" })).toBeNull();
    });
});
