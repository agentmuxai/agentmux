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
    page: "",
    field: "Social Security Number",
    offer_always: true,
    reason: "the first time this entry is used on this site",
    agent_reason: "",
    needs_value: false,
    kind: "ssn",
    command: "",
    cwd: "",
    replaces: false,
    signin: false,
    account: false,
    requested_ms: 1,
    block_id: "pane-1",
    deadline_ms: 1,
    expires_ms: 1,
    goes_ahead: false,
    ...over,
});

let uses: UseRequest[] = [];
let entries: VaultEntryInfo[] = [];
const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "list") return { entries, problem: null, requests: [], uses, backend: "keychain" };
    return {};
});
vi.mock("@/util/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

import { VaultUseItem as UsePrompt } from "./vault-requests-list";
import { clipMiddle, clock, siteOf } from "./vault-use-question";
import { cardAnswerText } from "./vault-use-card";
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
        const dialog = await screen.findByRole("group");
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
            const dialog = await screen.findByRole("group");
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
        const dialog = await screen.findByRole("group");
        expect(dialog.textContent).toContain("Agent1 wants to run a command with your API key");
        expect(dialog.textContent).toContain("stripe balance retrieve");
        expect(dialog.textContent).toContain("STRIPE_API_KEY");
        expect(dialog.textContent).toContain("C:/work");
        expect(dialog.textContent).toContain("cleaned of it");
        expect(dialog.textContent).not.toContain("Into");
        fireEvent.click(within(dialog).getByRole("button", { name: "Always for Agent1" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "always" }]));
    });

    it("quotes what the agent says it wants the entry for, attributed to it, and says nothing when it gave none", async () => {
        uses = [use({ command: "stripe balance retrieve", cwd: "C:/work", origin: "", field: "STRIPE_API_KEY", agent_reason: "check the balance before the payout, task 27968" })];
        await refreshVault();
        const first = render(() => <UsePrompt use={uses[0]} close={() => {}} />);
        const says = (await screen.findByRole("group")).querySelector(".vault-use-says")!;
        expect(says.textContent).toBe("Agent1 says: check the balance before the payout, task 27968");
        first.unmount();

        uses = [use({ command: "stripe balance retrieve", cwd: "C:/work", origin: "", field: "STRIPE_API_KEY" })];
        await refreshVault();
        const second = render(() => <UsePrompt use={uses[0]} close={() => {}} />);
        expect((await screen.findByRole("group")).querySelector(".vault-use-says")).toBeNull();
        second.unmount();

        // A long reason is cut, so the command stays in view next to the buttons.
        const long = `check the nightly build output for task 27968 ${"and compare the bundle sizes ".repeat(10)}`.trim().slice(0, 300);
        uses = [use({ command: "stripe balance retrieve", cwd: "C:/work", origin: "", field: "STRIPE_API_KEY", agent_reason: long })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={() => {}} />);
        const shown = (await screen.findByRole("group")).querySelector(".vault-use-says q")!.textContent!;
        expect(Array.from(shown).length).toBe(140);
        expect(shown.endsWith("…")).toBe(true);
    });

    it("asks to copy a secret in from AWS: names the secret, key, region and Vault, offers no Always, and never goes ahead on silence", async () => {
        const importing = { service: "AWS Secrets Manager", secret: "staging/db-creds", key: "db.password", region: "eu-west-1", profile: "work-read", grant: false, vault: "Staging", replaces: false };
        uses = [use({ entry_id: "", entry_label: "Staging database", kind: "api-key", origin: "", field: "", what: "secret", offer_always: false, agent_reason: "copy the staging password, task 28071", import: importing })];
        await refreshVault();
        const first = render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        expect(dialog.textContent).toContain("Agent1 wants to copy a secret from AWS Secrets Manager into your Vault as Staging database");
        expect(dialog.textContent).toContain("staging/db-creds");
        expect(dialog.textContent).toContain("db.password");
        expect(dialog.textContent).toContain("eu-west-1");
        expect(dialog.textContent).toContain("Staging");
        expect(dialog.textContent).toContain("It is read with the AWS profile work-read on this computer.");
        expect(dialog.textContent).toContain("Agent1 says: copy the staging password, task 28071");
        expect(dialog.textContent).toContain("The agent never sees the value: AgentMux reads it.");
        expect(dialog.textContent).toContain("nothing is copied without a yes");
        expect(within(dialog).queryByRole("button", { name: /Always/ })).toBeNull();
        expect(within(dialog).queryByRole("timer")).toBeNull();
        fireEvent.click(within(dialog).getByRole("button", { name: "Allow once" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "once" }]));
        first.unmount();

        uses = [use({ entry_id: "api-key-stripe", entry_label: "Stripe", kind: "api-key", origin: "", field: "", what: "secret", offer_always: false, import: { ...importing, key: "", region: "", profile: "", replaces: true } })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const replacing = (await screen.findByRole("group")).textContent!;
        expect(replacing).toContain("Agent1 wants to replace the value of Stripe with a secret from AWS Secrets Manager");
        expect(replacing).toContain("The old value is kept for 7 days");
        expect(replacing).not.toContain("(key");
        expect(replacing).toContain("It is read with this computer's default AWS login.");
    });

    it("cuts a long secret id in the middle, so the name at its end stays readable, with the whole in the tooltip's place", async () => {
        const long = `team-a/secrets/${"team/".repeat(20)}staging-db-creds-AbCdEf`;
        const shown = clipMiddle(long, 90);
        expect(Array.from(shown).length).toBe(90);
        expect(shown.startsWith("team-a/")).toBe(true);
        expect(shown.endsWith("staging-db-creds-AbCdEf")).toBe(true);
        expect(clipMiddle("staging/db-creds", 90)).toBe("staging/db-creds");

        uses = [use({ entry_id: "", entry_label: "Staging database", kind: "api-key", origin: "", field: "", offer_always: false, import: { service: "AWS Secrets Manager", secret: long, key: "", region: "eu-west-1", profile: "", grant: false, vault: "Personal", replaces: false } })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const text = (await screen.findByRole("group")).textContent!;
        expect(text).toContain("staging-db-creds-AbCdEf");
        // The secret leads the card: it is read before the countdown line and the agent's words.
        expect(text.indexOf("The secret")).toBeLessThan(text.indexOf("Waiting for you"));
    });

    it("asks for an agent's sign-in by site and masked user, and offers Always for that agent on that site", async () => {
        uses = [
            use({
                signin: true,
                what: "sign-in",
                kind: "login",
                entry_label: "Example",
                origin: "https://accounts.example.com",
                field: "al***",
                reason: "Agent1 hasn't signed in with this before",
            }),
        ];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        expect(dialog.textContent).toContain("Agent1 wants to sign in to accounts.example.com as al***");
        expect(dialog.textContent).toContain("sign-in form on accounts.example.com");
        expect(dialog.textContent).toContain("never sees the user name or the password");
        expect(dialog.textContent).not.toContain("Into");
        fireEvent.click(within(dialog).getByRole("button", { name: "Always for Agent1 on accounts.example.com" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "always" }]));
    });

    it("asks for an agent's new account by site, says the Vault makes the password, and offers no Always", async () => {
        uses = [
            use({
                account: true,
                what: "new account",
                kind: "login",
                entry_id: "",
                entry_label: "a new account on https://shop.example.com",
                origin: "https://shop.example.com",
                field: "",
                offer_always: false,
                reason: "Agent1 hasn't made an account on this site with you before",
            }),
        ];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        expect(dialog.textContent).toContain("Agent1 wants to create an account on shop.example.com");
        expect(dialog.textContent).toContain("sign-up form on shop.example.com");
        expect(dialog.textContent).toContain("The agent never sees it");
        expect(dialog.textContent).not.toContain("Into");
        expect(within(dialog).queryByRole("button", { name: /^Always/ })).toBeNull();
    });

    it("offers Always for that agent on that site for a new account, and sends it", async () => {
        uses = [
            use({
                account: true,
                what: "new account",
                kind: "login",
                entry_id: "",
                entry_label: "a new account on https://shop.example.com",
                origin: "https://shop.example.com",
                field: "",
                offer_always: true,
            }),
        ];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        fireEvent.click(within(dialog).getByRole("button", { name: "Always for Agent1 on shop.example.com" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "always" }]));
    });

    it("cuts a long agent name and a long site short in the Always button, which is for the whole card to fit", async () => {
        const site = `accounts.${"very-long-subdomain.".repeat(5)}shop.example.com`;
        uses = [use({ account: true, what: "new account", kind: "login", entry_id: "", entry_label: `a new account on https://${site}`, origin: `https://${site}`, field: "", offer_always: true, agent: "Maricon-the-Second-Agent-With-A-Very-Long-Display-Name" })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        const always = within(dialog).getByRole("button", { name: /^Always for / });
        expect(always.textContent!.length).toBeLessThan(140);
        expect(always.textContent).toContain("…");
    });

    it("says a command's output will replace the entry's value, that the old one is kept, and offers no Always", async () => {
        uses = [use({ origin: "", field: "", what: "value", entry_label: "GitHub token", kind: "", command: "gh auth token", cwd: "C:/work", offer_always: false, replaces: true, reason: "what the command prints replaces the value" })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        expect(dialog.textContent).toContain("Agent1 wants to replace GitHub token with what a command prints");
        expect(dialog.textContent).toContain("gh auth token");
        expect(dialog.textContent).toContain("becomes the value of GitHub token");
        expect(dialog.textContent).toContain("kept for 7 days");
        expect(dialog.textContent).not.toContain("in its environment");
        expect(within(dialog).queryByRole("button", { name: /Always/ })).toBeNull();
        fireEvent.click(within(dialog).getByRole("button", { name: "Allow once" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "once" }]));
    });

    it("offers no Always when the field doesn't look like what the entry is", async () => {
        uses = [use({ offer_always: false, reason: "the field doesn't look like what this entry is" })];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        expect(within(dialog).queryByRole("button", { name: /Always/ })).toBeNull();
        expect(within(dialog).getByRole("button", { name: "Allow once" })).toBeInTheDocument();
        expect(dialog.textContent).toContain("doesn't look like what this entry is");
    });

    it("goes when the request was answered elsewhere, and shows an error if the answer fails", async () => {
        const close = vi.fn();
        render(() => <UsePrompt use={uses[0]} close={close} />);
        const dialog = await screen.findByRole("group");
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
        const dialog = await screen.findByRole("group");
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
        const dialog = await screen.findByRole("group");
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
        const dialog = await screen.findByRole("group");
        const field = dialog.querySelector('input[type="password"]') as HTMLInputElement;
        fireEvent.input(field, { target: { value: "987-65-4321" } });
        fireEvent.click(within(dialog).getByRole("button", { name: "Deny" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "deny" }]));
        expect(JSON.stringify(vault.mock.calls)).not.toContain("987-65");
    });

    it("asks for a whole card when the agent asks for a card, and sends it as one text, kept only if they say so", async () => {
        uses = [use({ needs_value: true, offer_always: false, kind: "card", what: "card", entry_label: "Alex's card", field: "the card fields: number, expiry, cvc", reason: "there is no card saved for this, so you type it here" })];
        await refreshVault();
        const close = vi.fn();
        render(() => <UsePrompt use={uses[0]} close={close} />);
        const dialog = await screen.findByRole("group");
        expect(dialog.textContent).toContain("Agent1 needs your card");
        expect(dialog.querySelector('input[type="password"]')).toBeNull();
        expect(within(dialog).queryByRole("button", { name: /Always/ })).toBeNull();
        for (const label of ["Card number", "Expiry", "Name on card", "Billing ZIP"]) expect(dialog.textContent).toContain(label);
        // Nothing to fill until the number is typed.
        expect(within(dialog).getByRole("button", { name: "Fill it in" })).toBeDisabled();
        const number = within(dialog).getByLabelText("Card number") as HTMLInputElement;
        fireEvent.input(number, { target: { value: "4242424242424242" } });
        expect(number.value).toBe("4242 4242 4242 4242");
        expect(dialog.textContent).toContain("Visa");
        fireEvent.input(within(dialog).getByLabelText(/Security code/), { target: { value: "123" } });
        fireEvent.input(within(dialog).getByLabelText("Name on card"), { target: { value: "Alex Doe" } });
        fireEvent.input(within(dialog).getByLabelText("Billing ZIP"), { target: { value: "94107" } });
        fireEvent.click(within(dialog).getByLabelText(/Save to the Vault as "Alex's card"/));
        fireEvent.click(within(dialog).getByRole("button", { name: "Fill it in" }));
        await waitFor(() => expect(calls("answer_use")).toHaveLength(1));
        const sent = calls("answer_use")[0] as { id: string; decision: string; value: string; save: boolean };
        expect(sent).toMatchObject({ id: "use-1", decision: "once", save: true });
        expect(JSON.parse(sent.value)).toEqual({ number: "4242 4242 4242 4242", exp: "", cvc: "123", name: "Alex Doe", zip: "94107" });
        await waitFor(() => expect(close).toHaveBeenCalled());
        // The question stays in the list here (nothing removed it), yet the boxes no longer hold what was typed.
        expect([number.value, (within(dialog).getByLabelText(/Security code/) as HTMLInputElement).value, (within(dialog).getByLabelText("Name on card") as HTMLInputElement).value, (within(dialog).getByLabelText("Billing ZIP") as HTMLInputElement).value]).toEqual(["", "", "", ""]);
        expect(document.body.innerHTML).not.toContain("4242 4242");
    });

    it("makes the card's text only once there is a number, and an expiry only when both its month and year are chosen", () => {
        const card = { number: "", month: "03", year: "2030", cvc: "123", name: "", zip: "" };
        expect(cardAnswerText(card)).toBe("");
        expect(JSON.parse(cardAnswerText({ ...card, number: "4242 4242 4242 4242" })).exp).toBe("03/2030");
        expect(JSON.parse(cardAnswerText({ ...card, number: "4242 4242 4242 4242", year: "" })).exp).toBe("");
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
                vault_id: "personal",
                fields: {},
                tags: [],
                parts_set: ["value"],
                grants: ["https://www.irs.gov", "https://ssa.gov"],
                tail: "6789",
                versions: 0,
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

    it("lists the agents that may sign in with it apart from the sites, and revokes one", async () => {
        entries = [{ ...entries[0], kind: "login", grants: ["signin:Agent1|personal|https://accounts.example.com"] }];
        await refreshVault();
        render(() => <VaultEntries section="personal" />);
        const row = (await screen.findByText("Alex")).closest("tr") as HTMLElement;
        expect(row.textContent).toContain("Agents may sign in with it");
        expect(row.textContent).not.toContain("Agents may fill it on");
        fireEvent.click(within(row).getByRole("button", { name: "Stop signing in with Alex without asking for Agent1 on accounts.example.com" }));
        await waitFor(() => expect(calls("revoke_grant")).toEqual([{ entry_id: "ssn-alex", origin: "signin:Agent1|personal|https://accounts.example.com" }]));
    });
});

