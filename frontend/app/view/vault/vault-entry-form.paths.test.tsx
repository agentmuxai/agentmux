// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The add / change form, path by path: what it titles itself and says for a new entry, an agent's
 *  request and a change, which kind it starts on and which it offers, the Vault and label rows, the
 *  secret parts (their hints, placeholders, Clear toggle and multiline note), the plain fields and the
 *  browser-profile choice, what a save sends for a new entry and for a change, the busy and error
 *  states, what is wiped after a save, and the note before the first save. Written ahead of splitting
 *  `VaultEntryForm`. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultRequest } from "@/types/rpc/VaultRequest";

const h = vi.hoisted(() => ({
    createEntry: vi.fn(),
    updateEntry: vi.fn(),
    ensure: vi.fn(),
    entries: undefined as unknown[] | undefined,
    profiles: [] as { id: string; name: string }[],
    platform: "win32",
    focused: [] as HTMLElement[],
}));
vi.mock("@/util/focusutil", async (orig) => ({
    ...(await orig<typeof import("@/util/focusutil")>()),
    focusOnOpen: (el: HTMLElement) => void h.focused.push(el),
}));

vi.mock("./vault-store", () => ({
    createEntry: (...a: unknown[]) => h.createEntry(...a),
    updateEntry: (...a: unknown[]) => h.updateEntry(...a),
    vaultEntries: () => h.entries,
}));
vi.mock("@/app/view/browser/browser-profiles", () => ({
    browserProfiles: () => h.profiles,
    ensureBrowserProfiles: () => h.ensure(),
}));
vi.mock("@/util/platformutil", () => ({ getPlatform: () => h.platform }));
vi.mock("./vault-vaults", () => ({
    VaultField: (props: { value: string; onChange: (id: string) => void; description?: string }) => (
        <div
            data-testid="vault-field"
            data-value={props.value}
            data-description={props.description}
            onClick={() => props.onChange("work")}
        />
    ),
}));

import { VaultEntryForm, type VaultFormTarget } from "./vault-entry-form";

const dialog = () => screen.getByRole("dialog");
const type = (el: HTMLElement, value: string) => fireEvent.input(el, { target: { value } });
const field = (label: string) => screen.getByLabelText(label) as HTMLInputElement;
const maybeField = (label: string) => screen.queryByLabelText(label) as HTMLInputElement | null;
const btn = (name: string) => screen.getByRole("button", { name });
const vaultField = () => screen.queryByTestId("vault-field");
const hintOf = (label: string) =>
    field(label).closest(".ui-field")!.querySelector(".ui-field-hint")?.textContent ?? null;
const rows = () => Array.from(dialog().querySelectorAll(".ui-field-label")).map((l) => l.textContent);
const selectOf = (label: string) => screen.getByLabelText(label) as unknown as HTMLSelectElement;
const optionsOf = (s: HTMLSelectElement) => Array.from(s.options).map((o) => [o.value, o.textContent]);

