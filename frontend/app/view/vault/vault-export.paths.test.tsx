// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The export dialog, path by path: the passphrase meter at its edges, which entries are listed and
 *  chosen, the passphrase fields and when saving is allowed, the suggested passphrase, saving a
 *  `.vault` or KeePass file, the unencrypted CSV and JSON (the place first, then the word), what each
 *  way of closing tells the host, and what the finished dialog says. Written ahead of splitting
 *  `VaultExportDialog`. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

const entry = (id: string, label: string, kind: string, section: VaultEntryInfo["section"]): VaultEntryInfo => ({
    id,
    kind,
    section,
    label,
    vault_id: "personal",
    fields: {},
    tags: [],
    parts_set: ["value"],
    tail: "6789",
    versions: 0,
    created_at: 1,
    updated_at: 1,
    last_used_at: 0,
    grants: [],
});
const ENTRIES = [
    entry("ssn-alex", "Alex", "ssn", "personal"),
    entry("card-visa", "Visa", "card", "cards"),
    entry("api-key-stripe", "Stripe", "api-key", "api-keys"),
    entry("api-key-github", "GitHub", "api-key", "api-keys"),
    entry("login-site", "Example site", "login", "logins"),
    entry("note-n", "Recovery codes", "note", "notes"),
];
const ALL_IDS = ENTRIES.map((e) => e.id);
const PASS = "a long enough passphrase";
const SUGGESTED = "amber-birch-cedar-daisy-eagle-finch-grove-heron";

let entries: VaultEntryInfo[] = ENTRIES;
let suggestImpl: () => Promise<unknown> = async () => ({ passphrase: SUGGESTED });
const vault = vi.fn(async (op: string): Promise<unknown> => {
    if (op === "list") return { entries, vaults: [], problem: null, requests: [], backend: "keychain" };
    if (op === "suggest_passphrase") return suggestImpl();
    return {};
});
const vaultExport = vi.fn();
vi.mock("@/util/app-api", () => ({ getApi: () => ({ approvals: { vault, vaultExport } }) }));

import { MIN_PASSPHRASE, passphraseStrength, PLAIN_CONFIRM_WORD, VaultExportDialog } from "./vault-export";
import { refreshVault } from "./vault-store";

const DATE_NAME = /^agentmux-\d{4}-\d{2}-\d{2}$/;
const type = (el: HTMLElement, value: string) => fireEvent.input(el, { target: { value } });
const btn = (name: string | RegExp) => screen.getByRole("button", { name });
const dialog = () => screen.getByRole("dialog");
const group = () => screen.getByRole("group", { name: "What to export" });
const save = () => btn(/Choose where to save/);
const exportButton = () => btn(/Export unencrypted/);
const radio = (name: RegExp) =>
    within(screen.getByRole("radiogroup", { name: "File format" })).getByRole("radio", { name });
const fields = () =>
    Array.from(dialog().querySelectorAll('input[type="password"], input[type="text"]')) as HTMLInputElement[];
const fill = (pass: string, again = pass) => {
    const [a, b] = fields();
    type(a, pass);
    type(b, again);
};
const wordBox = () => screen.getByLabelText(/Type "export" to confirm/) as HTMLInputElement;
const sections = () =>
    Array.from(dialog().querySelectorAll(".vault-export-group > label:first-child")).map((l) => l.textContent);

