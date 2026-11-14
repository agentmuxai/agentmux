// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The import dialog, path by path: what it says while the host's file window is open, how each step
 *  titles itself, the passphrase and key-file rows, the column choices for a CSV, the preview, where
 *  the entries go and what happens to duplicates, the result, deleting the file, and what it tells
 *  srv when it closes. Written ahead of splitting `VaultImportDialog`. */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VaultInfo } from "@/types/rpc/VaultInfo";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

const VAULTS: VaultInfo[] = [
    { id: "personal", name: "Personal", agents: { mode: "none" }, entries: 3 },
    { id: "work", name: "Work", agents: { mode: "none" }, entries: 1 },
];
let vaults: VaultInfo[] = [];
let commitImpl: () => Promise<unknown> = async () => ({ result: { added: 2, replaced: 0, skipped: 1 } });

const vault = vi.fn(async (op: string): Promise<unknown> => {
    if (op === "list") return { entries: [], vaults, problem: null, requests: [], backend: "keychain" };
    if (op === "import_commit") return commitImpl();
    return {};
});
const vaultImport = vi.fn();
const vaultImportDelete = vi.fn();
vi.mock("@/util/app-api", () => ({ getApi: () => ({ approvals: { vault, vaultImport, vaultImportDelete } }) }));

import { VaultImportDialog } from "./vault-import";
import { refreshVault } from "./vault-store";

const calls = (op: string) => (vault.mock.calls as unknown[][]).filter((c) => c[0] === op).map((c) => c[1] ?? {});
const type = (el: HTMLElement, value: string) => fireEvent.input(el, { target: { value } });
const btn = (name: string) => screen.getByRole("button", { name });
const dialog = () => screen.getByRole("dialog");