function gate<T>() {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

const entry = (over: Partial<VaultEntryInfo> = {}): VaultEntryInfo => ({
    id: "e1",
    kind: "api-key",
    section: "api-keys",
    label: "Stripe live",
    vault_id: "personal",
    fields: { service: "Stripe" },
    tags: ["billing"],
    parts_set: ["value"],
    grants: [],
    tail: "1234",
    versions: 0,
    created_at: 0,
    updated_at: 0,
    last_used_at: 0,
    ...over,
});
const req = (over: Partial<VaultRequest> = {}): VaultRequest => ({
    id: "r1",
    agent: "Clamk",
    kind: "api-key",
    label: "Stripe key",
    why: "to open the invoices",
    requested_ms: 0,
    block_id: "",
    vault: "",
    ...over,
});

let onClose: ReturnType<typeof vi.fn>;
const mount = (target: VaultFormTarget) =>
    render(() => <VaultEntryForm target={target} onClose={onClose as unknown as (id?: string) => void} />);
const mountNew = (
    section: "personal" | "cards" | "api-keys" | "logins" | "notes" = "api-keys",
    extra: { request?: VaultRequest; vault?: string } = {}
) => mount({ mode: "new", section, ...extra });
const mountEdit = (over: Partial<VaultEntryInfo> = {}) => mount({ mode: "edit", entry: entry(over) });

beforeEach(() => {
    onClose = vi.fn();
    h.createEntry.mockReset().mockResolvedValue({ id: "new-1" });
    h.updateEntry.mockReset().mockResolvedValue({});
    h.ensure.mockReset();
    h.entries = [{}];
    h.profiles = [];
    h.platform = "win32";
    h.focused = [];
});
afterEach(cleanup);

describe("title and description", () => {
    it("titles a new entry 'Add to the Vault' with no description", () => {
        mountNew();
        expect(screen.getByText("Add to the Vault")).toBeTruthy();
        expect(dialog().querySelector(".modal-panel-description")).toBeNull();
    });

    it("titles an agent's request with who asks for what, and gives its reason", () => {
        mountNew("api-keys", { request: req() });
        expect(screen.getByText("Clamk asks for: Stripe key")).toBeTruthy();
        expect(screen.getByText("Why: to open the invoices")).toBeTruthy();
    });

    it("says no reason when the request has none", () => {
        mountNew("api-keys", { request: req({ why: "" }) });
        expect(screen.getByText("Clamk asks for: Stripe key")).toBeTruthy();
        expect(dialog().textContent).not.toContain("Why:");
    });

    it("titles a change with the entry's label and says values are not shown", () => {
        mountEdit();
        expect(screen.getByText("Change Stripe live")).toBeTruthy();
        expect(
            screen.getByText("Values aren't shown. Type one to replace it; leave it empty to keep it.")
        ).toBeTruthy();
    });

    it("loads the browser profiles when it opens", () => {
        mountNew();
        expect(h.ensure).toHaveBeenCalledTimes(1);
    });
});

describe("closing", () => {
    it("Cancel closes with nothing saved", () => {
        mountNew();
        fireEvent.click(btn("Cancel"));
        expect(onClose).toHaveBeenCalledTimes(1);
        expect(onClose).toHaveBeenCalledWith();
        expect(h.createEntry).not.toHaveBeenCalled();
    });

    it("Escape closes it with nothing saved", () => {
        mountNew();
        fireEvent.keyDown(btn("Cancel"), { key: "Escape" });
        expect(onClose).toHaveBeenCalledWith();
    });

    it("a click on the backdrop does not close it, but nudges Cancel", () => {
        mountNew();
        const backdrop = document.querySelector(".modal-backdrop")!;
        fireEvent.click(backdrop);
        expect(onClose).not.toHaveBeenCalled();
        expect(btn("Cancel").classList.contains("modal-dismiss--nudge")).toBe(true);
        expect(btn("Save to Vault").classList.contains("modal-dismiss--nudge")).toBe(false);
    });

    it("is the vault form panel", () => {
        mountNew();
        expect(document.querySelector(".modal-panel.vault-form")).not.toBeNull();
    });

    it("Cancel is off while saving", async () => {
        const g = gate<{ id: string }>();
        h.createEntry.mockReturnValue(g.promise);
        mountNew();
        type(field("Key"), "sk-1");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect((btn("Cancel") as HTMLButtonElement).disabled).toBe(true));
        g.resolve({ id: "x" });
        await waitFor(() => expect((btn("Cancel") as HTMLButtonElement).disabled).toBe(false));
    });
});

describe("the kind", () => {
    it("starts on the section's first kind", () => {
        mountNew("personal");
        expect(rows()).toContain("Number");
        expect(field("Number").placeholder).toBe("123-45-6789");
    });

    it("offers the section's kinds in a Kind choice", () => {
        mountNew("cards");
        const sel = selectOf("Kind");
        expect(optionsOf(sel)).toEqual([
            ["card", "Payment card"],
            ["bank-account", "Bank account"],
        ]);
        expect(sel.value).toBe("card");
    });

    it("has no Kind choice when the section has one kind", () => {
        mountNew("notes");
        expect(maybeField("Kind")).toBeNull();
        expect(rows()).toContain("Text");
    });

    it("has no Kind choice when changing an entry", () => {
        mountEdit({ kind: "card", section: "cards", parts_set: [] });
        expect(maybeField("Kind")).toBeNull();
    });

    it("starts on the kind an agent's request names, and offers that kind's section", () => {
        mountNew("api-keys", { request: req({ kind: "ssn" }) });
        expect(rows()).toContain("Whose");
        expect(optionsOf(selectOf("Kind")).map((o) => o[0])).toEqual([
            "ssn",
            "itin",
            "tin",
            "ein",
            "date-of-birth",
            "passport",
            "drivers-licence",
            "other-id",
        ]);
        expect(selectOf("Kind").value).toBe("ssn");
    });

    it("switching kind swaps the parts and fields", () => {
        mountNew("cards");
        expect(rows()).toEqual(
            expect.arrayContaining([
                "Card number",
                "Expiry",
                "Security code (CVC, 3 digits)",
                "Name on card",
                "Billing ZIP or postal code",
            ])
        );
        fireEvent.change(selectOf("Kind"), { target: { value: "bank-account" } });
        expect(rows()).toEqual(expect.arrayContaining(["Account number", "Routing number", "Bank"]));
        expect(rows()).not.toContain("Card number");
    });

    it("shows no parts or fields for a kind it does not know, and saving does nothing", () => {
        mountEdit({ kind: "mystery", section: "notes", parts_set: [] });
        expect(rows()).toEqual(["Label"]);
        fireEvent.click(btn("Save changes"));
        expect(h.updateEntry).not.toHaveBeenCalled();
        expect(h.createEntry).not.toHaveBeenCalled();
        expect(screen.queryByRole("alert")).toBeNull();
        expect((btn("Save changes") as HTMLButtonElement).disabled).toBe(false);
    });
});

