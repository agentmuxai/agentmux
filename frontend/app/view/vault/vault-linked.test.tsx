// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A linked entry in the pane: its badges and what it says of its key, that it offers no value to show, what a check
 *  of its place found, the form's inputs for one, and the questions that make or change one. Every place named here
 *  is invented. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

let uses: UseRequest[] = [];
let entries: VaultEntryInfo[] = [];
const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "list") return { entries, problem: null, requests: [], uses, backend: "keychain" };
    if (op === "check_source") return { checked: { id: "api-key-invoice", label: "Invoice provider key", outcome: "present", source: "aws-secret:prod/invoice-sync#api.token", checked_at: 1 } };
    return {};
});
vi.mock("@/util/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));
const copied = vi.fn(async (_text: string) => {});
vi.mock("@/util/clipboard", () => ({ writeText: (text: string) => copied(text) }));

import { VaultEntries } from "./vault-entries";
import { VaultEntryForm } from "./vault-entry-form";
import { isLinked, linkFields, uncheckedDays } from "./vault-linked";
import { VaultUseItem as UsePrompt } from "./vault-requests-list";
import { sourceCheck } from "./vault-source";
import { refreshVault } from "./vault-store";

const PLACE = { source_kind: "aws-secret", source_location: "prod/invoice-sync", source_key: "api.token", source_region: "eu-west-1" };
const SAID = { readers: "invoice-sync (lambda): at run time\nrelease (workflow): on a tag", rotated: "2026-10-01", copies: "github-secret: example-org/example-app#EXAMPLE_TOKEN" };
const DAY = 24 * 60 * 60 * 1000;

const entry = (over: Partial<VaultEntryInfo> = {}): VaultEntryInfo => ({
    id: "api-key-invoice",
    kind: "api-key",
    section: "api-keys",
    label: "Invoice provider key",
    vault_id: "personal",
    fields: { link: "only", ...PLACE, ...SAID },
    tags: [],
    parts_set: [],
    grants: [],
    tail: "",
    versions: 0,
    created_at: 1,
    updated_at: 1,
    last_used_at: 0,
    ...over,
});

const use = (over: Partial<UseRequest>): UseRequest =>
    ({
        id: "use-1",
        agent: "Agent1",
        entry_id: "",
        entry_label: "Invoice provider key",
        what: "machine copy",
        origin: "",
        page: "",
        field: "",
        offer_always: false,
        reason: "",
        agent_reason: "record the keys the invoice service reads for task 31007",
        needs_value: false,
        kind: "api-key",
        command: "",
        cwd: "",
        replaces: false,
        signin: false,
        account: false,
        change: false,
        requested_ms: 1,
        block_id: "pane-1",
        deadline_ms: Date.now() + 120_000,
        expires_ms: Date.now() + 120_000,
        goes_ahead: false,
        ...over,
    }) as UseRequest;

const calls = (op: string) => vault.mock.calls.filter((c) => c[0] === op).map((c) => c[1] ?? {});
const row = (id: string) => document.querySelector(`tr[data-entry="${id}"]`) as HTMLElement;

beforeEach(() => {
    vault.mockClear();
    copied.mockClear();
    uses = [];
    entries = [];
});
afterEach(cleanup);

describe("a linked entry's fields and what a check of its place found", () => {
    it("is marked by link: only, and saves a date of last change for a link alone", () => {
        expect(isLinked({ link: "only" })).toBe(true);
        expect(isLinked({ source_kind: "aws-secret" })).toBe(false);
        expect(linkFields({ link: "only", readers: " a \n\n b ", rotated: " 2026-10-01 ", copies: "", service: "Example" })).toEqual({ link: "only", readers: "a\nb", rotated: "2026-10-01" });
        expect(linkFields({ readers: "a", rotated: "2026-10-01" })).toEqual({ readers: "a" });
    });

    it("says the place is there, changed or missing, and why", () => {
        const at = Date.now() - 60_000;
        expect(sourceCheck({ source_checked: { at, outcome: "present" } })).toMatchObject({ state: "present", text: expect.stringContaining("it is there, checked") });
        const changed = sourceCheck({ source_checked: { at, outcome: "changed since recorded: the place last changed on 2026-11-02, after the 2026-10-01 recorded" } });
        expect(changed?.state).toBe("changed");
        expect(changed?.text).toMatch(/^changed since recorded, checked .*: the place last changed on 2026-11-02, after the 2026-10-01 recorded$/);
        expect(sourceCheck({ source_checked: { at, outcome: "missing: AWS has no secret by that id, in that region" } })).toMatchObject({ state: "missing", text: expect.stringContaining(": AWS has no secret by that id") });
        expect(sourceCheck({ source_checked: { at, outcome: "could not read: AWS refused: this identity can't read that secret" } })?.state).toBe("unread");
    });

    it("counts a link as unchecked once its newest check is more than 30 days old", () => {
        const now = 100 * DAY;
        const linked = { link: "only", ...PLACE };
        expect(uncheckedDays({ fields: linked, source_checked: { at: now - 45 * DAY, outcome: "present" } }, now)).toBe(45);
        expect(uncheckedDays({ fields: linked, source_checked: { at: now - 30 * DAY, outcome: "present" } }, now)).toBeNull();
        expect(uncheckedDays({ fields: linked }, now)).toBeNull();
        expect(uncheckedDays({ fields: PLACE, source_checked: { at: now - 45 * DAY, outcome: "same" } }, now)).toBeNull();
    });
});

describe("a linked entry in the list", () => {
    it("has its badges, where the key is kept, who reads it, and no value to show", async () => {
        entries = [entry({ source_checked: { at: Date.now() - 45 * DAY, outcome: "present" } })];
        await refreshVault();
        render(() => <VaultEntries section="api-keys" />);
        const line = row("api-key-invoice");
        expect(line.textContent).toContain("Linked");
        expect(line.textContent).toContain("Readable outside the Vault");
        expect(line.textContent).toContain("Kept at: AWS secret prod/invoice-sync, key api.token");
        expect(line.querySelector('[data-check="present"]')?.textContent).toContain("it is there, checked");
        expect(line.querySelector("[data-unchecked]")?.textContent).toContain("unchecked for 45 days");
        expect(line.querySelector('[data-said="readers"]')?.textContent).toBe("Read by: invoice-sync (lambda): at run time; release (workflow): on a tag");
        expect(line.querySelector('[data-said="rotated"]')?.textContent).toBe("Last changed, as recorded: 2026-10-01");
        expect(line.querySelector('[data-said="copies"]')?.textContent).toBe("Also kept at: github-secret: example-org/example-app#EXAMPLE_TOKEN");
        expect(line.querySelector(".vault-value")?.textContent).toBe("no value kept");
        expect(screen.queryByRole("button", { name: "Show or copy" })).toBeNull();

        fireEvent.click(screen.getByRole("button", { name: "Copy the place" }));
        await waitFor(() => expect(copied).toHaveBeenCalledWith("prod/invoice-sync"));
        fireEvent.click(screen.getByRole("button", { name: "Check the place" }));
        await waitFor(() => expect(calls("check_source")).toEqual([{ id: "api-key-invoice" }]));
    });

    it("says a delete removes the record of where the key lives, and for an entry that holds a value, the value", async () => {
        entries = [entry(), entry({ id: "api-key-staging", label: "Staging database", fields: { ...PLACE }, parts_set: ["value"], tail: "k4vz" })];
        await refreshVault();
        render(() => <VaultEntries section="api-keys" />);
        fireEvent.click(within(row("api-key-invoice")).getByRole("button", { name: "Delete" }));
        const asked = await screen.findByRole("dialog");
        expect(asked.textContent).toContain("Delete Invoice provider key?");
        expect(asked.textContent).toContain("This entry holds no value. Deleting it removes the record of where the key lives from this computer. The key itself is not touched.");
        expect(asked.textContent).not.toContain("Its value is deleted");
        fireEvent.click(within(asked).getByRole("button", { name: "Cancel" }));
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());

        fireEvent.click(within(row("api-key-staging")).getByRole("button", { name: "Delete" }));
        const other = await screen.findByRole("dialog");
        expect(other.textContent).toContain("Its value is deleted from this computer.");
        expect(other.textContent).not.toContain("holds no value");
    });

    it("leaves an entry that holds its value as it was", async () => {
        entries = [entry({ id: "api-key-staging", label: "Staging database", fields: { ...PLACE }, parts_set: ["value"], tail: "k4vz" })];
        await refreshVault();
        render(() => <VaultEntries section="api-keys" />);
        const line = row("api-key-staging");
        expect(line.textContent).not.toContain("Linked");
        expect(line.textContent).toContain("Machine copy: AWS secret prod/invoice-sync");
        expect(screen.getByRole("button", { name: "Show or copy" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Check the machine copy" })).toBeTruthy();
    });
});

describe("a linked entry in the form", () => {
    it("has no value to type, and a save keeps the link and what it says", async () => {
        render(() => <VaultEntryForm target={{ mode: "edit", entry: entry() }} onClose={() => {}} />);
        expect(screen.getByText("This entry holds no value. It says where the key lives and who reads it.")).toBeTruthy();
        expect(screen.queryByLabelText("Key")).toBeNull();
        expect(screen.queryByLabelText("Link only")).toBeNull();
        expect((screen.getByLabelText("Who reads it") as HTMLTextAreaElement).value).toBe(SAID.readers);
        fireEvent.input(screen.getByLabelText("Last changed"), { target: { value: "2026-11-02" } });
        fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
        await waitFor(() => expect(calls("update")).toHaveLength(1));
        const sent = calls("update")[0];
        expect(sent.fields).toEqual({ link: "only", ...PLACE, ...SAID, rotated: "2026-11-02" });
        expect(sent.parts).toEqual({});
        expect(sent.clear).toEqual([]);
    });

    it("makes a new entry a link with a switch, which takes the value's input away", async () => {
        render(() => <VaultEntryForm target={{ mode: "new", section: "api-keys" }} onClose={() => {}} />);
        fireEvent.input(screen.getByLabelText("Label"), { target: { value: "Invoice provider key" } });
        fireEvent.input(screen.getByLabelText("Key"), { target: { value: "typed-before-the-switch" } });
        fireEvent.click(screen.getByLabelText("Link only"));
        expect(screen.queryByLabelText("Key")).toBeNull();
        fireEvent.input(screen.getByLabelText("Secret"), { target: { value: "prod/invoice-sync" } });
        fireEvent.input(screen.getByLabelText("Who reads it"), { target: { value: "invoice-sync (lambda): at run time" } });
        fireEvent.click(screen.getByRole("button", { name: "Save to Vault" }));
        await waitFor(() => expect(calls("create")).toHaveLength(1));
        const sent = calls("create")[0];
        expect(sent.fields).toEqual({ link: "only", source_kind: "aws-secret", source_location: "prod/invoice-sync", readers: "invoice-sync (lambda): at run time" });
        expect(sent.parts).toEqual({});
    });
});

describe("the questions that make or change a linked entry", () => {
    const shown = async (over: Partial<UseRequest>) => {
        uses = [use(over)];
        await refreshVault();
        render(() => <UsePrompt use={uses[0]} close={vi.fn()} />);
        return await screen.findByRole("group");
    };

    it("says a batch of links records where keys live and reads nothing", async () => {
        const items = [
            { label: "Invoice provider key", kind: "api-key", secret: "prod/invoice-sync", key: "api.token", replaces: false, link: true },
            { label: "Mail provider key", kind: "api-key", secret: "prod/mail", key: "", replaces: false, link: true },
        ];
        const card = await shown({ entry_label: "invoice service keys", what: "secrets", batch: { name: "invoice service keys", service: "AWS Secrets Manager", region: "eu-west-1", profile: "work-read", grant: false, vault: "Fleet", agents: { mode: "all" }, vaults: [], items } });
        expect(card.textContent).toContain("Agent1 wants to record where 2 keys live: invoice service keys");
        expect(card.querySelector('[data-batch="source"]')?.textContent).toBe("The keys are in AWS Secrets Manager in eu-west-1. Nothing is read. ");
        expect(card.querySelector('[data-batch="vault"]')?.textContent).toContain("becomes a linked entry in the Vault Fleet");
        expect([...card.querySelectorAll('[data-row="link"]')].map((r) => r.textContent?.trim())).toEqual(["link only (no value read)", "link only (no value read)"]);
        expect(card.textContent).toContain("nothing is recorded without a yes");
        expect(card.textContent).toContain("No value is read and none is kept.");
        expect(card.textContent).not.toContain("work-read");
    });

    it("keeps the copying words for a batch that mixes links and copies", async () => {
        const items = [
            { label: "Invoice provider key", kind: "api-key", secret: "prod/invoice-sync", key: "", replaces: false, link: true },
            { label: "Alpha", kind: "login", secret: "hosts/alpha", key: "", replaces: false, link: false },
        ];
        const card = await shown({ entry_label: "mixed", what: "secrets", batch: { name: "mixed", service: "AWS Secrets Manager", region: "", profile: "", grant: false, vault: "Personal", agents: { mode: "all" }, vaults: [], items } });
        expect(card.textContent).toContain("Agent1 wants to copy 2 secrets from AWS Secrets Manager into your Vault: mixed");
        expect(card.querySelectorAll('[data-row="link"]').length).toBe(1);
        expect(card.querySelector('[data-row="copy"]')?.textContent?.trim()).toBe("new login entry");
    });

    it("shows what a link says now and what is asked for, when an agent changes who reads the key", async () => {
        const place = "the AWS secret prod/invoice-sync, key api.token, region eu-west-1";
        const link = { from: place, to: place, about_from: "Read by: invoice-sync (lambda): at run time.", about_to: "Read by: invoice-sync (lambda): at run time; release (workflow): on a tag.", only: true };
        const card = await shown({ entry_id: "api-key-invoice", link });
        expect(card.textContent).toContain("Agent1 wants to change what the Vault says about the key Invoice provider key");
        expect(card.textContent).toContain("is a linked entry: it holds no value");
        expect(card.querySelector('[data-link="about-from"]')?.textContent).toBe(link.about_from);
        expect(card.querySelector('[data-link="about-to"]')?.textContent).toBe(link.about_to);
        // The place stays as it is, so the card does not speak of changing it.
        expect(card.textContent).toContain("nothing is changed without a yes.");
        expect(card.textContent).toContain("This is what the Vault tells the next agent about the key.");
        expect(screen.queryByRole("button", { name: /Always/ })).toBeNull();
    });
});
