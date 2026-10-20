// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Export to and import from a `.vault` file: the passphrase and its meter, which
 *  entries go, the host picking the file, the passphrase prompt, the preview with
 *  no values, the duplicate choice, and all-or-nothing commit
 *  (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8). */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

const entry = (id: string, label: string, kind: string, section: VaultEntryInfo["section"]): VaultEntryInfo => ({
    id,
    kind,
    section,
    label,
    fields: {},
    tags: [],
    parts_set: ["value"],
    tail: "6789",
    created_at: 1,
    updated_at: 1,
    last_used_at: 0,
});
const ENTRIES = [entry("ssn-alex", "Alex", "ssn", "personal"), entry("api-key-stripe", "Stripe", "api-key", "api-keys")];

const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "list") return { entries: ENTRIES, problem: null, requests: [], backend: "keychain" };
    if (op === "suggest_passphrase") return { passphrase: "amber-birch-cedar-daisy-eagle-finch-grove-heron" };
    if (op === "import_commit") return { result: { added: 2, replaced: 0, skipped: 1 } };
    return {};
});
const vaultExport = vi.fn();
const vaultImport = vi.fn();
const vaultImportDelete = vi.fn();
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ approvals: { vault, vaultExport, vaultImport, vaultImportDelete } }) }));

import { MIN_PASSPHRASE, passphraseStrength, VaultExportDialog } from "./vault-export";
import { VaultImportDialog } from "./vault-import";
import { refreshVault } from "./vault-store";

const calls = (op: string) => vault.mock.calls.filter((c) => c[0] === op).map((c) => c[1] ?? {});
const type = (el: HTMLElement, value: string) => fireEvent.input(el, { target: { value } });

describe("the passphrase meter", () => {
    it("is short below the minimum, then weak, fair and strong", () => {
        expect(passphraseStrength("")).toEqual({ level: 0, label: "" });
        expect(passphraseStrength("abc")).toEqual({ level: 0, label: "Too short" });
        expect(passphraseStrength("abcdefgh").label).toBe("Weak");
        expect(passphraseStrength("abcdefghijkl1").level).toBe(2);
        expect(passphraseStrength("amber-birch-cedar-daisy-eagle-finch-grove-heron").label).toBe("Strong");
        expect(MIN_PASSPHRASE).toBe(8);
    });
});

describe("Export", () => {
    beforeEach(async () => {
        vault.mockClear();
        vaultExport.mockReset();
        await refreshVault();
    });
    afterEach(cleanup);

    const open = async () => {
        const onClose = vi.fn();
        render(() => <VaultExportDialog onClose={onClose} />);
        await screen.findByRole("dialog");
        return { onClose, dialog: () => screen.getByRole("dialog") };
    };
    const fill = (pass: string, again = pass) => {
        const [a, b] = Array.from(screen.getByRole("dialog").querySelectorAll('input[type="password"], input[type="text"]')) as HTMLInputElement[];
        type(a, pass);
        type(b, again);
    };
    const save = () => screen.getByRole("button", { name: /Choose where to save/ });

    it("lists every entry, selected, and needs a long enough passphrase typed twice", async () => {
        await open();
        const group = screen.getByRole("group", { name: "What to export" });
        expect(within(group).getByLabelText("Alex")).toBeChecked();
        expect(within(group).getByLabelText("Stripe")).toBeChecked();
        expect(save()).toBeDisabled();
        fill("short");
        expect(save()).toBeDisabled();
        fill("a long enough passphrase", "a different one entirely");
        expect(screen.getByText("The two don't match.")).toBeInTheDocument();
        expect(save()).toBeDisabled();
        fill("a long enough passphrase");
        expect(save()).not.toBeDisabled();
    });

    it("suggests a passphrase of words, shows it and fills both fields", async () => {
        await open();
        fireEvent.click(screen.getByRole("button", { name: /Suggest one/ }));
        await waitFor(() => expect(screen.getAllByDisplayValue("amber-birch-cedar-daisy-eagle-finch-grove-heron")).toHaveLength(2));
        expect(save()).not.toBeDisabled();
    });

    it("exports everything without naming entries, or only the chosen ones", async () => {
        vaultExport.mockResolvedValue({ count: 2, file_name: "agentmux-test.vault" });
        await open();
        fill("a long enough passphrase");
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(1));
        expect(vaultExport.mock.calls[0][0]).toMatchObject({ ids: undefined, passphrase: "a long enough passphrase" });
        expect(vaultExport.mock.calls[0][0].default_name).toMatch(/^agentmux-\d{4}-\d{2}-\d{2}$/);
        cleanup();

        vaultExport.mockResolvedValue({ count: 1, file_name: "some.vault" });
        await open();
        fireEvent.click(within(screen.getByRole("group", { name: "What to export" })).getByLabelText("Stripe"));
        fill("a long enough passphrase");
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(2));
        expect(vaultExport.mock.calls[1][0].ids).toEqual(["ssn-alex"]);
    });

    it("says where it saved, how to open it without AgentMux, and forgets the passphrase", async () => {
        vaultExport.mockResolvedValue({ count: 2, file_name: "agentmux-test.vault" });
        await open();
        fill("a long enough passphrase");
        fireEvent.click(save());
        const done = await screen.findByRole("status");
        expect(done.textContent).toContain("Saved 2 entries to agentmux-test.vault");
        expect(done.textContent).toContain("age -d agentmux-test.vault");
        expect(document.body.innerHTML).not.toContain("a long enough passphrase");
    });

    it("stays open, quietly, when the person closes the save dialog, and shows a real error", async () => {
        vaultExport.mockResolvedValueOnce({ cancelled: true });
        await open();
        fill("a long enough passphrase");
        fireEvent.click(save());
        await waitFor(() => expect(vaultExport).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(save()).not.toBeDisabled());
        expect(screen.queryByRole("alert")).toBeNull();
        vaultExport.mockRejectedValueOnce(new Error("couldn't save the file there: access denied"));
        fireEvent.click(save());
        expect((await screen.findByRole("alert")).textContent).toContain("access denied");
    });
});