describe("the Vault row", () => {
    it("is offered for a new entry, starting on Personal, with its description", () => {
        mountNew();
        expect(vaultField()!.getAttribute("data-value")).toBe("personal");
        expect(vaultField()!.getAttribute("data-description")).toBe("Which agents may use it is the Vault's setting.");
    });

    it("starts on the Vault the lists are on", () => {
        mountNew("api-keys", { vault: "work" });
        expect(vaultField()!.getAttribute("data-value")).toBe("work");
    });

    it("starts on the Vault an agent asked for", () => {
        mountNew("api-keys", { request: req({ vault: "ops" }) });
        expect(vaultField()!.getAttribute("data-value")).toBe("ops");
    });

    it("prefers the Vault the lists are on over the one a request names", () => {
        mountNew("api-keys", { vault: "work", request: req({ vault: "ops" }) });
        expect(vaultField()!.getAttribute("data-value")).toBe("work");
    });

    it("is not offered when changing an entry", () => {
        mountEdit();
        expect(vaultField()).toBeNull();
    });

    it("sends the Vault that was picked", async () => {
        mountNew();
        type(field("Key"), "sk-1");
        fireEvent.click(vaultField()!);
        expect(vaultField()!.getAttribute("data-value")).toBe("work");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        expect(h.createEntry.mock.calls[0][0].vault).toBe("work");
    });
});

describe("the label", () => {
    it("starts empty for a new entry, with a limit and a description", () => {
        mountNew();
        expect(field("Label").value).toBe("");
        expect(field("Label").maxLength).toBe(200);
        expect(dialog().textContent).toContain("How you and agents name it. Agents see labels, never values.");
    });

    it("starts on the label an agent asked for", () => {
        mountNew("api-keys", { request: req() });
        expect(field("Label").value).toBe("Stripe key");
    });

    it("starts on the entry's label when changing", () => {
        mountEdit();
        expect(field("Label").value).toBe("Stripe live");
    });

    it("asks for focus on the label when there is no request", () => {
        mountNew();
        expect(h.focused).toEqual([field("Label")]);
    });

    it("asks for focus on the main part, not the label, when an agent asked", () => {
        mountNew("cards", { request: req({ kind: "card" }) });
        expect(h.focused).toEqual([field("Card number")]);
    });

    it("asks for focus on the label, and on no part, when changing", () => {
        mountEdit();
        expect(h.focused).toEqual([field("Label")]);
    });
});