function gate<T>() {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

const PREVIEW = {
    token: "tok-1",
    file_name: "backup.vault",
    total: 3,
    duplicates: 0,
    sections: [
        { section: "personal", count: 1, first: [{ label: "Alex", kind: "ssn", duplicate: false }] },
        {
            section: "api-keys",
            count: 2,
            first: [
                { label: "Stripe", kind: "api-key", duplicate: false },
                { label: "GitHub", kind: "api-key", duplicate: false },
            ],
        },
    ],
    skipped: [] as { label: string; reason: string }[],
    source: "vault",
    source_label: "an AgentMux Vault file",
    plaintext_file: false,
    browser_signins: 0,
};
const previewStep = (over: Partial<typeof PREVIEW> = {}, extra: Record<string, unknown> = {}) => ({
    preview: { ...PREVIEW, ...over },
    pick_id: "pick-1",
    file_name: over.file_name ?? PREVIEW.file_name,
    ...extra,
});
const needsPass = (file_name: string, key_file_name?: string | null) => ({
    needs_passphrase: true,
    pick_id: "pick-1",
    file_name,
    key_file_name,
});
const needsMapping = (columns: string[], rows = 2, file_name = "keys.csv") => ({
    needs_mapping: { columns, rows },
    pick_id: "pick-3",
    file_name,
});

beforeEach(async () => {
    vaults = [];
    commitImpl = async () => ({ result: { added: 2, replaced: 0, skipped: 1 } });
    vault.mockClear();
    vaultImport.mockReset();
    vaultImportDelete.mockReset();
    await refreshVault();
    vault.mockClear();
});
afterEach(cleanup);

describe("while the host's file window is open", () => {
    it("says so, with a plain title, and Cancel works; the line goes once a file is read", async () => {
        const g = gate<unknown>();
        vaultImport.mockReturnValueOnce(g.promise);
        const onClose = vi.fn();
        render(() => <VaultImportDialog onClose={onClose} />);
        expect(screen.getByText("Import into the Vault")).toBeInTheDocument();
        expect(screen.getByText("Nothing is added until you press Import.")).toBeInTheDocument();
        expect(screen.getByText("Choose the file in the window that opened…")).toBeInTheDocument();
        expect(btn("Cancel")).not.toBeDisabled();
        expect(screen.queryByLabelText("Passphrase")).toBeNull();
        expect(screen.queryByRole("button", { name: "Import" })).toBeNull();
        g.resolve(previewStep());
        await screen.findByText(/to import/);
        expect(screen.queryByText("Choose the file in the window that opened…")).toBeNull();
        fireEvent.click(btn("Cancel"));
        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("does not show the line again while a file is being read for its passphrase", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("backup.vault"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        const field = await screen.findByLabelText("Passphrase");
        const g = gate<unknown>();
        vaultImport.mockReturnValueOnce(g.promise);
        type(field, "right");
        fireEvent.click(btn("Open file"));
        await waitFor(() => expect(btn("Open file")).toHaveAttribute("aria-busy", "true"));
        expect(screen.queryByText("Choose the file in the window that opened…")).toBeNull();
        g.resolve(previewStep());
        await screen.findByText(/to import/);
    });

    it("closes and shows no error when the host's window is closed, at any step", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("backup.vault"));
        const onClose = vi.fn();
        render(() => <VaultImportDialog onClose={onClose} />);
        const field = await screen.findByLabelText("Passphrase");
        vaultImport.mockResolvedValueOnce({ cancelled: true });
        type(field, "x");
        fireEvent.click(btn("Open file"));
        await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
        expect(screen.queryByRole("alert")).toBeNull();
    });
});

describe("the title", () => {
    it("names the file once it is read: the preview's own name, or the one the host gave a locked file or a CSV", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("locked.kdbx"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByLabelText("Passphrase");
        expect(screen.getByText("Import locked.kdbx")).toBeInTheDocument();
        cleanup();

        vaultImport.mockResolvedValueOnce(needsMapping(["A", "B"], 2, "keys.csv"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/doesn't know this file's format/);
        expect(screen.getByText("Import keys.csv")).toBeInTheDocument();
        cleanup();

        vaultImport.mockResolvedValueOnce(
            previewStep({ file_name: "from-preview.vault" }, { file_name: "ignored-by-title.vault" })
        );
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(screen.getByText("Import from-preview.vault")).toBeInTheDocument();
    });
});

describe("the passphrase step", () => {
    it("is a password field the person can submit with Enter, and it is cleared once sent", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("backup.vault"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        const field = (await screen.findByLabelText("Passphrase")) as HTMLInputElement;
        expect(field.type).toBe("password");
        vaultImport.mockRejectedValueOnce(new Error("that passphrase didn't open the file"));
        type(field, "secret words");
        const form = field.closest("form")!;
        expect(form.querySelector('input[type="submit"]')).not.toBeNull();
        expect(fireEvent.submit(form)).toBe(false);
        expect((await screen.findByRole("alert")).textContent).toContain("didn't open the file");
        expect(vaultImport).toHaveBeenLastCalledWith({ pick_id: "pick-1", passphrase: "secret words" });
        expect(field.value).toBe("");
        // The step stays, so a second try is possible.
        expect(btn("Open file")).toBeDisabled();
    });

    it("cannot be opened with nothing typed and no key file, and can with either", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("Passwords.kdbx"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        const field = await screen.findByLabelText("Passphrase");
        expect(btn("Open file")).toBeDisabled();
        type(field, "p");
        expect(btn("Open file")).not.toBeDisabled();
        type(field, "");
        expect(btn("Open file")).toBeDisabled();
        cleanup();

        vaultImport.mockResolvedValueOnce(needsPass("Passwords.kdbx", "my.keyx"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByLabelText("Passphrase");
        expect(btn("Open file")).not.toBeDisabled();
    });

    it("is busy while it reads the file, and the key-file buttons wait", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("Passwords.kdbx"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        const field = await screen.findByLabelText("Passphrase");
        expect(btn("Choose a key file…")).not.toBeDisabled();
        expect(btn("Open file")).not.toHaveAttribute("aria-busy");
        const g = gate<unknown>();
        vaultImport.mockReturnValueOnce(g.promise);
        type(field, "p");
        fireEvent.click(btn("Open file"));
        await waitFor(() => expect(btn("Open file")).toHaveAttribute("aria-busy", "true"));
        expect(btn("Choose a key file…")).toBeDisabled();
        g.resolve(needsPass("Passwords.kdbx"));
        await waitFor(() => expect(btn("Open file")).not.toHaveAttribute("aria-busy"));
    });

    it("asks for the key file with the file's id and nothing else, and removing it asks the same way", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("Passwords.kdbx"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByLabelText("Passphrase");
        vaultImport.mockResolvedValueOnce(needsPass("Passwords.kdbx", "my.keyx"));
        fireEvent.click(btn("Choose a key file…"));
        await screen.findByText("my.keyx");
        expect(vaultImport).toHaveBeenLastCalledWith({
            pick_id: "pick-1",
            passphrase: undefined,
            mapping: undefined,
            choose_key_file: true,
            clear_key_file: undefined,
        });
        expect(screen.getByText("my.keyx").className).toContain("warden-manager-mono");
        expect(dialog().querySelector(".vault-keyfile")!.textContent).toContain("my.keyx Remove");
        expect(screen.queryByRole("button", { name: "Choose a key file…" })).toBeNull();

        const g = gate<unknown>();
        vaultImport.mockReturnValueOnce(g.promise);
        fireEvent.click(btn("Remove"));
        await waitFor(() => expect(btn("Remove")).toBeDisabled());
        expect(vaultImport).toHaveBeenLastCalledWith({
            pick_id: "pick-1",
            passphrase: undefined,
            mapping: undefined,
            choose_key_file: undefined,
            clear_key_file: true,
        });
        g.resolve(needsPass("Passwords.kdbx", null));
        await screen.findByRole("button", { name: "Choose a key file…" });
    });

    it("shows a key-file error and keeps the step", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("Passwords.kdbx"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByLabelText("Passphrase");
        vaultImport.mockRejectedValueOnce("the key file could not be read");
        fireEvent.click(btn("Choose a key file…"));
        expect((await screen.findByRole("alert")).textContent).toBe("the key file could not be read");
        expect(btn("Choose a key file…")).not.toBeDisabled();
    });

    it("offers a key file only where the file could be a KeePass database", async () => {
        const cases: [string, boolean][] = [
            ["backup.vault.bak", true],
            ["notes.vaultx", true],
            ["a.json.txt", true],
            ["BACKUP.VAULT", false],
            ["Export.JSON", false],
            ["export.json", false],
            ["backup.vault", false],
        ];
        for (const [name, offered] of cases) {
            vaultImport.mockResolvedValueOnce(needsPass(name));
            render(() => <VaultImportDialog onClose={vi.fn()} />);
            await screen.findByLabelText("Passphrase");
            expect(screen.queryByText("A KeePass database can also need a key file.") !== null, name).toBe(offered);
            cleanup();
        }
    });
});

describe("the column choices for a CSV", () => {
    const selectsOf = () => Array.from(dialog().querySelectorAll("select")) as HTMLSelectElement[];
    const optionTexts = (s: HTMLSelectElement) => Array.from(s.options).map((o) => o.textContent);

    it("starts on an API key, the first column as its name and the second as its value, and offers the kinds a row can be", async () => {
        vaultImport.mockResolvedValueOnce(needsMapping(["Service", "Secret", "Owner"]));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/doesn't know this file's format/);
        expect(dialog().textContent).toContain("it has 2 rows");
        const [kind, label, value] = selectsOf();
        expect(selectsOf()).toHaveLength(3);
        expect(kind.value).toBe("api-key");
        expect(label.value).toBe("0");
        expect(value.value).toBe("1");
        const kinds = optionTexts(kind);
        expect(kinds).toContain("API key or token");
        expect(kinds).toContain("Login");
        expect(kinds).toContain("Secret note");
        expect(kinds).not.toContain("Payment card");
        expect(kinds.some((k) => /bank/i.test(k ?? ""))).toBe(false);
        expect(optionTexts(label)).toEqual(["Service", "Secret", "Owner"]);
        expect(screen.getByText("Label (its name)")).toBeInTheDocument();
        expect(screen.getByText("Key")).toBeInTheDocument();
        // The passphrase form belongs to its own step only.
        expect(screen.queryByLabelText("Passphrase")).toBeNull();
        expect(btn("Continue")).not.toBeDisabled();
    });

    it("names a column with no heading by its place, in every list", async () => {
        vaultImport.mockResolvedValueOnce(needsMapping(["Service", "", "Owner"]));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/doesn't know this file's format/);
        fireEvent.change(selectsOf()[0], { target: { value: "login" } });
        const [, label, value, site, user] = selectsOf();
        expect(optionTexts(label)).toEqual(["Service", "Column 2", "Owner"]);
        expect(optionTexts(value)).toEqual(["Service", "Column 2", "Owner"]);
        expect(optionTexts(site)).toEqual(["none", "Service", "Column 2", "Owner"]);
        expect(optionTexts(user)).toEqual(["none", "Service", "Column 2", "Owner"]);
    });

    it("calls the second column by what the kind keeps in it, and asks for a site and username only for a login", async () => {
        vaultImport.mockResolvedValueOnce(needsMapping(["A", "B", "C"]));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/doesn't know this file's format/);
        expect(screen.queryByText("Site (optional)")).toBeNull();
        expect(screen.queryByText("Username (optional)")).toBeNull();
        fireEvent.change(selectsOf()[0], { target: { value: "login" } });
        expect(screen.getByText("Password")).toBeInTheDocument();
        expect(screen.queryByText("Key")).toBeNull();
        expect(screen.getByText("Site (optional)")).toBeInTheDocument();
        expect(screen.getByText("Username (optional)")).toBeInTheDocument();
        expect(selectsOf()).toHaveLength(5);
        fireEvent.change(selectsOf()[0], { target: { value: "note" } });
        expect(screen.getByText("Text")).toBeInTheDocument();
        expect(screen.queryByText("Site (optional)")).toBeNull();
        expect(selectsOf()).toHaveLength(3);
    });

    it("sends the kind, the label and value columns as numbers, and no site or username it was not given", async () => {
        vaultImport.mockResolvedValueOnce(needsMapping(["A", "B", "C"]));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/doesn't know this file's format/);
        fireEvent.change(selectsOf()[0], { target: { value: "note" } });
        fireEvent.change(selectsOf()[1], { target: { value: "2" } });
        fireEvent.change(selectsOf()[2], { target: { value: "0" } });
        vaultImport.mockResolvedValueOnce(previewStep({ file_name: "keys.csv" }));
        fireEvent.click(btn("Continue"));
        await screen.findByText(/to import/);
        const sent = vaultImport.mock.lastCall![0];
        expect(sent).toEqual({
            pick_id: "pick-3",
            passphrase: undefined,
            mapping: { kind: "note", label: 2, value: 0 },
        });
        expect(Object.keys(sent.mapping)).toEqual(["kind", "label", "value"]);
    });

    it("sends a login's site and username columns each as itself, and column 0 counts", async () => {
        vaultImport.mockResolvedValueOnce(needsMapping(["Name", "Pass", "URL", "User"]));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/doesn't know this file's format/);
        fireEvent.change(selectsOf()[0], { target: { value: "login" } });
        fireEvent.change(selectsOf()[3], { target: { value: "2" } });
        fireEvent.change(selectsOf()[4], { target: { value: "0" } });
        vaultImport.mockResolvedValueOnce(previewStep());
        fireEvent.click(btn("Continue"));
        await screen.findByText(/to import/);
        expect(vaultImport.mock.lastCall![0].mapping).toEqual({
            kind: "login",
            label: 0,
            value: 1,
            site: 2,
            username: 0,
        });
    });

    it("sends the site alone, or the username alone, when only one is chosen", async () => {
        vaultImport.mockResolvedValueOnce(needsMapping(["Name", "Pass", "URL", "User"]));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/doesn't know this file's format/);
        fireEvent.change(selectsOf()[0], { target: { value: "login" } });
        fireEvent.change(selectsOf()[3], { target: { value: "3" } });
        vaultImport.mockRejectedValueOnce(new Error("not that"));
        fireEvent.click(btn("Continue"));
        await screen.findByRole("alert");
        let mapping = vaultImport.mock.lastCall![0].mapping;
        expect(mapping).toEqual({ kind: "login", label: 0, value: 1, site: 3 });
        expect("username" in mapping).toBe(false);

        fireEvent.change(selectsOf()[3], { target: { value: "" } });
        fireEvent.change(selectsOf()[4], { target: { value: "2" } });
        vaultImport.mockRejectedValueOnce(new Error("not that either"));
        fireEvent.click(btn("Continue"));
        await waitFor(() => expect(vaultImport).toHaveBeenCalledTimes(3));
        mapping = vaultImport.mock.lastCall![0].mapping;
        expect(mapping).toEqual({ kind: "login", label: 0, value: 1, username: 2 });
        expect("site" in mapping).toBe(false);
    });

    it("is busy while it reads the file again, and shows why it could not", async () => {
        vaultImport.mockResolvedValueOnce(needsMapping(["A", "B"]));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/doesn't know this file's format/);
        const g = gate<unknown>();
        vaultImport.mockReturnValueOnce(g.promise);
        fireEvent.click(btn("Continue"));
        await waitFor(() => expect(btn("Continue")).toHaveAttribute("aria-busy", "true"));
        g.reject(new Error("column 3 is empty"));
        expect((await screen.findByRole("alert")).textContent).toBe("column 3 is empty");
        expect(btn("Continue")).not.toHaveAttribute("aria-busy");
        expect(dialog().textContent).toContain("doesn't know this file's format");
    });
});