describe("srv's deadline in the prompt", () => {
    beforeEach(() => vault.mockClear());
    afterEach(cleanup);

    it("counts down for an API key, which goes ahead on silence, and the person's activity holds it", async () => {
        const key = use({ kind: "api-key", entry_label: "GitHub token", goes_ahead: true, deadline_ms: Date.now() + 24_500 });
        uses = [key];
        await refreshVault();
        render(() => <UsePrompt use={key} close={vi.fn()} />);
        const timer = await screen.findByRole("timer");
        expect(timer.textContent).toMatch(/Goes ahead in 0:2[45] unless you deny/);
        fireEvent.pointerMove(timer);
        fireEvent.pointerMove(timer);
        await waitFor(() => expect(calls("hold_use")).toEqual([{ id: key.id }]));
    });

    it("holds it while the person moves between the answers too", async () => {
        const key = use({ id: "use-keys", kind: "api-key", goes_ahead: true, deadline_ms: Date.now() + 20_000 });
        uses = [key];
        await refreshVault();
        render(() => <UsePrompt use={key} close={vi.fn()} />);
        fireEvent.focusIn(await screen.findByRole("button", { name: "Allow once" }));
        await waitFor(() => expect(calls("hold_use")).toEqual([{ id: "use-keys" }]));
    });

    it("says an ID entry waits for a yes, with no countdown and no hold", async () => {
        uses = [use()];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        const dialog = await screen.findByRole("group");
        expect(dialog.textContent).toContain("ID and card details are never used without a yes");
        expect(screen.queryByRole("timer")).toBeNull();
        fireEvent.pointerMove(dialog.querySelector(".vault-use-deadline") as HTMLElement);
        expect(calls("hold_use")).toEqual([]);
    });

    it("formats the clock as minutes and seconds", () => {
        expect([clock(30), clock(5), clock(0), clock(90)]).toEqual(["0:30", "0:05", "0:00", "1:30"]);
    });

    describe("a whole card the person picks", () => {
        const cards = [
            { id: "card-a", label: "Probe Visa", detail: "Visa •••• 4242, expires 03/27", expired: false, default: true },
            { id: "card-old", label: "Old Mastercard", detail: "Mastercard •••• 4444, expired 03/2020", expired: true, default: false },
        ];
        const cardUse = (over: Partial<UseRequest> = {}) =>
            use({ kind: "card", entry_id: "", entry_label: "a card", what: "card", field: "the card fields: number, expiry", reason: "no card was named, and there is no default card", choices: cards, ...over });

        it("lists the cards with brand, last four and expiry, takes no yes until one is chosen, and answers with the chosen one", async () => {
            uses = [cardUse()];
            await refreshVault();
            render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
            const dialog = await screen.findByRole("group");
            const select = within(dialog).getByRole("combobox", { name: "Card" }) as HTMLSelectElement;
            expect(Array.from(select.options).map((o) => o.textContent)).toEqual([
                "Choose a card…",
                "Probe Visa: Visa •••• 4242, expires 03/27 (default)",
                "Old Mastercard: Mastercard •••• 4444, expired 03/2020",
            ]);
            expect(dialog.textContent).toContain("no card was named, and there is no default card");
            // Nothing chosen: no yes (the Always button too), but a no.
            expect(within(dialog).getByRole("button", { name: "Allow once" })).toBeDisabled();
            expect(within(dialog).getByRole("button", { name: "Always on www.irs.gov" })).toBeDisabled();
            expect(within(dialog).getByRole("button", { name: "Deny" })).toBeEnabled();

            fireEvent.change(select, { target: { value: "card-a" } });
            fireEvent.click(within(dialog).getByRole("button", { name: "Allow once" }));
            await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "once", choice: "card-a" }]));
        });

        it("starts on the card the agent named, and offers no Always for an expired one", async () => {
            uses = [cardUse({ entry_id: "card-a", entry_label: "Probe Visa", what: "Visa •••• 4242, expires 03/27" })];
            await refreshVault();
            render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
            const dialog = await screen.findByRole("group");
            const select = within(dialog).getByRole("combobox", { name: "Card" }) as HTMLSelectElement;
            expect(select.value).toBe("card-a");
            expect(within(dialog).getByRole("button", { name: "Allow once" })).toBeEnabled();
            expect(within(dialog).getByRole("button", { name: "Always on www.irs.gov" })).toBeEnabled();

            fireEvent.change(select, { target: { value: "card-old" } });
            expect(within(dialog).getByRole("button", { name: "Allow once" })).toBeEnabled();
            expect(within(dialog).queryByRole("button", { name: /^Always/ })).toBeNull();

            fireEvent.click(within(dialog).getByRole("button", { name: "Allow once" }));
            await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "once", choice: "card-old" }]));
        });

        it("names the payment frame and the page it is on, and the Always is for that pair", async () => {
            uses = [cardUse({ entry_id: "card-a", entry_label: "Probe Visa", what: "Visa •••• 4242, expires 03/27", origin: "https://js.stripe.com", page: "https://shop.example" })];
            await refreshVault();
            render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
            const dialog = await screen.findByRole("group");
            expect(dialog.textContent).toContain("in a frame from js.stripe.com");
            expect(dialog.textContent).toContain("on shop.example");
            expect(within(dialog).getByRole("button", { name: "Always on shop.example (js.stripe.com fields)" })).toBeEnabled();
            fireEvent.click(within(dialog).getByRole("button", { name: "Always on shop.example (js.stripe.com fields)" }));
            await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "always", choice: "card-a" }]));
        });

        it("sends no card with a no, and no picker for a use that is not a whole card", async () => {
            uses = [cardUse()];
            await refreshVault();
            render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
            fireEvent.click(within(await screen.findByRole("group")).getByRole("button", { name: "Deny" }));
            await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "deny" }]));
            cleanup();
            uses = [use()];
            await refreshVault();
            render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
            expect(within(await screen.findByRole("group")).queryByRole("combobox")).toBeNull();
        });
    });
});