describe("the secret parts", () => {
    it("wraps a part's control and its Clear in one box", () => {
        mountEdit({ kind: "bank-account", section: "cards", parts_set: ["account", "routing"], fields: {} });
        const box = field("Routing number").closest(".vault-part")!;
        expect(box).not.toBeNull();
        expect(box.contains(btn("Clear"))).toBe(true);
    });

    it("saves what is typed in a secret note as the note's part, not a field", async () => {
        mountNew("notes");
        type(field("Text"), "the vault code is 42");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        const sent = h.createEntry.mock.calls[0][0];
        expect(sent.parts).toEqual({ text: "the vault code is 42" });
        expect(sent.fields).toEqual({});
        expect((field("Text") as unknown as HTMLTextAreaElement).value).toBe("");
    });

    it("keeps what is typed in the note's text area on screen until it is saved", () => {
        mountNew("notes");
        type(field("Text"), "abc");
        expect((field("Text") as unknown as HTMLTextAreaElement).value).toBe("abc");
    });

    it("names the text areas for assistive tech by their label", () => {
        mountNew("api-keys");
        expect(field("Notes").getAttribute("aria-label")).toBe("Notes");
        cleanup();
        mountNew("notes");
        expect(field("Text").getAttribute("aria-label")).toBe("Text");
    });

    it("types a part hidden, without browser help", () => {
        mountNew();
        const key = field("Key");
        expect(key.type).toBe("password");
        expect(key.getAttribute("autocomplete")).toBe("new-password");
        expect(key.getAttribute("spellcheck")).toBe("false");
        expect(key.disabled).toBe(false);
    });

    it("keeps the form from filling itself", () => {
        mountNew();
        expect(dialog().querySelector("form")!.getAttribute("autocomplete")).toBe("off");
    });

    it("says the main part is required, and the others nothing, on a new entry", () => {
        mountNew("cards");
        expect(hintOf("Card number")).toBe("Required.");
        expect(hintOf("Security code (CVC, 3 digits)")).toBeNull();
    });

    it("shows the kind's placeholder on a new entry", () => {
        mountNew("cards");
        expect(field("Card number").placeholder).toBe("1234 5678 9012 3456");
    });

    it("says a part that is set is kept until a new value is typed, and masks it", () => {
        mountEdit({ kind: "card", section: "cards", parts_set: ["number", "cvc"], fields: {} });
        expect(hintOf("Card number")).toBe("Set. Type to replace it.");
        expect(field("Card number").placeholder).toBe("•••• •••• •••• ••••");
        expect(hintOf("Security code (CVC, 3 digits)")).toBe("Set. Type to replace it.");
        expect(field("Security code (CVC, 3 digits)").placeholder).toBe("••••••••");
    });

    it("says the main part is required when changing an entry whose main part is not set", () => {
        mountEdit({ kind: "card", section: "cards", parts_set: ["cvc"], fields: {} });
        expect(hintOf("Card number")).toBe("Required.");
        expect(field("Card number").placeholder).toBe("1234 5678 9012 3456");
    });

    it("says nothing about a part that is not set and not the main one", () => {
        mountEdit({ kind: "card", section: "cards", parts_set: ["number"], fields: {} });
        expect(hintOf("Security code (CVC, 3 digits)")).toBeNull();
        expect(field("Security code (CVC, 3 digits)").placeholder).toBe("");
    });

    it("offers Clear on a set part that is not the main one", () => {
        mountEdit({ kind: "card", section: "cards", parts_set: ["number", "cvc", "name"], fields: {} });
        const clears = screen.getAllByRole("button", { name: "Clear" });
        expect(clears).toHaveLength(2);
        const owners = clears.map((c) => c.closest(".ui-field")!.querySelector("label")!.textContent);
        expect(owners).toEqual(["Security code (CVC, 3 digits)", "Name on card"]);
    });

    it("offers no Clear on a new entry or on a part that is not set", () => {
        mountNew("cards");
        expect(screen.queryByRole("button", { name: "Clear" })).toBeNull();
        cleanup();
        mountEdit({ kind: "card", section: "cards", parts_set: ["number"], fields: {} });
        expect(screen.queryByRole("button", { name: "Clear" })).toBeNull();
    });

    it("Clear turns on, disables the part and says it will be cleared; again turns it off", () => {
        mountEdit({ kind: "card", section: "cards", parts_set: ["number", "cvc"], fields: {} });
        const clear = btn("Clear");
        expect(clear.getAttribute("aria-pressed")).toBe("false");
        fireEvent.click(clear);
        expect(clear.getAttribute("aria-pressed")).toBe("true");
        expect(field("Security code (CVC, 3 digits)").disabled).toBe(true);
        expect(hintOf("Security code (CVC, 3 digits)")).toBe("Will be cleared.");
        expect(field("Card number").disabled).toBe(false);
        fireEvent.click(clear);
        expect(clear.getAttribute("aria-pressed")).toBe("false");
        expect(field("Security code (CVC, 3 digits)").disabled).toBe(false);
        expect(hintOf("Security code (CVC, 3 digits)")).toBe("Set. Type to replace it.");
    });

    it("types a multiline part in a masked text area", () => {
        mountNew("notes");
        const ta = field("Text") as unknown as HTMLTextAreaElement;
        expect(ta.tagName).toBe("TEXTAREA");
        expect(ta.classList.contains("vault-secret-text")).toBe(true);
        expect(ta.classList.contains("vault-textarea")).toBe(true);
        expect(ta.rows).toBe(4);
        expect(ta.getAttribute("spellcheck")).toBe("false");
        expect(ta.getAttribute("autocomplete")).toBe("off");
        expect(ta.placeholder).toBe("");
        expect(hintOf("Text")).toBe("Required.");
    });

    it("tells a set multiline part to be typed over", () => {
        mountEdit({ kind: "note", section: "notes", parts_set: ["text"], fields: {} });
        const ta = field("Text") as unknown as HTMLTextAreaElement;
        expect(ta.placeholder).toBe("Set. Type to replace it.");
        expect(hintOf("Text")).toBe("Set. Type to replace it.");
    });
});

describe("the plain fields", () => {
    it("shows each field with its placeholder, and the multiline one as an open text area", () => {
        mountNew("api-keys");
        expect(field("Service").placeholder).toBe("e.g. Stripe");
        expect(field("Environment variable").placeholder).toBe("e.g. STRIPE_API_KEY");
        const notes = field("Notes") as unknown as HTMLTextAreaElement;
        expect(notes.tagName).toBe("TEXTAREA");
        expect(notes.classList.contains("vault-secret-text")).toBe(false);
        expect(notes.rows).toBe(4);
    });

    it("starts a change on the entry's own field values", () => {
        mountEdit({ fields: { service: "Stripe", notes: "live" } });
        expect(field("Service").value).toBe("Stripe");
        expect((field("Notes") as unknown as HTMLTextAreaElement).value).toBe("live");
    });

    it("hides the fields srv keeps, but sends them back on a change", async () => {
        mountEdit({
            kind: "login",
            section: "logins",
            parts_set: ["password"],
            fields: { site: "https://a.test", realm: "Staff", proxy: "p1" },
        });
        expect(maybeField("Realm")).toBeNull();
        expect(maybeField("Proxy")).toBeNull();
        fireEvent.click(btn("Save changes"));
        await waitFor(() => expect(h.updateEntry).toHaveBeenCalled());
        expect(h.updateEntry.mock.calls[0][0].fields).toEqual({ site: "https://a.test", realm: "Staff", proxy: "p1" });
    });
});