function gate<T>() {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

const open = async () => {
    const onClose = vi.fn();
    render(() => <VaultExportDialog onClose={onClose} />);
    await screen.findByRole("dialog");
    return { onClose };
};

const PLACE = {
    pick_id: "place-1",
    file_name: "agentmux-test.csv",
    folder: "/home/a/Documents",
    synced_folder: null as string | null,
};
/** Chooses a plain format and a place for it. */
const chooseCsvPlace = async (format: RegExp = /CSV, unencrypted/, place: object = PLACE) => {
    vaultExport.mockResolvedValueOnce(place);
    fireEvent.click(radio(format));
    fireEvent.click(save());
    await screen.findByRole("group", { name: "Where it will be saved" });
};
const exportCalls = () => vaultExport.mock.calls.map((c) => c[0]);

beforeEach(async () => {
    entries = ENTRIES;
    suggestImpl = async () => ({ passphrase: SUGGESTED });
    vault.mockClear();
    vaultExport.mockReset();
    await refreshVault();
});
afterEach(cleanup);

describe("the passphrase meter", () => {
    it("has the minimum it says, and calls anything under it too short", () => {
        expect(MIN_PASSPHRASE).toBe(8);
        expect(PLAIN_CONFIRM_WORD).toBe("export");
        expect(passphraseStrength("")).toEqual({ level: 0, label: "" });
        expect(passphraseStrength("a")).toEqual({ level: 0, label: "Too short" });
        expect(passphraseStrength("abcdefg")).toEqual({ level: 0, label: "Too short" });
        expect(passphraseStrength("abcdefgh")).toEqual({ level: 1, label: "Weak" });
    });

    it("scores length first: weak below 16, fair below 24, strong from there", () => {
        expect(passphraseStrength("abcdefghijklmno")).toEqual({ level: 1, label: "Weak" });
        expect(passphraseStrength("abcdefghijklmnop")).toEqual({ level: 2, label: "Fair" });
        expect(passphraseStrength("abcdefghijklmnopqrstuvw")).toEqual({ level: 2, label: "Fair" });
        expect(passphraseStrength("abcdefghijklmnopqrstuvwx")).toEqual({ level: 3, label: "Strong" });
    });

    it("adds three for each kind of character past the first, and each of the four kinds counts", () => {
        // 14 lowercase letters score 14: weak. One more kind adds 3, so it reaches 16: fair.
        expect(passphraseStrength("abcdefghijklmn").level).toBe(1);
        expect(passphraseStrength("abcdefghijkl1").level).toBe(2);
        expect(passphraseStrength("abcdefghijk1").level).toBe(1);
        expect(passphraseStrength("ABCDEFGHIJKL1").level).toBe(2);
        expect(passphraseStrength("abcdefghijkl!").level).toBe(2);
        expect(passphraseStrength("abcdefghijkl1").label).toBe("Fair");
        // Each kind alone, at the length where a second kind would tip it.
        expect(passphraseStrength("abcdefghijklm").level).toBe(1);
        expect(passphraseStrength("ABCDEFGHIJKLM").level).toBe(1);
        expect(passphraseStrength("1234567890123").level).toBe(1);
        expect(passphraseStrength("!!!!!!!!!!!!!").level).toBe(1);
    });

    it("adds eight for four or more words, split by spaces or hyphens", () => {
        expect(passphraseStrength("aaa-bbb-ccc-ddd")).toEqual({ level: 3, label: "Strong" });
        expect(passphraseStrength("aaa bbb ccc ddd")).toEqual({ level: 3, label: "Strong" });
        expect(passphraseStrength("aaa-bbb-ccc")).toEqual({ level: 1, label: "Weak" });
        // Three words are not enough, even long ones.
        expect(passphraseStrength("aaaaa-bbbbbb-cccccc")).toEqual({ level: 2, label: "Fair" });
        // The bonus is exactly eight: 13 + 3 + 8 = 24 is strong, 12 + 3 + 8 = 23 is fair.
        expect(passphraseStrength("aa-bb-cc-dd-e").level).toBe(3);
        expect(passphraseStrength("aa-bb-cc-dd-").level).toBe(2);
        // A separator at the edge is not a word.
        expect(passphraseStrength("-aaa-bbb-ccc")).toEqual({ level: 1, label: "Weak" });
    });
});

describe("what it opens with", () => {
    it("says what it saves, offers four formats with the Vault file chosen, and cannot save yet", async () => {
        await open();
        expect(screen.getByText("Export the Vault")).toBeInTheDocument();
        expect(dialog().textContent).toContain(
            "Saves a password-protected backup. Anyone with the file and its passphrase can read every secret in it."
        );
        expect(radio(/AgentMux Vault file/)).toBeChecked();
        expect(radio(/KeePass database/)).not.toBeChecked();
        expect(radio(/CSV, unencrypted/)).not.toBeChecked();
        expect(radio(/JSON, unencrypted/)).not.toBeChecked();
        expect(dialog().textContent).toContain("(.vault, recommended: every kind, part and detail)");
        expect(dialog().textContent).toContain("(.kdbx, to open in KeePassXC, KeePassDX or Strongbox)");
        expect(dialog().textContent).toContain("(.csv, Bitwarden's layout, which most managers import)");
        expect(dialog().textContent).toContain("(.json, every kind and detail; AgentMux imports it back)");
        expect(btn("Cancel")).not.toBeDisabled();
        expect(save()).toBeDisabled();
        expect(save()).not.toHaveAttribute("aria-busy");
    });

    it("lists the sections in their order with what is in each, all chosen", async () => {
        await open();
        expect(sections()).toEqual([
            "Personal data (1)",
            "Cards and banks (1)",
            "API keys (2)",
            "Logins (1)",
            "Notes (1)",
        ]);
        const labels = Array.from(group().querySelectorAll(".vault-check-entry")).map((l) => l.textContent);
        expect(labels).toEqual(["Alex", "Visa", "Stripe", "GitHub", "Example site", "Recovery codes"]);
        for (const box of within(group()).getAllByRole("checkbox")) expect(box).toBeChecked();
        expect(within(group()).getAllByRole("checkbox")).toHaveLength(11);
        expect(screen.queryByText(/nothing to export/)).toBeNull();
    });

    it("says the Vault is empty when it is, and cannot save", async () => {
        entries = [];
        await refreshVault();
        await open();
        expect(screen.getByText("The Vault is empty: there is nothing to export.")).toBeInTheDocument();
        expect(within(group()).queryAllByRole("checkbox")).toHaveLength(0);
        fill(PASS);
        expect(save()).toBeDisabled();
    });
});

describe("choosing what to export", () => {
    it("unchecks and rechecks a whole section, and only that section", async () => {
        await open();
        const apiKeys = within(group()).getAllByRole("checkbox")[4];
        expect(apiKeys.parentElement!.textContent).toContain("API keys");
        fireEvent.click(apiKeys);
        expect(within(group()).getByLabelText("Stripe")).not.toBeChecked();
        expect(within(group()).getByLabelText("GitHub")).not.toBeChecked();
        expect(within(group()).getByLabelText("Alex")).toBeChecked();
        expect(within(group()).getByLabelText("Visa")).toBeChecked();
        expect(within(group()).getByLabelText("Example site")).toBeChecked();
        expect(apiKeys).not.toBeChecked();
        fireEvent.click(apiKeys);
        expect(within(group()).getByLabelText("Stripe")).toBeChecked();
        expect(within(group()).getByLabelText("GitHub")).toBeChecked();
        expect(apiKeys).toBeChecked();
    });

    it("shows a section as chosen only when all its entries are", async () => {
        await open();
        const apiKeys = within(group()).getAllByRole("checkbox")[4];
        fireEvent.click(within(group()).getByLabelText("Stripe"));
        expect(within(group()).getByLabelText("Stripe")).not.toBeChecked();
        expect(apiKeys).not.toBeChecked();
        expect(within(group()).getAllByRole("checkbox")[0]).toBeChecked();
        fireEvent.click(within(group()).getByLabelText("Stripe"));
        expect(apiKeys).toBeChecked();
    });

    it("cannot save with nothing chosen, with or without the passphrase", async () => {
        await open();
        fill(PASS);
        expect(save()).not.toBeDisabled();
        for (const box of within(group()).getAllByRole("checkbox").slice(0, 1)) fireEvent.click(box);
        for (const name of ["Visa", "Stripe", "GitHub", "Example site", "Recovery codes"])
            fireEvent.click(within(group()).getByLabelText(name));
        expect(
            within(group())
                .getAllByRole("checkbox")
                .filter((b) => (b as HTMLInputElement).checked)
        ).toHaveLength(0);
        expect(save()).toBeDisabled();
        fireEvent.click(within(group()).getByLabelText("Visa"));
        expect(save()).not.toBeDisabled();
    });

    it("names no entries when all are chosen, and otherwise sends the chosen ones in order", async () => {
        vaultExport.mockResolvedValue({ count: 7, file_name: "x.vault" });
        await open();
        fill(PASS);
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(1));
        expect(exportCalls()[0].ids).toBeUndefined();
        cleanup();

        await open();
        fireEvent.click(within(group()).getByLabelText("Stripe"));
        fill(PASS);
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(2));
        expect(exportCalls()[1].ids).toEqual(ALL_IDS.filter((id) => id !== "api-key-stripe"));
        cleanup();

        await open();
        fireEvent.click(within(group()).getAllByRole("checkbox")[4]);
        fill(PASS);
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(3));
        expect(exportCalls()[2].ids).toEqual(["ssn-alex", "card-visa", "login-site", "note-n"]);
    });
});

