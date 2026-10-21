// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An agent's use of an entry: the prompt that names the site and the field, its
 *  three answers, and revoking a grant
 *  (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §6.4). */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

const use = (over: Partial<UseRequest> = {}): UseRequest => ({
    id: "use-1",
    agent: "Agent1",
    entry_id: "ssn-alex",
    entry_label: "Alex",
    what: "Social Security number",
    origin: "https://www.irs.gov",
    field: "Social Security Number",
    offer_always: true,
    reason: "the first time this entry is used on this site",
    needs_value: false,
    kind: "ssn",
    command: "",
    cwd: "",
    requested_ms: 1,
    ...over,
});

let uses: UseRequest[] = [];
let entries: VaultEntryInfo[] = [];
const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "list") return { entries, problem: null, requests: [], uses, backend: "keychain" };
    return {};
});
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

import { siteOf, UsePrompt } from "./vault-use-prompts";
import { VaultEntries } from "./vault-entries";
import { refreshVault } from "./vault-store";

const calls = (op: string) => vault.mock.calls.filter((c) => c[0] === op).map((c) => c[1] ?? {});

describe("the prompt for an agent's use of an entry", () => {
    beforeEach(async () => {
        vault.mockClear();
        uses = [use()];
        entries = [];
        await refreshVault();
    });
    afterEach(cleanup);

    it("names the agent, the entry, the site, the field and why, and says the agent never sees the value", async () => {
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("dialog");
        expect(dialog.textContent).toContain("Agent1 wants to fill your Social Security number");
        expect(dialog.textContent).toContain("Social Security Number");
        expect(dialog.textContent).toContain("www.irs.gov");
        expect(dialog.textContent).toContain("https://www.irs.gov");
        expect(dialog.textContent).toContain("Alex");
        expect(dialog.textContent).toContain("the first time this entry is used on this site");
        expect(dialog.textContent).toContain("never sees the value");
    });

    it("answers once, always or deny through the host, and closes", async () => {
        for (const [label, decision] of [
            ["Allow once", "once"],
            ["Always on www.irs.gov", "always"],
            ["Deny", "deny"],
        ] as const) {
            vault.mockClear();
            const close = vi.fn();
            const { unmount } = render(() => <UsePrompt use={uses[0]} close={close} />);
            const dialog = await screen.findByRole("dialog");
            fireEvent.click(within(dialog).getByRole("button", { name: label }));
            await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision }]));
            await waitFor(() => expect(close).toHaveBeenCalled());
            unmount();
        }
    });

    it("shows the command and where it runs, and offers Always for the agent, when the entry goes to a command", async () => {
        uses = [use({ origin: "", field: "STRIPE_API_KEY", what: "API key", entry_label: "Stripe", command: "stripe balance retrieve", cwd: "C:/work", reason: "it is the first time Agent1 runs a command with this entry" })];
        await refreshVault();
        const close = vi.fn();
        render(() => <UsePrompt use={uses[0]} close={close} />);
        const dialog = await screen.findByRole("dialog");
        expect(dialog.textContent).toContain("Agent1 wants to run a command with your API key");
        expect(dialog.textContent).toContain("stripe balance retrieve");
        expect(dialog.textContent).toContain("STRIPE_API_KEY");
        expect(dialog.textContent).toContain("C:/work");
        expect(dialog.textContent).toContain("cleaned of it");
        expect(dialog.textContent).not.toContain("Into");
        fireEvent.click(within(dialog).getByRole("button", { name: "Always for Agent1" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "always" }]));
    });

    it("offers no Always when the field doesn't look like what the entry is", async () => {
        uses = [use({ offer_always: false, reason: "the field doesn't look like what this entry is" })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("dialog");
        expect(within(dialog).queryByRole("button", { name: /Always/ })).toBeNull();
        expect(within(dialog).getByRole("button", { name: "Allow once" })).toBeInTheDocument();
        expect(dialog.textContent).toContain("doesn't look like what this entry is");
    });

    it("goes when the request was answered elsewhere, and shows an error if the answer fails", async () => {
        const close = vi.fn();
        render(() => <UsePrompt use={uses[0]} close={close} />);
        const dialog = await screen.findByRole("dialog");
        vault.mockRejectedValueOnce(new Error("couldn't reach AgentMux's host"));
        fireEvent.click(within(dialog).getByRole("button", { name: "Allow once" }));
        expect((await screen.findByRole("alert")).textContent).toContain("couldn't reach");
        expect(close).not.toHaveBeenCalled();
        uses = [];
        await refreshVault();
        await waitFor(() => expect(close).toHaveBeenCalled());
    });

    it("closes when the answer says nothing is waiting any more, instead of staying stuck", async () => {
        const close = vi.fn();
        render(() => <UsePrompt use={uses[0]} close={close} />);
        const dialog = await screen.findByRole("dialog");
        // The agent gave up and its prompt wasn't taken down: the answer is refused.
        vault.mockRejectedValueOnce(new Error("that request is no longer waiting"));
        fireEvent.click(within(dialog).getByRole("button", { name: "Deny" }));
        await waitFor(() => expect(close).toHaveBeenCalled());
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("asks the person to type a value no entry holds yet, and keeps it only if they say so", async () => {
        uses = [use({ needs_value: true, offer_always: false, entry_label: "Sam's SSN", reason: "there is no Vault entry for this yet, so you type it here" })];
        await refreshVault();
        const close = vi.fn();
        render(() => <UsePrompt use={uses[0]} close={close} />);
        const dialog = await screen.findByRole("dialog");
        expect(dialog.textContent).toContain("Agent1 needs your Social Security number");
        expect(dialog.textContent).toContain("there is no Vault entry for this yet");
        expect(within(dialog).queryByRole("button", { name: /Always/ })).toBeNull();
        // Nothing to fill until something is typed.
        expect(within(dialog).getByRole("button", { name: "Fill it in" })).toBeDisabled();
        const field = dialog.querySelector('input[type="password"]') as HTMLInputElement;
        fireEvent.input(field, { target: { value: "987-65-4321" } });
        fireEvent.click(within(dialog).getByLabelText(/Save to the Vault as "Sam's SSN"/));
        fireEvent.click(within(dialog).getByRole("button", { name: "Fill it in" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "once", value: "987-65-4321", save: true }]));
        await waitFor(() => expect(close).toHaveBeenCalled());
        expect(document.body.innerHTML).not.toContain("987-65-4321");
    });

    it("sends a typed value as not kept by default, and none when denied", async () => {
        uses = [use({ needs_value: true, offer_always: false })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("dialog");
        const field = dialog.querySelector('input[type="password"]') as HTMLInputElement;
        fireEvent.input(field, { target: { value: "987-65-4321" } });
        fireEvent.click(within(dialog).getByRole("button", { name: "Deny" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "deny" }]));
        expect(JSON.stringify(vault.mock.calls)).not.toContain("987-65");
    });

    it("shows a site by its host", () => {
        expect(siteOf("https://www.irs.gov")).toBe("www.irs.gov");
        expect(siteOf("http://localhost:3000")).toBe("localhost:3000");
        expect(siteOf("not a url")).toBe("not a url");
    });
});