describe("the browser profile choice", () => {
    it("is hidden until a site is typed, and again when it is blank", () => {
        mountNew("logins");
        expect(maybeField("Browser profile")).toBeNull();
        type(field("Site"), "   ");
        expect(maybeField("Browser profile")).toBeNull();
        type(field("Site"), "https://a.test");
        expect(maybeField("Browser profile")).not.toBeNull();
    });

    it("lists Personal and the named profiles, starting on Personal, and says whose tabs get it", () => {
        h.profiles = [
            { id: "p1", name: "Work" },
            { id: "p2", name: "Side" },
        ];
        mountNew("logins");
        type(field("Site"), "https://a.test");
        const sel = selectOf("Browser profile");
        expect(optionsOf(sel)).toEqual([
            ["personal", "Personal"],
            ["profile:p1", "Work"],
            ["profile:p2", "Side"],
        ]);
        expect(sel.value).toBe("personal");
        expect(dialog().textContent).toContain("Only this profile's tabs are offered it.");
        expect(dialog().textContent!.split("Only this profile's tabs are offered it.")).toHaveLength(2);
        expect(field("Site").closest(".ui-field")!.querySelector(".ui-field-description")).toBeNull();
    });

    it("keeps a deleted profile on the list while the entry names it", () => {
        h.profiles = [{ id: "p1", name: "Work" }];
        mountEdit({
            kind: "login",
            section: "logins",
            parts_set: ["password"],
            fields: { site: "https://a.test", profile: "profile:gone" },
        });
        const sel = selectOf("Browser profile");
        expect(optionsOf(sel)).toEqual([
            ["personal", "Personal"],
            ["profile:p1", "Work"],
            ["profile:gone", "A deleted profile"],
        ]);
        expect(sel.value).toBe("profile:gone");
    });

    it("does not call a profile that still exists deleted, nor Personal", () => {
        h.profiles = [{ id: "p1", name: "Work" }];
        mountEdit({
            kind: "login",
            section: "logins",
            parts_set: ["password"],
            fields: { site: "https://a.test", profile: "profile:p1" },
        });
        expect(optionsOf(selectOf("Browser profile")).map((o) => o[1])).toEqual(["Personal", "Work"]);
        cleanup();
        mountEdit({
            kind: "login",
            section: "logins",
            parts_set: ["password"],
            fields: { site: "https://a.test", profile: "personal" },
        });
        expect(optionsOf(selectOf("Browser profile")).map((o) => o[1])).toEqual(["Personal", "Work"]);
    });

    it("sends the profile that was picked", async () => {
        h.profiles = [{ id: "p1", name: "Work" }];
        mountNew("logins");
        type(field("Site"), "https://a.test");
        type(field("Password"), "pw");
        fireEvent.change(selectOf("Browser profile"), { target: { value: "profile:p1" } });
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        expect(h.createEntry.mock.calls[0][0].fields).toEqual({ site: "https://a.test", profile: "profile:p1" });
    });

    it("sends no profile when none was picked", async () => {
        mountNew("logins");
        type(field("Site"), "https://a.test");
        type(field("Password"), "pw");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        expect(h.createEntry.mock.calls[0][0].fields).toEqual({ site: "https://a.test" });
    });
});

describe("saving a new entry", () => {
    it("sends the kind, label, typed fields and parts, the Vault and the request, then closes with the new id", async () => {
        mountNew("api-keys", { request: req() });
        type(field("Label"), "Stripe prod");
        type(field("Key"), "sk-live-1");
        type(field("Service"), "Stripe");
        type(field("Notes"), "rotate monthly");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(onClose).toHaveBeenCalledWith("new-1"));
        expect(h.createEntry).toHaveBeenCalledTimes(1);
        expect(h.createEntry).toHaveBeenCalledWith({
            kind: "api-key",
            label: "Stripe prod",
            fields: { service: "Stripe", notes: "rotate monthly" },
            parts: { value: "sk-live-1" },
            request_id: "r1",
            vault: "personal",
        });
        expect(h.updateEntry).not.toHaveBeenCalled();
    });

    it("sends no request id for an entry nobody asked for", async () => {
        mountNew();
        type(field("Key"), "sk-1");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        expect(h.createEntry.mock.calls[0][0].request_id).toBeUndefined();
    });

    it("leaves out whatever was left empty", async () => {
        mountNew("cards");
        type(field("Card number"), "4111");
        type(field("Name on card"), "");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        const sent = h.createEntry.mock.calls[0][0];
        expect(sent.parts).toEqual({ number: "4111" });
        expect(sent.fields).toEqual({});
    });

    it("leaves behind what was typed for another kind", async () => {
        mountNew("cards");
        type(field("Card number"), "4111");
        type(field("Whose"), "Alex");
        fireEvent.change(selectOf("Kind"), { target: { value: "bank-account" } });
        type(field("Account number"), "999");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        const sent = h.createEntry.mock.calls[0][0];
        expect(sent.kind).toBe("bank-account");
        expect(sent.parts).toEqual({ account: "999" });
        expect(sent.fields).toEqual({ owner: "Alex" });
    });

    it("saves on the Save button and on Enter in a field, and keeps the form from reloading the page", async () => {
        mountNew();
        type(field("Key"), "sk-1");
        const form = dialog().querySelector("form")!;
        expect(form.querySelector('input[type="submit"]')!.hasAttribute("hidden")).toBe(true);
        const notPrevented = fireEvent.submit(form);
        expect(notPrevented).toBe(false);
        await waitFor(() => expect(h.createEntry).toHaveBeenCalledTimes(1));
    });
});

