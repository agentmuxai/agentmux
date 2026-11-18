// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Several Vaults in the pane: the Vaults row (the dropdown, Change, Delete, New Vault), Move to and the sign-in
 *  Vault of each browser profile
 *  (docs/specs/SPEC_VAULT_SHARED_ACROSS_MACHINES_LAN_AND_CLOUD_2026_11_01.md §5). */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultInfo } from "@/types/rpc/VaultInfo";

const vaults: VaultInfo[] = [
    { id: "personal", name: "Personal", agents: { mode: "all" }, entries: 3 },
    { id: "vault-w", name: "Work", agents: { mode: "named", agents: ["builder", "agent2-6"] }, entries: 1 },
    { id: "vault-n", name: "Mine only", agents: { mode: "none" }, entries: 0 },
];
const moveEntry = vi.fn<(id: string, vaultId: string) => Promise<unknown>>();
const createVault = vi.fn<(name: string, agents: unknown) => Promise<unknown>>();
const updateVault = vi.fn<(id: string, name: string, agents: unknown) => Promise<unknown>>();
const deleteVault = vi.fn<(id: string) => Promise<unknown>>();
const setSigninVault = vi.fn<(profile: string, vaultId: string) => Promise<unknown>>();
vi.mock("./vault-store", () => ({
    vaultList: () => vaults,
    signinVaultOf: () => ({ "profile:p-1": "vault-w" }),
    moveEntry: (id: string, v: string) => moveEntry(id, v),
    createVault: (n: string, a: unknown) => createVault(n, a),
    updateVault: (i: string, n: string, a: unknown) => updateVault(i, n, a),
    deleteVault: (id: string) => deleteVault(id),
    setSigninVault: (p: string, v: string) => setSigninVault(p, v),
}));
vi.mock("@/app/view/browser/browser-profiles", () => ({
    ensureBrowserProfiles: () => {},
    browserProfiles: () => [
        { id: "p-1", name: "Work profile", color: "#112233", created_at: 1, agents_allowed: false },
        { id: "agent-x", name: "Agent X", color: "#445566", created_at: 1, agents_allowed: false, owner_agent: "x" },
    ],
}));

import { parseAgentIds } from "./vault-manage";
import { VaultPicker } from "./vault-picker";
import { VaultMoveDialog, agentsSummary } from "./vault-vaults";

const entry: VaultEntryInfo = {
    id: "api-key-openai",
    kind: "api-key",
    section: "api-keys",
    label: "OpenAI",
    vault_id: "personal",
    fields: {},
    tags: [],
    parts_set: ["value"],
    grants: [],
    tail: "9xQ2",
    versions: 0,
    created_at: 1,
    updated_at: 1,
    last_used_at: 0,
};

beforeEach(() => {
    for (const f of [moveEntry, createVault, updateVault, deleteVault, setSigninVault]) f.mockReset().mockResolvedValue({});
});
afterEach(cleanup);

describe("which agents a Vault is for", () => {
    it("says it in a few words", () => {
        expect(agentsSummary({ mode: "all" })).toBe("All agents");
        expect(agentsSummary({ mode: "none" })).toBe("No agents: only you");
        expect(agentsSummary({ mode: "named", agents: ["a", "b"] })).toBe("Only a, b");
        expect(agentsSummary({ mode: "named", agents: [] })).toBe("No agents: only you");
    });

    it("reads typed agent ids at commas and spaces", () => {
        expect(parseAgentIds(" agent2-6, builder ;qa\n")).toEqual(["agent2-6", "builder", "qa"]);
        expect(parseAgentIds("")).toEqual([]);
    });
});