describe("the passphrase", () => {
    it("is hidden, asked for twice, and says its strength as it is typed", async () => {
        await open();
        const [pass, again] = fields();
        expect(pass.type).toBe("password");
        expect(again.type).toBe("password");
        expect(screen.getByText("At least 8 characters.")).toBeInTheDocument();
        const meter = dialog().querySelector("meter") as HTMLMeterElement;
        expect(meter.max).toBe(3);
        expect(meter.value).toBe(0);
        const steps: [string, string, number][] = [
            ["short", "Too short", 0],
            ["abcdefgh", "Weak", 1],
            ["abcdefghijklmnop", "Fair", 2],
            ["abcdefghijklmnopqrstuvwx", "Strong", 3],
        ];
        for (const [value, label, level] of steps) {
            type(fields()[0], value);
            expect(screen.getByText(`Strength: ${label}`)).toBeInTheDocument();
            expect(meter.value).toBe(level);
        }
        type(fields()[0], "");
        expect(screen.getByText("At least 8 characters.")).toBeInTheDocument();
        expect(meter.value).toBe(0);
    });

    it("says the two don't match only when something is typed in the second and they differ", async () => {
        await open();
        const [pass, again] = fields();
        type(pass, PASS);
        expect(screen.queryByText("The two don't match.")).toBeNull();
        type(again, PASS.slice(0, 5));
        expect(screen.getByText("The two don't match.")).toBeInTheDocument();
        type(again, PASS);
        expect(screen.queryByText("The two don't match.")).toBeNull();
    });

    it("is typed into the field it belongs to, and shown or hidden in both together", async () => {
        await open();
        type(fields()[0], "first one");
        type(fields()[1], "second");
        expect(fields()[0].value).toBe("first one");
        expect(fields()[1].value).toBe("second");
        const show = screen.getByLabelText("Show the passphrase") as HTMLInputElement;
        expect(show).not.toBeChecked();
        fireEvent.click(show);
        expect(show).toBeChecked();
        expect(fields().map((f) => f.type)).toEqual(["text", "text"]);
        fireEvent.click(show);
        expect(show).not.toBeChecked();
        expect(fields().map((f) => f.type)).toEqual(["password", "password"]);
    });

    it("can save only from the minimum length up, and only when typed twice the same", async () => {
        await open();
        fill("abcdefg");
        expect(save()).toBeDisabled();
        fill("abcdefgh");
        expect(save()).not.toBeDisabled();
        fill("abcdefgh", "abcdefgi");
        expect(save()).toBeDisabled();
        fill("abcdefgh", "");
        expect(save()).toBeDisabled();
        fill("abcdefgh", "abcdefgh");
        expect(save()).not.toBeDisabled();
    });

    it("keeps what was typed when the format changes and comes back", async () => {
        await open();
        fill(PASS);
        fireEvent.click(radio(/CSV, unencrypted/));
        expect(screen.queryByText("Show the passphrase")).toBeNull();
        fireEvent.click(radio(/AgentMux Vault file/));
        expect(fields().map((f) => f.value)).toEqual([PASS, PASS]);
    });
});