describe("saving a change", () => {
    it("sends the id, label, kept tags, typed fields and parts, and what is to be cleared, then closes with the id", async () => {
        mountEdit({
            kind: "card",
            section: "cards",
            label: "Visa",
            parts_set: ["number", "exp_month", "cvc"],
            fields: { owner: "Alex" },
            tags: ["t1", "t2"],
        });
        type(field("Label"), "Visa 2");
        type(field("Card number"), "4222");
        type(field("Whose"), "Alexa");
        // The expiry row's Clear comes first, then the security code's.
        const clears = screen.getAllByRole("button", { name: "Clear" });
        fireEvent.click(clears[1]);
        fireEvent.click(btn("Save changes"));
        await waitFor(() => expect(onClose).toHaveBeenCalledWith("e1"));
        expect(h.updateEntry).toHaveBeenCalledTimes(1);
        expect(h.updateEntry).toHaveBeenCalledWith({
            id: "e1",
            label: "Visa 2",
            fields: { owner: "Alexa" },
            tags: ["t1", "t2"],
            parts: { number: "4222" },
            clear: ["cvc"],
        });
        expect(h.createEntry).not.toHaveBeenCalled();
    });

    it("sends no parts when none were typed, so each keeps its value", async () => {
        mountEdit();
        fireEvent.click(btn("Save changes"));
        await waitFor(() => expect(h.updateEntry).toHaveBeenCalled());
        const sent = h.updateEntry.mock.calls[0][0];
        expect(sent.parts).toEqual({});
        expect(sent.clear).toEqual([]);
        expect(sent.fields).toEqual({ service: "Stripe" });
    });

    it("clears in the kind's order", async () => {
        mountEdit({ kind: "card", section: "cards", parts_set: ["number", "cvc", "name", "zip"], fields: {} });
        const clears = screen.getAllByRole("button", { name: "Clear" });
        fireEvent.click(clears[2]);
        fireEvent.click(clears[0]);
        fireEvent.click(btn("Save changes"));
        await waitFor(() => expect(h.updateEntry).toHaveBeenCalled());
        expect(h.updateEntry.mock.calls[0][0].clear).toEqual(["cvc", "zip"]);
    });

    it("drops a field that was emptied", async () => {
        mountEdit();
        type(field("Service"), "");
        fireEvent.click(btn("Save changes"));
        await waitFor(() => expect(h.updateEntry).toHaveBeenCalled());
        expect(h.updateEntry.mock.calls[0][0].fields).toEqual({});
    });
});

describe("while saving and when it fails", () => {
    it("shows Save busy and disabled until the save answers, and clears any earlier error as it starts", async () => {
        h.createEntry.mockRejectedValueOnce(new Error("no keychain"));
        mountNew();
        type(field("Key"), "sk-1");
        fireEvent.click(btn("Save to Vault"));
        await screen.findByRole("alert");
        const g = gate<{ id: string }>();
        h.createEntry.mockReturnValueOnce(g.promise);
        type(field("Key"), "sk-1");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(btn("Save to Vault").getAttribute("aria-busy")).toBe("true"));
        expect((btn("Save to Vault") as HTMLButtonElement).disabled).toBe(true);
        expect(screen.queryByRole("alert")).toBeNull();
        g.resolve({ id: "ok" });
        await waitFor(() => expect(btn("Save to Vault").getAttribute("aria-busy")).toBeNull());
        expect(onClose).toHaveBeenCalledWith("ok");
    });

    it("shows an Error's message in an alert and does not close", async () => {
        h.createEntry.mockRejectedValue(new Error("no keychain"));
        mountNew();
        type(field("Key"), "sk-1");
        fireEvent.click(btn("Save to Vault"));
        const alert = await screen.findByRole("alert");
        expect(alert.textContent).toBe("no keychain");
        expect(alert.classList.contains("warden-manager-error")).toBe(true);
        expect(onClose).not.toHaveBeenCalled();
        expect(btn("Save to Vault").getAttribute("aria-busy")).toBeNull();
    });

    it("shows a thrown string as it is", async () => {
        h.updateEntry.mockRejectedValue("refused");
        mountEdit();
        fireEvent.click(btn("Save changes"));
        expect((await screen.findByRole("alert")).textContent).toBe("refused");
        expect(onClose).not.toHaveBeenCalled();
    });

    it("wipes the typed secrets after a failed save, and keeps the rest", async () => {
        h.createEntry.mockRejectedValue(new Error("nope"));
        mountNew("cards");
        type(field("Label"), "Visa");
        type(field("Card number"), "4111");
        type(field("Whose"), "Alex");
        fireEvent.click(btn("Save to Vault"));
        await screen.findByRole("alert");
        expect(field("Card number").value).toBe("");
        expect(field("Label").value).toBe("Visa");
        expect(field("Whose").value).toBe("Alex");
    });

    it("wipes the typed secrets after a good save too", async () => {
        mountNew();
        type(field("Key"), "sk-1");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(onClose).toHaveBeenCalled());
        await waitFor(() => expect(field("Key").value).toBe(""));
    });
});