describe("the sites an entry may be filled into", () => {
    beforeEach(async () => {
        vault.mockClear();
        uses = [];
        entries = [
            {
                id: "ssn-alex",
                kind: "ssn",
                section: "personal",
                label: "Alex",
                fields: {},
                tags: [],
                parts_set: ["value"],
                grants: ["https://www.irs.gov", "https://ssa.gov"],
                tail: "6789",
                created_at: 1,
                updated_at: 1,
                last_used_at: 0,
            },
        ];
        await refreshVault();
    });
    afterEach(cleanup);

    it("lists each site with a way to revoke it", async () => {
        render(() => <VaultEntries section="personal" />);
        const row = (await screen.findByText("Alex")).closest("tr") as HTMLElement;
        expect(row.textContent).toContain("www.irs.gov");
        expect(row.textContent).toContain("ssa.gov");
        fireEvent.click(within(row).getByRole("button", { name: "Stop filling Alex on www.irs.gov without asking" }));
        await waitFor(() => expect(calls("revoke_grant")).toEqual([{ entry_id: "ssn-alex", origin: "https://www.irs.gov" }]));
    });

    it("lists the agents that may run commands with it apart from the sites, and revokes one", async () => {
        entries = [{ ...entries[0], grants: ["https://www.irs.gov", "command:Agent1:value"] }];
        await refreshVault();
        render(() => <VaultEntries section="personal" />);
        const row = (await screen.findByText("Alex")).closest("tr") as HTMLElement;
        expect(row.textContent).toContain("Agents may run commands with it");
        expect(within(row).queryByRole("button", { name: /Stop filling Alex on command/ })).toBeNull();
        fireEvent.click(within(row).getByRole("button", { name: "Stop Agent1 (value) running commands with Alex without asking" }));
        await waitFor(() => expect(calls("revoke_grant")).toEqual([{ entry_id: "ssn-alex", origin: "command:Agent1:value" }]));
    });
});