describe("a suggested passphrase", () => {
    it("fills both fields, shows them, and can then be saved", async () => {
        await open();
        fireEvent.click(btn(/Suggest one/));
        await waitFor(() => expect(fields().map((f) => f.value)).toEqual([SUGGESTED, SUGGESTED]));
        expect(fields().map((f) => f.type)).toEqual(["text", "text"]);
        expect(screen.getByLabelText("Show the passphrase")).toBeChecked();
        expect(screen.getByText("Strength: Strong")).toBeInTheDocument();
        expect(save()).not.toBeDisabled();
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("says why when none can be made, leaves the fields alone, and forgets the complaint on the next try", async () => {
        suggestImpl = async () => {
            throw new Error("no words today");
        };
        await open();
        type(fields()[0], "mine");
        fireEvent.click(btn(/Suggest one/));
        expect((await screen.findByRole("alert")).textContent).toBe("no words today");
        expect(fields()[0].value).toBe("mine");
        expect(fields()[1].value).toBe("");
        expect(fields()[0].type).toBe("password");
        suggestImpl = async () => {
            throw "plain text";
        };
        fireEvent.click(btn(/Suggest one/));
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("plain text"));
        suggestImpl = async () => ({ passphrase: SUGGESTED });
        fireEvent.click(btn(/Suggest one/));
        await waitFor(() => expect(fields()[0].value).toBe(SUGGESTED));
        expect(screen.queryByRole("alert")).toBeNull();
    });
});

describe("saving an encrypted file", () => {
    it("asks the host with the chosen entries, the passphrase, a dated name and the format", async () => {
        vaultExport.mockResolvedValue({ count: 7, file_name: "agentmux-test.vault" });
        await open();
        fill(PASS);
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(1));
        expect(exportCalls()[0]).toEqual({
            ids: undefined,
            passphrase: PASS,
            default_name: expect.stringMatching(DATE_NAME),
            format: "vault",
        });
        cleanup();

        await open();
        fireEvent.click(radio(/KeePass database/));
        fill(PASS);
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(2));
        expect(exportCalls()[1]).toEqual({
            ids: undefined,
            passphrase: PASS,
            default_name: expect.stringMatching(DATE_NAME),
            format: "kdbx",
        });
    });

    it("is busy while the host works, and nothing else can be pressed", async () => {
        const g = gate<unknown>();
        vaultExport.mockReturnValueOnce(g.promise);
        await open();
        fill(PASS);
        fireEvent.click(save());
        await waitFor(() => expect(save()).toHaveAttribute("aria-busy", "true"));
        expect(btn("Cancel")).toBeDisabled();
        g.resolve({ count: 7, file_name: "agentmux-test.vault" });
        await screen.findByRole("status");
        expect(btn("Close")).not.toBeDisabled();
    });

    it("stays as it was, quietly, when the person closes the save window, passphrase and all", async () => {
        vaultExport.mockResolvedValueOnce({ cancelled: true });
        await open();
        fill(PASS);
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(save()).not.toHaveAttribute("aria-busy"));
        expect(save()).not.toBeDisabled();
        expect(screen.queryByRole("alert")).toBeNull();
        expect(screen.queryByRole("status")).toBeNull();
        expect(fields().map((f) => f.value)).toEqual([PASS, PASS]);
    });

    it("shows why it failed, keeps the passphrase, forgets the complaint on the next try, and can be tried again", async () => {
        vaultExport.mockRejectedValueOnce(new Error("couldn't save the file there: access denied"));
        await open();
        fill(PASS);
        fireEvent.click(save());
        expect((await screen.findByRole("alert")).textContent).toBe("couldn't save the file there: access denied");
        expect(save()).not.toHaveAttribute("aria-busy");
        expect(save()).not.toBeDisabled();
        expect(fields().map((f) => f.value)).toEqual([PASS, PASS]);
        const g = gate<unknown>();
        vaultExport.mockReturnValueOnce(g.promise);
        fireEvent.click(save());
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
        g.reject("disk full");
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("disk full"));
    });

    it("says what was saved, once for one entry and in the plural for more, and forgets the passphrase", async () => {
        vaultExport.mockResolvedValueOnce({ count: 1, file_name: "one.vault" });
        await open();
        fill(PASS);
        fireEvent.click(save());
        let done = await screen.findByRole("status");
        expect(done.textContent).toContain("Saved 1 entry to one.vault.");
        expect(document.body.innerHTML).not.toContain(PASS);
        expect(screen.queryByRole("radiogroup")).toBeNull();
        expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
        cleanup();

        vaultExport.mockResolvedValueOnce({ count: 0, file_name: "none.vault" });
        await open();
        fill(PASS);
        fireEvent.click(save());
        done = await screen.findByRole("status");
        expect(done.textContent).toContain("Saved 0 entries to none.vault.");
        cleanup();

        vaultExport.mockResolvedValueOnce({ count: 3, file_name: "three.vault" });
        await open();
        fill(PASS);
        fireEvent.click(save());
        expect((await screen.findByRole("status")).textContent).toContain("Saved 3 entries to three.vault.");
    });

    it("tells a Vault file's owner how to open it without AgentMux, and a KeePass file's how to open that", async () => {
        vaultExport.mockResolvedValueOnce({ count: 2, file_name: "backup.vault" });
        await open();
        fill(PASS);
        fireEvent.click(save());
        let done = await screen.findByRole("status");
        expect(done.textContent).toContain(
            "Keep the passphrase somewhere safe: nothing can recover the file without it."
        );
        expect(done.textContent).toContain("age -d backup.vault");
        expect(done.textContent).toContain("opens it without AgentMux");
        expect(done.textContent).not.toContain("KeePass");
        expect(screen.queryByRole("alert")).toBeNull();
        fireEvent.click(btn("Close"));
        cleanup();

        vaultExport.mockResolvedValueOnce({ count: 2, file_name: "backup.kdbx" });
        await open();
        fireEvent.click(radio(/KeePass database/));
        fill(PASS);
        fireEvent.click(save());
        done = await screen.findByRole("status");
        expect(done.textContent).toContain("Open it in KeePassXC, KeePassDX or Strongbox with that password.");
        expect(done.textContent).toContain("It is a KDBX 4.1 file, so a KeePass app from before 2022 can't open it.");
        expect(done.textContent).not.toContain("age -d");
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("turns Cancel into Close, which closes", async () => {
        vaultExport.mockResolvedValueOnce({ count: 2, file_name: "backup.vault" });
        const { onClose } = await open();
        fill(PASS);
        fireEvent.click(save());
        await screen.findByRole("status");
        expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
        expect(screen.queryByRole("button", { name: /Choose where to save|Export unencrypted/ })).toBeNull();
        fireEvent.click(btn("Close"));
        expect(onClose).toHaveBeenCalledTimes(1);
        expect(vaultExport).toHaveBeenCalledTimes(1);
    });
});