describe("Import", () => {
    beforeEach(() => {
        vault.mockClear();
        vaultImport.mockReset();
    });
    afterEach(cleanup);

    const PREVIEW = {
        token: "tok-1",
        file_name: "backup.vault",
        total: 3,
        duplicates: 1,
        sections: [
            { section: "personal", count: 1, first: [{ label: "Alex", kind: "ssn", duplicate: true }] },
            { section: "api-keys", count: 2, first: [{ label: "Stripe", kind: "api-key", duplicate: false }, { label: "GitHub", kind: "api-key", duplicate: false }] },
        ],
        skipped: [{ label: "Odd one", reason: "unknown kind \"rocket\"" }],
        source: "vault",
        source_label: "an AgentMux Vault file",
        plaintext_file: false,
    };
    const PLAIN = { ...PREVIEW, file_name: "lastpass_export.csv", source: "last-pass", source_label: "LastPass", plaintext_file: true };

    it("asks the host for a file when it opens, and closes if the person closes that dialog", async () => {
        vaultImport.mockResolvedValue({ cancelled: true });
        const onClose = vi.fn();
        render(() => <VaultImportDialog onClose={onClose} />);
        await waitFor(() => expect(onClose).toHaveBeenCalled());
        expect(vaultImport).toHaveBeenCalledWith({ pick_id: undefined, passphrase: undefined });
    });

    it("asks for the passphrase, retries the same file by its id, and says when it's wrong", async () => {
        vaultImport.mockResolvedValueOnce({ needs_passphrase: true, pick_id: "pick-1", file_name: "backup.vault" });
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        const field = await screen.findByLabelText("Passphrase");
        vaultImport.mockRejectedValueOnce(new Error("that passphrase didn't open the file"));
        type(field, "wrong passphrase");
        fireEvent.click(screen.getByRole("button", { name: "Open file" }));
        expect((await screen.findByRole("alert")).textContent).toContain("didn't open the file");
        expect(vaultImport).toHaveBeenLastCalledWith({ pick_id: "pick-1", passphrase: "wrong passphrase" });
        expect(document.body.innerHTML).not.toContain("wrong passphrase");
    });

    it("previews counts, labels, duplicates and what can't come in, with no values, then imports", async () => {
        vaultImport.mockResolvedValueOnce({ preview: PREVIEW, pick_id: "pick-1", file_name: "backup.vault" });
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        const dialog = screen.getByRole("dialog");
        expect(dialog.textContent).toContain("Personal data");
        expect(dialog.textContent).toContain("Alex");
        expect(dialog.textContent).toContain("already in your Vault");
        expect(dialog.textContent).toContain("Stripe");
        expect(dialog.textContent).toContain("1 can't be imported");
        expect(dialog.textContent).toContain('unknown kind "rocket"');
        expect(calls("import_commit")).toHaveLength(0);

        fireEvent.click(within(dialog).getByRole("radio", { name: "Replace mine" }));
        fireEvent.click(within(dialog).getByRole("button", { name: "Import" }));
        await waitFor(() => expect(calls("import_commit")).toEqual([{ token: "tok-1", duplicates: "replace" }]));
        expect((await screen.findByRole("status")).textContent).toContain("2 added, 0 replaced, 1 skipped");
    });

    it("drops what srv holds when the person closes before importing, and not after", async () => {
        vaultImport.mockResolvedValue({ preview: PREVIEW, pick_id: "pick-1", file_name: "backup.vault" });
        const { unmount } = render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        unmount();
        await waitFor(() => expect(calls("import_cancel")).toEqual([{ token: "tok-1" }]));

        vault.mockClear();
        const again = render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import/);
        fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Import" }));
        await screen.findByRole("status");
        again.unmount();
        expect(calls("import_cancel")).toHaveLength(0);
    });

    it("says which service a file came from, and what the person should do with a plaintext one", async () => {
        vaultImport.mockResolvedValueOnce({ preview: PLAIN, pick_id: "pick-2", file_name: "lastpass_export.csv", synced_folder: "OneDrive" });
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import, from LastPass/);
        const dialog = screen.getByRole("dialog");
        fireEvent.click(within(dialog).getByRole("button", { name: "Import" }));
        const done = await screen.findByRole("status");
        expect(done.textContent).toContain("2 added");
        expect(dialog.textContent).toContain("holds every secret in it unencrypted");
        expect(dialog.textContent).toContain("It is in your OneDrive folder");
        expect(dialog.textContent).toContain("a copy may already be in the cloud");
        vaultImportDelete.mockResolvedValueOnce({ deleted: true });
        fireEvent.click(within(dialog).getByRole("button", { name: "Delete the file" }));
        await waitFor(() => expect(vaultImportDelete).toHaveBeenCalledWith({ pick_id: "pick-2" }));
        expect((await screen.findAllByRole("status")).map((s) => s.textContent).join(" ")).toContain("isn't securely erased");
    });

    it("offers no deletion for an encrypted Vault file", async () => {
        vaultImport.mockResolvedValueOnce({ preview: PREVIEW, pick_id: "pick-1", file_name: "backup.vault" });
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/to import, from an AgentMux Vault file/);
        fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Import" }));
        await screen.findByRole("status");
        expect(screen.queryByRole("button", { name: "Delete the file" })).toBeNull();
        expect(screen.getByRole("dialog").textContent).toContain("The file stays where it is");
    });

    it("asks which column is which for a CSV nothing knows, and reads it again with the choice", async () => {
        vaultImport.mockResolvedValueOnce({ needs_mapping: { columns: ["Service", "Secret", "Owner"], rows: 2 }, pick_id: "pick-3", file_name: "keys.csv" });
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        await screen.findByText(/doesn't know this file's format/);
        const dialog = screen.getByRole("dialog");
        expect(dialog.textContent).toContain("it has 2 rows");
        const selects = Array.from(dialog.querySelectorAll("select")) as HTMLSelectElement[];
        // Each row becomes / Label / Key.
        fireEvent.change(selects[1], { target: { value: "0" } });
        fireEvent.change(selects[2], { target: { value: "1" } });
        vaultImport.mockResolvedValueOnce({ preview: { ...PLAIN, source: "csv", source_label: "a CSV file" }, pick_id: "pick-3", file_name: "keys.csv" });
        fireEvent.click(within(dialog).getByRole("button", { name: "Continue" }));
        await waitFor(() => expect(vaultImport).toHaveBeenLastCalledWith({ pick_id: "pick-3", passphrase: undefined, mapping: { kind: "api-key", label: 0, value: 1 } }));
        await screen.findByText(/to import, from a CSV file/);
    });

    it("shows why a file can't be read", async () => {
        vaultImport.mockRejectedValue(new Error("this isn't a Vault file: it isn't an age-encrypted file"));
        render(() => <VaultImportDialog onClose={vi.fn()} />);
        expect((await screen.findByRole("alert")).textContent).toContain("isn't an age-encrypted file");
    });
});