describe("the preview", () => {
    it("counts one entry in the singular, and says which service the file came from", async () => {
        vaultImport.mockResolvedValueOnce(previewStep({ total: 1, source_label: "LastPass" }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import, from LastPass\./);
        expect(dialog().textContent).toContain("1 entry to import");
        expect(screen.queryByLabelText("Passphrase")).toBeNull();
        cleanup();
        vaultImport.mockResolvedValueOnce(previewStep({ total: 2 }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(dialog().textContent).toContain("2 entries to import");
        cleanup();
        vaultImport.mockResolvedValueOnce(previewStep({ total: 0, sections: [] }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(dialog().textContent).toContain("0 entries to import");
    });

    it("lists each section with its count and what it holds, naming a kind it does not know as it is", async () => {
        vaultImport.mockResolvedValueOnce(
            previewStep({
                sections: [
                    {
                        section: "logins",
                        count: 2,
                        first: [
                            { label: "GitHub", kind: "login", duplicate: false },
                            { label: "Odd", kind: "rocket", duplicate: false },
                        ],
                    },
                ],
            })
        );
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        const section = dialog().querySelector(".vault-import-section")!;
        expect(within(section as HTMLElement).getByText("Logins")).toBeInTheDocument();
        expect(section.textContent).toContain("(2)");
        const items = Array.from(section.querySelectorAll("li")).map((li) => li.textContent);
        expect(items).toEqual(["GitHub Login", "Odd rocket"]);
    });

    it("says how many more there are only when the list shows fewer than the section holds", async () => {
        vaultImport.mockResolvedValueOnce(
            previewStep({
                sections: [
                    {
                        section: "api-keys",
                        count: 7,
                        first: [
                            { label: "A", kind: "api-key", duplicate: false },
                            { label: "B", kind: "api-key", duplicate: false },
                        ],
                    },
                    { section: "notes", count: 1, first: [{ label: "N", kind: "note", duplicate: false }] },
                    {
                        section: "logins",
                        count: 2,
                        first: [
                            { label: "L1", kind: "login", duplicate: false },
                            { label: "L2", kind: "login", duplicate: false },
                        ],
                    },
                ],
            })
        );
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        const more = Array.from(dialog().querySelectorAll("li")).filter((li) => /more$/.test(li.textContent ?? ""));
        expect(more.map((li) => li.textContent)).toEqual(["and 5 more"]);
    });

    it("marks the entries the Vault already has, and only those", async () => {
        vaultImport.mockResolvedValueOnce(
            previewStep({
                duplicates: 1,
                sections: [
                    {
                        section: "api-keys",
                        count: 2,
                        first: [
                            { label: "Stripe", kind: "api-key", duplicate: true },
                            { label: "GitHub", kind: "api-key", duplicate: false },
                        ],
                    },
                ],
            })
        );
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        const [stripe, github] = Array.from(dialog().querySelectorAll(".vault-import-section li"));
        expect(stripe.textContent).toContain("· already in your Vault");
        expect(github.textContent).not.toContain("already in your Vault");
    });

    it("lists what cannot be imported, with the reason, and nothing when all can", async () => {
        vaultImport.mockResolvedValueOnce(
            previewStep({ skipped: [{ label: "Odd one", reason: 'unknown kind "rocket"' }] })
        );
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(dialog().textContent).toContain("1 can't be imported");
        expect(dialog().querySelector(".vault-import-skipped li")!.textContent).toBe('Odd one unknown kind "rocket"');
        cleanup();
        vaultImport.mockResolvedValueOnce(previewStep({ skipped: [] }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(dialog().querySelector(".vault-import-skipped")).toBeNull();
        expect(dialog().textContent).not.toContain("can't be imported");
    });

    it("cannot import a file with nothing in it", async () => {
        vaultImport.mockResolvedValueOnce(previewStep({ total: 0, sections: [] }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(btn("Import")).toBeDisabled();
        cleanup();
        vaultImport.mockResolvedValueOnce(previewStep({ total: 1 }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(btn("Import")).not.toBeDisabled();
    });
});

describe("what to do with duplicates", () => {
    it("is not asked when there are none, and is asked with the count when there are", async () => {
        vaultImport.mockResolvedValueOnce(previewStep({ duplicates: 0 }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(screen.queryByRole("radio")).toBeNull();
        expect(dialog().textContent).not.toContain("already in your Vault");
        cleanup();

        vaultImport.mockResolvedValueOnce(previewStep({ duplicates: 1 }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(screen.getByText("1 already in your Vault")).toBeInTheDocument();
        expect(screen.getByText("The same value (or the same site and username, for a login).")).toBeInTheDocument();
        expect(screen.getAllByRole("radio").map((r) => r.getAttribute("aria-label") ?? r.textContent)).toEqual([
            "Skip them",
            "Replace mine",
            "Keep both",
        ]);
        cleanup();

        vaultImport.mockResolvedValueOnce(previewStep({ duplicates: 4 }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(screen.getByText("4 already in your Vault")).toBeInTheDocument();
    });

    it("skips them unless told otherwise, and sends whichever was chosen", async () => {
        for (const [choice, sent] of [
            [null, "skip"],
            ["Skip them", "skip"],
            ["Replace mine", "replace"],
            ["Keep both", "keep_both"],
        ] as const) {
            vault.mockClear();
            vaultImport.mockResolvedValueOnce(previewStep({ duplicates: 1 }));
            render(() => <VaultImportDialog onClose={vi.fn()} />);
            await screen.findByText(/to import/);
            if (choice) fireEvent.click(screen.getByRole("radio", { name: choice }));
            if (choice) expect(screen.getByRole("radio", { name: choice })).toBeChecked();
            fireEvent.click(btn("Import"));
            await waitFor(() => expect(calls("import_commit")).toHaveLength(1));
            expect(calls("import_commit")[0], String(choice)).toEqual({
                token: "tok-1",
                duplicates: sent,
                vault: "personal",
            });
            cleanup();
        }
    });

    it("asks which browser profile a browser export's logins belong to, and sends it", async () => {
        vaultImport.mockResolvedValueOnce(previewStep({ browser_signins: 2, source: "chrome", source_label: "a browser's password export (Chrome, Edge, Brave)" }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        const field = screen.getByLabelText("Browser profile for the sign-ins");
        expect((field as HTMLSelectElement).value).toBe("personal");
        expect(dialog().textContent).toContain("2 logins with a site go to this profile's sign-in Vault (Personal)");
        fireEvent.click(btn("Import"));
        await waitFor(() =>
            expect(calls("import_commit")).toEqual([{ token: "tok-1", duplicates: "skip", vault: "personal", profile: "personal" }])
        );
    });

    it("points at the Vault above only while that field is on screen", async () => {
        const preview = { browser_signins: 2, source: "chrome", source_label: "a browser's password export (Chrome, Edge, Brave)" };
        vaultImport.mockResolvedValueOnce(previewStep(preview));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(screen.queryByText("Import into")).toBeNull();
        expect(dialog().textContent).toContain("where its browser offers them.");
        expect(dialog().textContent).not.toContain("Vault above");
        cleanup();

        vaults = VAULTS;
        await refreshVault();
        vaultImport.mockResolvedValueOnce(previewStep(preview));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(dialog().textContent).toContain("whatever Vault is chosen above. The rest go to the Vault above.");
    });

    it("asks nothing about a profile for a file with no browser logins", async () => {
        vaultImport.mockResolvedValueOnce(previewStep());
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(screen.queryByLabelText("Browser profile for the sign-ins")).toBeNull();
    });

    it("sends the choice even where the question was not asked", async () => {
        vaultImport.mockResolvedValueOnce(previewStep({ duplicates: 0 }));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        fireEvent.click(btn("Import"));
        await waitFor(() =>
            expect(calls("import_commit")).toEqual([{ token: "tok-1", duplicates: "skip", vault: "personal" }])
        );
    });
});

describe("where the entries go", () => {
    const selectNamed = (name: string) => screen.getByRole("combobox", { name }) as HTMLSelectElement;

    it("asks only when there is more than one Vault, starting on Personal", async () => {
        vaultImport.mockResolvedValueOnce(previewStep());
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(screen.queryByText("Import into")).toBeNull();
        cleanup();

        vaults = VAULTS;
        await refreshVault();
        vaultImport.mockResolvedValueOnce(previewStep());
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        expect(screen.getByText("Import into")).toBeInTheDocument();
        expect(
            screen.getByText("New entries go to this Vault. One you replace stays in the Vault it is in.")
        ).toBeInTheDocument();
        expect(selectNamed("Import into").value).toBe("personal");
        expect(Array.from(selectNamed("Import into").options).map((o) => o.textContent)).toEqual(["Personal", "Work"]);
    });

    it("starts on the Vault it was opened from, and imports into the one picked", async () => {
        vaults = VAULTS;
        await refreshVault();
        vaultImport.mockResolvedValueOnce(previewStep());
        render(() => <VaultImportDialog onClose={vi.fn()} into="work" />);
        await screen.findByText(/to import/);
        expect(selectNamed("Import into").value).toBe("work");
        fireEvent.click(btn("Import"));
        await waitFor(() =>
            expect(calls("import_commit")).toEqual([{ token: "tok-1", duplicates: "skip", vault: "work" }])
        );
        cleanup();

        vault.mockClear();
        vaultImport.mockResolvedValueOnce(previewStep());
        render(() => <VaultImportDialog onClose={vi.fn()} into="work" />);
        await screen.findByText(/to import/);
        fireEvent.change(selectNamed("Import into"), { target: { value: "personal" } });
        fireEvent.click(btn("Import"));
        await waitFor(() =>
            expect(calls("import_commit")).toEqual([{ token: "tok-1", duplicates: "skip", vault: "personal" }])
        );
    });

    it("uses Personal when it was opened with no Vault or an empty one, and sends the Vault even when none is asked", async () => {
        for (const into of [undefined, ""]) {
            vault.mockClear();
            vaultImport.mockResolvedValueOnce(previewStep());
            render(() => <VaultImportDialog onClose={vi.fn()} into={into} />);
            await screen.findByText(/to import/);
            fireEvent.click(btn("Import"));
            await waitFor(() =>
                expect(calls("import_commit")).toEqual([{ token: "tok-1", duplicates: "skip", vault: "personal" }])
            );
            cleanup();
        }
    });

    it("imports into the Vault it was opened from even when only that one is shown", async () => {
        vaultImport.mockResolvedValueOnce(previewStep());
        render(() => <VaultImportDialog onClose={vi.fn()} into="work" />);
        await screen.findByText(/to import/);
        expect(screen.queryByText("Import into")).toBeNull();
        fireEvent.click(btn("Import"));
        await waitFor(() =>
            expect(calls("import_commit")).toEqual([{ token: "tok-1", duplicates: "skip", vault: "work" }])
        );
    });
});

describe("importing", () => {
    it("sends the preview's token, not an older one", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("backup.vault"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        const field = await screen.findByLabelText("Passphrase");
        vaultImport.mockResolvedValueOnce(previewStep({ token: "tok-after-unlock" }));
        type(field, "p");
        fireEvent.click(btn("Open file"));
        await screen.findByText(/to import/);
        fireEvent.click(btn("Import"));
        await waitFor(() =>
            expect(calls("import_commit")).toEqual([
                { token: "tok-after-unlock", duplicates: "skip", vault: "personal" },
            ])
        );
    });

    it("is busy while it writes, then shows the counts in the right order", async () => {
        const g = gate<unknown>();
        commitImpl = () => g.promise;
        vaultImport.mockResolvedValueOnce(previewStep());
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        fireEvent.click(btn("Import"));
        await waitFor(() => expect(btn("Import")).toHaveAttribute("aria-busy", "true"));
        expect(btn("Cancel")).not.toBeDisabled();
        g.resolve({ result: { added: 5, replaced: 3, skipped: 2 } });
        const done = await screen.findByRole("status");
        expect(done.textContent).toContain("Imported: 5 added, 3 replaced, 2 skipped.");
        expect(screen.queryByRole("button", { name: "Import" })).toBeNull();
        expect(screen.queryByText(/to import/)).toBeNull();
    });

    it("says why it could not, clears that on the next try, and can be tried again", async () => {
        let n = 0;
        commitImpl = async () => {
            if (n++ === 0) throw new Error("the Vault is locked");
            return { result: { added: 3, replaced: 0, skipped: 0 } };
        };
        vaultImport.mockResolvedValueOnce(previewStep());
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        fireEvent.click(btn("Import"));
        expect((await screen.findByRole("alert")).textContent).toBe("the Vault is locked");
        await waitFor(() => expect(btn("Import")).not.toHaveAttribute("aria-busy"));
        expect(btn("Import")).not.toBeDisabled();
        expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
        expect(screen.queryByRole("status")).toBeNull();

        const g = gate<unknown>();
        commitImpl = () => g.promise;
        fireEvent.click(btn("Import"));
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
        g.resolve({ result: { added: 3, replaced: 0, skipped: 0 } });
        expect((await screen.findByRole("status")).textContent).toContain("3 added");
    });

    it("clears a failed read's message when the next read works", async () => {
        vaultImport.mockResolvedValueOnce(needsPass("backup.vault"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        const field = await screen.findByLabelText("Passphrase");
        vaultImport.mockRejectedValueOnce(new Error("wrong passphrase"));
        type(field, "a");
        fireEvent.click(btn("Open file"));
        await screen.findByRole("alert");
        const g = gate<unknown>();
        vaultImport.mockReturnValueOnce(g.promise);
        type(field, "b");
        fireEvent.click(btn("Open file"));
        await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
        g.resolve(previewStep());
        await screen.findByText(/to import/);
        expect(screen.queryByRole("alert")).toBeNull();
    });

    it("shows a failure that is not an Error as its text", async () => {
        vaultImport.mockRejectedValue("plain text");
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        expect((await screen.findByRole("alert")).textContent).toBe("plain text");
        expect(screen.queryByText("Choose the file in the window that opened…")).toBeNull();
        expect(btn("Cancel")).not.toBeDisabled();
    });
});

describe("after the import", () => {
    const imported = async (preview: Partial<typeof PREVIEW> = {}, extra: Record<string, unknown> = {}) => {
        vaultImport.mockResolvedValueOnce(previewStep(preview, extra));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        fireEvent.click(btn("Import"));
        await screen.findByText(/Imported:/);
    };

    it("turns Cancel into Close, which closes", async () => {
        const onClose = vi.fn();
        vaultImport.mockResolvedValueOnce(previewStep());
        render(() => <VaultImportDialog onClose={onClose} />);
        await screen.findByText(/to import/);
        expect(screen.queryByRole("button", { name: "Close" })).toBeNull();
        fireEvent.click(btn("Import"));
        await screen.findByText(/Imported:/);
        expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
        expect(btn("Close")).not.toBeDisabled();
        fireEvent.click(btn("Close"));
        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("leaves an encrypted file alone and says so", async () => {
        await imported({ plaintext_file: false }, { synced_folder: "OneDrive" });
        expect(dialog().textContent).toContain("The file stays where it is. Delete it if you don't need it.");
        expect(dialog().textContent).not.toContain("unencrypted");
        expect(dialog().textContent).not.toContain("OneDrive");
        expect(screen.queryByRole("button", { name: "Delete the file" })).toBeNull();
    });

    it("warns about a plaintext file and offers to delete it, with no word about a cloud copy when it is not in a synced folder", async () => {
        await imported({ plaintext_file: true });
        expect(dialog().textContent).toContain("That file holds every secret in it unencrypted");
        expect(dialog().textContent).not.toContain("The file stays where it is");
        expect(dialog().textContent).not.toContain("a copy may already be in the cloud");
        expect(screen.queryByRole("alert")).toBeNull();
        expect(btn("Delete the file")).toBeInTheDocument();
    });

    it("says a browser's export is also still in the browser, and no other file does", async () => {
        await imported({ plaintext_file: true, source: "chrome", source_label: "a browser's password export (Chrome, Edge, Brave)" });
        expect(dialog().textContent).toContain("also still saved in the browser, and in Google Password Manager");
    });

    it("says nothing of the browser for a file from another source", async () => {
        await imported({ plaintext_file: true, source: "lastpass" });
        expect(dialog().textContent).not.toContain("Google Password Manager");
    });

    it("names the synced folder a plaintext file is in", async () => {
        await imported({ plaintext_file: true }, { synced_folder: "Dropbox" });
        expect(screen.getByRole("alert").textContent).toContain(
            "It is in your Dropbox folder: a copy may already be in the cloud"
        );
    });

    it("deletes the file by its id, then says it is gone and takes the button away", async () => {
        await imported({ plaintext_file: true });
        vaultImportDelete.mockResolvedValueOnce({ deleted: true });
        fireEvent.click(btn("Delete the file"));
        await waitFor(() => expect(vaultImportDelete).toHaveBeenCalledWith({ pick_id: "pick-1" }));
        expect(await screen.findByText(/^Deleted\./)).toBeInTheDocument();
        expect(screen.getByText(/isn't securely erased/)).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Delete the file" })).toBeNull();
    });

    it("keeps the button and says why when the file could not be deleted", async () => {
        await imported({ plaintext_file: true });
        vaultImportDelete.mockRejectedValueOnce(new Error("the file is open elsewhere"));
        fireEvent.click(btn("Delete the file"));
        expect((await screen.findByRole("alert")).textContent).toBe("the file is open elsewhere");
        expect(btn("Delete the file")).toBeInTheDocument();
        expect(screen.queryByText(/^Deleted\./)).toBeNull();
        vaultImportDelete.mockRejectedValueOnce("denied");
        fireEvent.click(btn("Delete the file"));
        await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("denied"));
    });
});

describe("closing", () => {
    it("does not close on a click outside it, and closes on Escape", async () => {
        vaultImport.mockResolvedValueOnce(previewStep());
        const onClose = vi.fn();
        render(() => <VaultImportDialog onClose={onClose} />);
        await screen.findByText(/to import/);
        fireEvent.click(document.querySelector(".modal-backdrop")!);
        expect(onClose).not.toHaveBeenCalled();
        fireEvent.keyDown(document.querySelector(".modal-panel")!, { key: "Escape" });
        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("tells srv to drop the file when closed before importing, with the latest token", async () => {
        vaultImport.mockResolvedValueOnce(previewStep({ token: "tok-a" }));
        const { unmount } = render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        unmount();
        await waitFor(() => expect(calls("import_cancel")).toEqual([{ token: "tok-a" }]));
    });

    it("tells srv nothing when no file was read, or when the file was still locked", async () => {
        const g = gate<unknown>();
        vaultImport.mockReturnValueOnce(g.promise);
        const first = render(() => <VaultImportDialog onClose={vi.fn()} />);
        first.unmount();
        vaultImport.mockResolvedValueOnce(needsPass("backup.vault"));
        const second = render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByLabelText("Passphrase");
        second.unmount();
        expect(calls("import_cancel")).toEqual([]);
    });

    it("still drops the file when the import itself failed", async () => {
        commitImpl = async () => {
            throw new Error("the Vault is locked");
        };
        vaultImport.mockResolvedValueOnce(previewStep({ token: "tok-b" }));
        const { unmount } = render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        fireEvent.click(btn("Import"));
        await screen.findByRole("alert");
        unmount();
        await waitFor(() => expect(calls("import_cancel")).toEqual([{ token: "tok-b" }]));
    });

    it("does not drop what it already imported", async () => {
        vaultImport.mockResolvedValueOnce(previewStep());
        const { unmount } = render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        fireEvent.click(btn("Import"));
        await screen.findByText(/Imported:/);
        unmount();
        expect(calls("import_cancel")).toEqual([]);
    });
});