describe("the format", () => {
    it("checks only the one picked, whichever it is", async () => {
        await open();
        const names = [/AgentMux Vault file/, /KeePass database/, /CSV, unencrypted/, /JSON, unencrypted/];
        // The Vault file starts chosen, so go to it last: clicking the one already chosen changes nothing.
        for (const i of [1, 2, 3, 0]) {
            fireEvent.click(radio(names[i]));
            names.forEach((other, j) =>
                j === i ? expect(radio(other)).toBeChecked() : expect(radio(other)).not.toBeChecked()
            );
        }
    });

    it("drops the passphrase for an unencrypted file and says so, and asks for it again for an encrypted one", async () => {
        await open();
        for (const name of [/CSV, unencrypted/, /JSON, unencrypted/]) {
            fireEvent.click(radio(name));
            expect(screen.queryByText("Passphrase")).toBeNull();
            expect(screen.queryByText("Type it again")).toBeNull();
            expect(dialog().textContent).toContain(
                "Saves a file with every secret in it, unencrypted, for another password manager. Anyone who can open the file can read them."
            );
            expect(dialog().textContent).not.toContain("Saves a password-protected backup");
            expect(save()).not.toBeDisabled();
        }
        fireEvent.click(radio(/KeePass database/));
        expect(screen.getByText("Passphrase")).toBeInTheDocument();
        expect(dialog().textContent).toContain("Saves a password-protected backup");
        expect(save()).toBeDisabled();
    });

    it("needs something chosen to export even without a passphrase", async () => {
        await open();
        fireEvent.click(radio(/CSV, unencrypted/));
        for (const box of within(group()).getAllByRole("checkbox"))
            if ((box as HTMLInputElement).checked) fireEvent.click(box);
        expect(save()).toBeDisabled();
        fireEvent.click(within(group()).getByLabelText("Alex"));
        expect(save()).not.toBeDisabled();
    });
});