describe("the note before the first save", () => {
    it("tells a Mac what will ask when the Vault is empty", () => {
        h.platform = "darwin";
        h.entries = [];
        mountNew();
        const note = screen.getByRole("note");
        expect(note.textContent).toBe(
            "Saving creates your Vault key: macOS asks whether AgentMux may use the keychain. Choose Always Allow."
        );
        expect(note.classList.contains("vault-prompt-note")).toBe(true);
    });

    it("tells Linux about the keyring", () => {
        h.platform = "linux";
        h.entries = [];
        mountNew();
        expect(screen.getByRole("note").textContent).toBe(
            "Saving creates your Vault key: if your keyring is locked, your desktop asks for its password."
        );
    });

    it("shows it while the list has not loaded", () => {
        h.platform = "darwin";
        h.entries = undefined;
        mountNew();
        expect(screen.getByRole("note")).toBeTruthy();
    });

    it("says nothing on Windows", () => {
        h.platform = "win32";
        h.entries = [];
        mountNew();
        expect(screen.queryByRole("note")).toBeNull();
    });

    it("says nothing once the Vault has an entry", () => {
        h.platform = "darwin";
        h.entries = [{}];
        mountNew();
        expect(screen.queryByRole("note")).toBeNull();
    });

    it("says nothing when changing an entry", () => {
        h.platform = "darwin";
        h.entries = [];
        mountEdit();
        expect(screen.queryByRole("note")).toBeNull();
    });
});

describe("the buttons", () => {
    it("makes Save the accent button, and Cancel plain", () => {
        mountNew();
        expect(btn("Save to Vault").classList.contains("ui-tone-accent")).toBe(true);
        expect(btn("Cancel").classList.contains("ui-tone-accent")).toBe(false);
    });

    it("names Save by what it does", () => {
        mountNew();
        expect(btn("Save to Vault")).toBeTruthy();
        cleanup();
        mountEdit();
        expect(btn("Save changes")).toBeTruthy();
    });

    it("Save saves like Enter", async () => {
        mountEdit();
        fireEvent.click(within(dialog()).getByRole("button", { name: "Save changes" }));
        await waitFor(() => expect(h.updateEntry).toHaveBeenCalledTimes(1));
    });
});