describe("the Vaults row", () => {
    const row = () => document.querySelector(".vault-picker") as HTMLElement;

    it("offers every Vault with its count, and All, in one dropdown; nothing opens a dialog of Vaults", () => {
        const onChange = vi.fn();
        render(() => <VaultPicker value="" onChange={onChange} />);
        const select = screen.getByRole("combobox", { name: "Vault" }) as HTMLSelectElement;
        expect(Array.from(select.options).map((o) => o.textContent)).toEqual(["All Vaults", "Personal (3)", "Work (1)", "Mine only (0)"]);
        fireEvent.change(select, { target: { value: "vault-w" } });
        expect(onChange).toHaveBeenCalledWith("vault-w");
        expect(screen.queryByRole("button", { name: "Vaults" })).toBeNull();
        expect(screen.getByRole("button", { name: "New Vault" })).toBeTruthy();
    });

    it("says who the chosen Vault is for, with Change and Delete for it; Personal can't be deleted", () => {
        const [value, setValue] = createSignal("vault-w");
        render(() => <VaultPicker value={value()} onChange={setValue} />);
        expect(row().textContent).toContain("Only builder, agent2-6");
        expect(screen.getByRole("button", { name: "Change Work" })).toBeTruthy();
        expect(screen.getByRole("button", { name: "Delete Work" })).toBeTruthy();
        setValue("personal");
        expect(row().textContent).toContain("All agents");
        expect(screen.getByRole("button", { name: "Change Personal" })).toBeTruthy();
        expect(screen.queryByRole("button", { name: "Delete Personal" })).toBeNull();
        // All Vaults: there is no one Vault to change.
        setValue("");
        expect(screen.queryByRole("button", { name: /^Change / })).toBeNull();
    });

    it("takes the only Vault there is as the chosen one, with no All to choose", () => {
        const others = vaults.splice(1);
        try {
            render(() => <VaultPicker value="" onChange={() => {}} />);
            const select = screen.getByRole("combobox", { name: "Vault" }) as HTMLSelectElement;
            expect(Array.from(select.options).map((o) => o.textContent)).toEqual(["Personal (3)"]);
            expect(row().textContent).toContain("All agents");
            expect(screen.getByRole("button", { name: "Change Personal" })).toBeTruthy();
        } finally {
            vaults.push(...others);
        }
    });

    it("adds a Vault for named agents in the pane, and shows the new Vault", async () => {
        createVault.mockResolvedValue({ id: "vault-c", name: "Client", agents: { mode: "named", agents: ["builder", "qa"] }, entries: 0 });
        const onChange = vi.fn();
        render(() => <VaultPicker value="" onChange={onChange} />);
        fireEvent.click(screen.getByRole("button", { name: "New Vault" }));
        // A form in the pane, not a dialog.
        expect(screen.queryByRole("dialog")).toBeNull();
        expect(screen.getByRole("region", { name: "New Vault" })).toBeTruthy();
        fireEvent.input(screen.getByRole("textbox", { name: /Name/ }), { target: { value: "Client" } });
        fireEvent.click(screen.getByRole("radio", { name: "Named agents" }));
        fireEvent.input(screen.getByRole("textbox", { name: /Agent ids/ }), { target: { value: "builder, qa" } });
        fireEvent.click(screen.getByRole("button", { name: "Add Vault" }));
        await waitFor(() => expect(onChange).toHaveBeenCalledWith("vault-c"));
        expect(createVault).toHaveBeenCalledWith("Client", { mode: "named", agents: ["builder", "qa"] });
        expect(screen.queryByRole("region", { name: "New Vault" })).toBeNull();
    });

    it("changes the chosen Vault's name and agents, and Cancel closes the form", async () => {
        render(() => <VaultPicker value="vault-w" onChange={() => {}} />);
        fireEvent.click(screen.getByRole("button", { name: "Change Work" }));
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(screen.queryByRole("region", { name: "Change Work" })).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Change Work" }));
        fireEvent.click(screen.getByRole("radio", { name: "None" }));
        fireEvent.click(screen.getByRole("button", { name: "Save" }));
        await waitFor(() => expect(updateVault).toHaveBeenCalled());
        expect(updateVault).toHaveBeenCalledWith("vault-w", "Work", { mode: "none" });
        await waitFor(() => expect(screen.queryByRole("region", { name: "Change Work" })).toBeNull());
    });

    it("deletes the chosen Vault after a yes, and goes back to all of them", async () => {
        const onChange = vi.fn();
        render(() => <VaultPicker value="vault-n" onChange={onChange} />);
        fireEvent.click(screen.getByRole("button", { name: "Delete Mine only" }));
        fireEvent.click(await screen.findByRole("button", { name: "Delete" }));
        await waitFor(() => expect(deleteVault).toHaveBeenCalledWith("vault-n"));
        await waitFor(() => expect(onChange).toHaveBeenCalledWith(""));
    });

    it("says why a Vault that still holds entries can't be deleted", async () => {
        deleteVault.mockRejectedValue(new Error("Work still holds entries: move them to another Vault, or delete them, first"));
        const onChange = vi.fn();
        render(() => <VaultPicker value="vault-w" onChange={onChange} />);
        fireEvent.click(screen.getByRole("button", { name: "Delete Work" }));
        fireEvent.click(await screen.findByRole("button", { name: "Delete" }));
        expect((await screen.findAllByRole("alert")).map((a) => a.textContent).join(" ")).toContain("still holds entries");
        expect(deleteVault).toHaveBeenCalledWith("vault-w");
        expect(onChange).not.toHaveBeenCalled();
    });

    it("lets each of the person's browser profiles keep its sign-ins in a Vault of its own, in a Vault's form", async () => {
        render(() => <VaultPicker value="vault-w" onChange={() => {}} />);
        fireEvent.click(screen.getByRole("button", { name: "Change Work" }));
        const selects = (screen.getAllByRole("combobox") as HTMLSelectElement[]).filter((el) => el.getAttribute("aria-label")?.startsWith("Vault for "));
        expect(selects.map((el) => el.getAttribute("aria-label"))).toEqual(["Vault for Personal sign-ins", "Vault for Work profile sign-ins"]);
        expect(selects[1].value).toBe("vault-w");
        expect(selects[0].value).toBe("personal");
        fireEvent.change(selects[1], { target: { value: "vault-n" } });
        await waitFor(() => expect(setSigninVault).toHaveBeenCalledWith("profile:p-1", "vault-n"));
    });
});

describe("Move to", () => {
    it("offers the other Vaults, says who may use the entry there, and moves it", async () => {
        const onClose = vi.fn();
        render(() => <VaultMoveDialog entry={entry} onClose={onClose} />);
        const select = screen.getByRole("combobox", { name: "Move to" }) as HTMLSelectElement;
        expect(Array.from(select.options).map((o) => o.value)).toEqual(["vault-w", "vault-n"]);
        expect(document.body.textContent).toContain("Only builder, agent2-6");
        fireEvent.change(select, { target: { value: "vault-n" } });
        expect(document.body.textContent).toContain("No agents: only you");
        fireEvent.click(screen.getByRole("button", { name: "Move" }));
        await waitFor(() => expect(onClose).toHaveBeenCalled());
        expect(moveEntry).toHaveBeenCalledWith("api-key-openai", "vault-n");
    });

    it("says why a move failed and stays open", async () => {
        moveEntry.mockRejectedValue(new Error("no Vault vault-w"));
        const onClose = vi.fn();
        render(() => <VaultMoveDialog entry={entry} onClose={onClose} />);
        fireEvent.click(screen.getByRole("button", { name: "Move" }));
        expect((await screen.findByRole("alert")).textContent).toContain("no Vault vault-w");
        expect(onClose).not.toHaveBeenCalled();
    });
});