describe("an unencrypted file", () => {
    it("asks the host where, in the format picked, with a dated name, before it writes anything", async () => {
        await open();
        await chooseCsvPlace();
        expect(vaultExport).toHaveBeenCalledTimes(1);
        expect(exportCalls()[0]).toEqual({ format: "csv", default_name: expect.stringMatching(DATE_NAME) });
        cleanup();

        vaultExport.mockClear();
        await open();
        await chooseCsvPlace(/JSON, unencrypted/, { ...PLACE, file_name: "agentmux-test.json" });
        expect(exportCalls()[0]).toEqual({ format: "json", default_name: expect.stringMatching(DATE_NAME) });
    });

    it("says where it will be saved, and asks for the word before the export button works", async () => {
        await open();
        await chooseCsvPlace();
        const where = screen.getByRole("group", { name: "Where it will be saved" });
        expect(where.textContent).toContain("It will be saved as agentmux-test.csv in /home/a/Documents.");
        expect(within(where).queryByRole("alert")).toBeNull();
        expect(
            screen.getByText("Windows or macOS then asks you to confirm it is you, as it does for Show and Copy.")
        ).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: /Choose where to save/ })).toBeNull();
        expect(exportButton()).toBeDisabled();
        for (const wrong of ["", "expor", "exports", "export it"]) {
            type(wordBox(), wrong);
            expect(exportButton(), wrong).toBeDisabled();
        }
        for (const right of ["export", "Export", " EXPORT "]) {
            type(wordBox(), right);
            expect(exportButton(), right).not.toBeDisabled();
        }
    });

    it("writes with the place's id and the chosen entries, in the format picked, and says what it wrote", async () => {
        await open();
        await chooseCsvPlace();
        vaultExport.mockResolvedValueOnce({ count: 7, file_name: "agentmux-test.csv" });
        type(wordBox(), "export");
        fireEvent.click(exportButton());
        await screen.findByRole("status");
        expect(exportCalls()[1]).toEqual({ format: "csv", pick_id: "place-1", ids: undefined, confirmed: true });
        cleanup();

        vaultExport.mockReset();
        await open();
        fireEvent.click(within(group()).getByLabelText("Stripe"));
        await chooseCsvPlace(/JSON, unencrypted/, { ...PLACE, pick_id: "place-2", file_name: "agentmux-test.json" });
        vaultExport.mockResolvedValueOnce({ count: 6, file_name: "agentmux-test.json" });
        type(wordBox(), "export");
        fireEvent.click(exportButton());
        await screen.findByRole("status");
        expect(exportCalls()[1]).toEqual({
            format: "json",
            pick_id: "place-2",
            ids: ALL_IDS.filter((id) => id !== "api-key-stripe"),
            confirmed: true,
        });
    });

    it("is busy while it writes, and cannot be closed meanwhile", async () => {
        await open();
        await chooseCsvPlace();
        const g = gate<unknown>();
        vaultExport.mockReturnValueOnce(g.promise);
        type(wordBox(), "export");
        fireEvent.click(exportButton());
        await waitFor(() => expect(exportButton()).toHaveAttribute("aria-busy", "true"));
        expect(btn("Cancel")).toBeDisabled();
        g.resolve({ count: 7, file_name: "agentmux-test.csv" });
        await screen.findByRole("status");
    });

    it("warns that a copy may reach the cloud when the folder is synced, and not otherwise", async () => {
        await open();
        await chooseCsvPlace(/CSV, unencrypted/, { ...PLACE, synced_folder: "OneDrive" });
        const where = screen.getByRole("group", { name: "Where it will be saved" });
        expect(within(where).getByRole("alert").textContent).toContain(
            "That folder is synced by OneDrive: a copy may reach the cloud"
        );
    });

    it("lets the person choose another place, which is asked for again in the same format", async () => {
        await open();
        await chooseCsvPlace();
        vaultExport.mockResolvedValueOnce({ ...PLACE, pick_id: "place-2", file_name: "elsewhere.csv", folder: "/tmp" });
        fireEvent.click(btn("Choose another place"));
        await waitFor(() =>
            expect(screen.getByRole("group", { name: "Where it will be saved" }).textContent).toContain(
                "elsewhere.csv in /tmp"
            )
        );
        expect(exportCalls()[1]).toEqual({ format: "csv", default_name: expect.stringMatching(DATE_NAME) });
        const g = gate<unknown>();
        vaultExport.mockReturnValueOnce(g.promise);
        fireEvent.click(btn("Choose another place"));
        await waitFor(() => expect(btn("Choose another place")).toBeDisabled());
        // Closing that window leaves no place chosen: the person is asked to choose again.
        g.resolve({ cancelled: true });
        await screen.findByRole("button", { name: /Choose where to save/ });
        expect(screen.queryByRole("group", { name: "Where it will be saved" })).toBeNull();
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("quietly stays on the format when the person closes the place window", async () => {
        vaultExport.mockResolvedValueOnce({ cancelled: true });
        await open();
        fireEvent.click(radio(/CSV, unencrypted/));
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(save()).not.toHaveAttribute("aria-busy"));
        expect(save()).not.toBeDisabled();
        expect(screen.queryByRole("alert")).toBeNull();
        expect(screen.queryByRole("group", { name: "Where it will be saved" })).toBeNull();
    });

    it("shows why a place could not be chosen, and forgets that on the next try", async () => {
        vaultExport.mockRejectedValueOnce(new Error("needs this computer to confirm it is you first"));
        await open();
        fireEvent.click(radio(/CSV, unencrypted/));
        fireEvent.click(save());
        expect((await screen.findByRole("alert")).textContent).toBe("needs this computer to confirm it is you first");
        expect(save()).not.toHaveAttribute("aria-busy");
        expect(save()).not.toBeDisabled();
        const g = gate<unknown>();
        vaultExport.mockReturnValueOnce(g.promise);
        fireEvent.click(save());
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
        g.reject("denied");
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("denied"));
    });

    it("keeps the place and the word when the write fails, forgets the complaint on the next try, and can write again", async () => {
        await open();
        await chooseCsvPlace();
        type(wordBox(), "export");
        vaultExport.mockRejectedValueOnce(new Error("the OS did not confirm it was you"));
        fireEvent.click(exportButton());
        expect((await screen.findByRole("alert")).textContent).toBe("the OS did not confirm it was you");
        expect(exportButton()).not.toHaveAttribute("aria-busy");
        expect(exportButton()).not.toBeDisabled();
        expect(wordBox().value).toBe("export");
        expect(screen.getByRole("group", { name: "Where it will be saved" })).toBeInTheDocument();
        const g = gate<unknown>();
        vaultExport.mockReturnValueOnce(g.promise);
        fireEvent.click(exportButton());
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
        g.reject("again");
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("again"));
    });

    it("forgets the place and the word once written, so closing afterwards tells the host nothing", async () => {
        const { onClose } = await open();
        await chooseCsvPlace();
        vaultExport.mockResolvedValueOnce({ count: 2, file_name: "agentmux-test.csv" });
        type(wordBox(), "export");
        fireEvent.click(exportButton());
        await screen.findByRole("status");
        expect(screen.queryByRole("group", { name: "Where it will be saved" })).toBeNull();
        expect(screen.queryByLabelText(/Type "export"/)).toBeNull();
        expect(screen.queryByRole("button", { name: /Choose where to save|Export unencrypted/ })).toBeNull();
        expect(vaultExport).toHaveBeenCalledTimes(2);
        fireEvent.click(btn("Close"));
        expect(onClose).toHaveBeenCalledTimes(1);
        expect(vaultExport).toHaveBeenCalledTimes(2);
    });

    it("cannot be written once everything is unchecked, even with the place chosen and the word typed", async () => {
        await open();
        await chooseCsvPlace();
        type(wordBox(), "export");
        expect(exportButton()).not.toBeDisabled();
        for (const box of within(group()).getAllByRole("checkbox"))
            if ((box as HTMLInputElement).checked) fireEvent.click(box);
        expect(exportButton()).toBeDisabled();
        fireEvent.click(within(group()).getByLabelText("Alex"));
        expect(exportButton()).not.toBeDisabled();
    });

    it("says an unencrypted file is, with a word about spreadsheets only for a CSV, and nothing about a passphrase", async () => {
        await open();
        await chooseCsvPlace();
        vaultExport.mockResolvedValueOnce({ count: 1, file_name: "agentmux-test.csv" });
        type(wordBox(), "export");
        fireEvent.click(exportButton());
        const done = await screen.findByRole("status");
        expect(done.textContent).toContain("Saved 1 entry to agentmux-test.csv.");
        expect(done.querySelector('[role="alert"]')!.textContent).toContain(
            "This file isn't encrypted. Import it into the other manager, then delete it, and empty the trash or recycle bin."
        );
        expect(done.textContent).toContain("Don't open it in a spreadsheet and save it");
        expect(done.textContent).not.toContain("passphrase");
        expect(done.textContent).not.toContain("age -d");
        expect(done.textContent).not.toContain("KeePass");
        cleanup();

        vaultExport.mockReset();
        await open();
        await chooseCsvPlace(/JSON, unencrypted/, { ...PLACE, file_name: "agentmux-test.json" });
        vaultExport.mockResolvedValueOnce({ count: 2, file_name: "agentmux-test.json" });
        type(wordBox(), "export");
        fireEvent.click(exportButton());
        const json = await screen.findByRole("status");
        expect(json.textContent).toContain("Saved 2 entries to agentmux-test.json.");
        expect(json.querySelector('[role="alert"]')!.textContent).toContain("This file isn't encrypted.");
        expect(json.textContent).not.toContain("spreadsheet");
        expect(json.textContent).not.toContain("passphrase");
    });
});