describe("the card form", () => {
    const CVC3 = "Security code (CVC, 3 digits)";
    const CVC4 = "Security code (CID, 4 digits)";

    it("groups the number as it is typed and badges its brand", () => {
        mountNew("cards");
        type(field("Card number"), "4111111111111111");
        expect(field("Card number").value).toBe("4111 1111 1111 1111");
        expect(dialog().querySelector(".vault-card-number .ui-pill")?.textContent).toBe("Visa");
        type(field("Card number"), "378282246310005");
        expect(field("Card number").value).toBe("3782 822463 10005");
        expect(dialog().querySelector(".vault-card-number .ui-pill")?.textContent).toBe("American Express");
    });

    it("warns that a number doesn't look right, and still lets it be saved", async () => {
        mountNew("cards");
        type(field("Card number"), "4111111111111112");
        expect(dialog().querySelector(".vault-card-warn")?.textContent).toContain("doesn't look right");
        type(field("Label"), "Visa");
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        expect(h.createEntry.mock.calls[0][0].parts.number).toBe("4111 1111 1111 1112");
    });

    it("says nothing about a number that passes, or one still being typed", () => {
        mountNew("cards");
        type(field("Card number"), "4111111111111111");
        expect(dialog().querySelector(".vault-card-warn")).toBeNull();
        type(field("Card number"), "4111");
        expect(dialog().querySelector(".vault-card-warn")).toBeNull();
    });

    it("gives an American Express a four-digit code", () => {
        mountNew("cards");
        expect(field(CVC3).maxLength).toBe(3);
        type(field("Card number"), "378282246310005");
        expect(field(CVC4).maxLength).toBe(4);
        expect(maybeField(CVC3)).toBeNull();
    });

    it("keeps the saved card's brand until a new number is typed", () => {
        mountEdit({ kind: "card", section: "cards", parts_set: ["number"], fields: { brand: "amex" } });
        expect(field(CVC4).maxLength).toBe(4);
        type(field("Card number"), "4111");
        expect(field(CVC3).maxLength).toBe(3);
    });

    it("sends the month and the year it was given, as two parts", async () => {
        mountNew("cards");
        type(field("Label"), "Visa");
        type(field("Card number"), "4111111111111111");
        fireEvent.change(selectOf("Expiry month"), { target: { value: "03" } });
        const year = String(new Date().getFullYear() + 2);
        fireEvent.change(selectOf("Expiry year"), { target: { value: year } });
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        expect(h.createEntry.mock.calls[0][0].parts).toEqual({
            number: "4111 1111 1111 1111",
            exp_month: "03",
            exp_year: year,
        });
    });

    it("asks for the month and the year together", async () => {
        mountNew("cards");
        type(field("Card number"), "4111111111111111");
        fireEvent.change(selectOf("Expiry month"), { target: { value: "03" } });
        fireEvent.click(btn("Save to Vault"));
        await screen.findByRole("alert");
        expect(screen.getByRole("alert").textContent).toContain("Choose the expiry month and the year.");
        expect(h.createEntry).not.toHaveBeenCalled();
    });

    it("shows a saved card's expiry from its plain date and sends nothing for it until changed", async () => {
        mountEdit({
            kind: "card",
            section: "cards",
            parts_set: ["number", "exp_month", "exp_year"],
            fields: { expires: "2031-07-31" },
        });
        expect(selectOf("Expiry month").value).toBe("07");
        expect(selectOf("Expiry year").value).toBe("2031");
        fireEvent.click(btn("Save changes"));
        await waitFor(() => expect(h.updateEntry).toHaveBeenCalled());
        expect(h.updateEntry.mock.calls[0][0].parts).toEqual({});
    });

    it("sends both parts when only the month of a saved card changes", async () => {
        mountEdit({
            kind: "card",
            section: "cards",
            parts_set: ["number", "exp_month", "exp_year"],
            fields: { expires: "2031-07-31" },
        });
        fireEvent.change(selectOf("Expiry month"), { target: { value: "11" } });
        fireEvent.click(btn("Save changes"));
        await waitFor(() => expect(h.updateEntry).toHaveBeenCalled());
        expect(h.updateEntry.mock.calls[0][0].parts).toEqual({ exp_month: "11", exp_year: "2031" });
    });

    it("offers a year that has already gone by when the saved card has one", () => {
        mountEdit({ kind: "card", section: "cards", parts_set: ["number"], fields: { expires: "2019-02-28" } });
        expect(selectOf("Expiry year").value).toBe("2019");
    });

    it("takes a saved expiry off with Clear: both parts and the older one are cleared, and nothing is sent for them", async () => {
        mountEdit({
            kind: "card",
            section: "cards",
            parts_set: ["number", "exp_month", "exp_year"],
            fields: { expires: "2031-07-31" },
        });
        const clear = btn("Clear");
        fireEvent.click(clear);
        expect(clear.getAttribute("aria-pressed")).toBe("true");
        expect(selectOf("Expiry month").disabled).toBe(true);
        expect(selectOf("Expiry year").disabled).toBe(true);
        expect(selectOf("Expiry month").value).toBe("");
        fireEvent.click(btn("Save changes"));
        await waitFor(() => expect(h.updateEntry).toHaveBeenCalled());
        const sent = h.updateEntry.mock.calls[0][0];
        expect(sent.clear).toEqual(["exp_month", "exp_year", "expiry"]);
        expect(sent.parts).toEqual({});
    });

    it("offers no Clear on an expiry that isn't saved, and lets Clear be undone", () => {
        mountNew("cards");
        expect(screen.queryByRole("button", { name: "Clear" })).toBeNull();
        cleanup();
        mountEdit({ kind: "card", section: "cards", parts_set: ["expiry"], fields: {} });
        const clear = btn("Clear");
        fireEvent.click(clear);
        fireEvent.click(clear);
        expect(clear.getAttribute("aria-pressed")).toBe("false");
        expect(selectOf("Expiry month").disabled).toBe(false);
    });

    it("marks the default card with a switch that saves yes or nothing", async () => {
        mountNew("cards");
        type(field("Label"), "Visa");
        type(field("Card number"), "4111111111111111");
        const toggle = screen.getByRole("switch", { name: "Default card" });
        fireEvent.click(toggle);
        fireEvent.click(btn("Save to Vault"));
        await waitFor(() => expect(h.createEntry).toHaveBeenCalled());
        expect(h.createEntry.mock.calls[0][0].fields).toEqual({ default: "yes" });
    });

    it("doesn't show the older single expiry, the brand or the date srv works out", () => {
        mountEdit({
            kind: "card",
            section: "cards",
            parts_set: ["number", "expiry"],
            fields: { brand: "visa", expires: "2031-07-31" },
        });
        expect(rows().filter((r) => r === "Expiry")).toHaveLength(1);
        expect(rows()).not.toContain("Brand");
        expect(rows()).not.toContain("Expires");
    });
});