describe("changing format and closing", () => {
    it("tells the host to forget a place when the format changes, naming the format it was for, and starts clean", async () => {
        await open();
        await chooseCsvPlace();
        type(wordBox(), "ex");
        vaultExport.mockResolvedValue({});
        fireEvent.click(radio(/JSON, unencrypted/));
        expect(exportCalls().at(-1)).toEqual({ format: "csv", cancel: true });
        expect(screen.queryByRole("group", { name: "Where it will be saved" })).toBeNull();
        expect(save()).toBeInTheDocument();
        vaultExport.mockResolvedValueOnce(PLACE);
        fireEvent.click(save());
        await screen.findByRole("group", { name: "Where it will be saved" });
        expect(wordBox().value).toBe("");
    });

    it("tells the host nothing when no place was chosen, or the format was not an unencrypted one", async () => {
        await open();
        fireEvent.click(radio(/CSV, unencrypted/));
        fireEvent.click(radio(/JSON, unencrypted/));
        fireEvent.click(radio(/KeePass database/));
        fireEvent.click(radio(/CSV, unencrypted/));
        expect(vaultExport).not.toHaveBeenCalled();
        cleanup();

        await open();
        await chooseCsvPlace();
        vaultExport.mockResolvedValue({});
        vaultExport.mockClear();
        fireEvent.click(radio(/AgentMux Vault file/));
        expect(exportCalls()).toEqual([{ format: "csv", cancel: true }]);
        vaultExport.mockClear();
        fireEvent.click(radio(/KeePass database/));
        expect(vaultExport).not.toHaveBeenCalled();
    });

    it("closes straight away from an encrypted format, or an unencrypted one with no place chosen", async () => {
        const first = await open();
        fireEvent.click(btn("Cancel"));
        expect(first.onClose).toHaveBeenCalledTimes(1);
        cleanup();

        const second = await open();
        fireEvent.click(radio(/CSV, unencrypted/));
        fireEvent.click(btn("Cancel"));
        expect(second.onClose).toHaveBeenCalledTimes(1);
        expect(vaultExport).not.toHaveBeenCalled();
    });

    it("tells the host to forget the place when closed with one chosen, by Cancel or by Escape, and still closes", async () => {
        const first = await open();
        await chooseCsvPlace();
        vaultExport.mockResolvedValue({});
        fireEvent.click(btn("Cancel"));
        expect(first.onClose).toHaveBeenCalledTimes(1);
        expect(exportCalls().at(-1)).toEqual({ format: "csv", cancel: true });
        cleanup();

        vaultExport.mockReset();
        const second = await open();
        await chooseCsvPlace(/JSON, unencrypted/, { ...PLACE, file_name: "agentmux-test.json" });
        vaultExport.mockResolvedValue({});
        fireEvent.keyDown(document.querySelector(".modal-panel")!, { key: "Escape" });
        expect(second.onClose).toHaveBeenCalledTimes(1);
        expect(exportCalls().at(-1)).toEqual({ format: "json", cancel: true });
    });

    it("does not close on a click outside it", async () => {
        const { onClose } = await open();
        fireEvent.click(document.querySelector(".modal-backdrop")!);
        expect(onClose).not.toHaveBeenCalled();
    });
});
