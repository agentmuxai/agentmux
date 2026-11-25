// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The strip of changes by agents (docs/specs/SPEC_VAULT_AGENT_ADDS_AND_EDITS_WITHOUT_ASKING_2026_11_25.md §5): a count that
// hides at zero, rows with proposals and replaced values first, Undo, Seen, Accept and Dismiss, the agent's words as text,
// and nothing that opens a modal.

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentChange } from "@/types/rpc/AgentChange";

const [changes, setChanges] = createSignal<AgentChange[]>([]);
const markChangeSeen = vi.fn<(seq: number) => Promise<unknown>>();
const markAllChangesSeen = vi.fn<() => Promise<unknown>>();
const undoChange = vi.fn<(seq: number) => Promise<unknown>>();
const acceptProposal = vi.fn<(id: string, field: string) => Promise<unknown>>();
const dismissProposal = vi.fn<(id: string, field: string) => Promise<unknown>>();
vi.mock("./vault-store", () => ({
    vaultChanges: () => changes(),
    markChangeSeen: (seq: number) => markChangeSeen(seq),
    markAllChangesSeen: () => markAllChangesSeen(),
    undoChange: (seq: number) => undoChange(seq),
    acceptProposal: (id: string, field: string) => acceptProposal(id, field),
    dismissProposal: (id: string, field: string) => dismissProposal(id, field),
}));

import { markOf, stripText, VaultChangesStrip } from "./vault-changes";

const change = (over: Partial<AgentChange>): AgentChange => ({
    seq: 1,
    at: Date.now() - 120_000,
    agent: "builder",
    entry_id: "api-key-mail",
    entry_label: "Mail key",
    kind: "added",
    action: "imported",
    target: "",
    reason: "copy the mail key for task 31030",
    seen: false,
    undo: "remove",
    replaces_yours: false,
    ...over,
});

const proposal = change({
    seq: 3,
    entry_id: "login-intranet",
    entry_label: "Intranet",
    kind: "proposal",
    action: "proposed a change",
    target: "field site",
    undo: "",
    proposal: { field: "site", value: "https://intranet2.example.com", current: "https://intranet.example.com" },
});
const replaced = change({ seq: 2, kind: "new value", action: "captured", entry_label: "GitHub", entry_id: "api-key-github", undo: "restore value", replaces_yours: true });
const added = change({ seq: 1 });

describe("Changes by agents", () => {
    beforeEach(() => {
        for (const f of [markChangeSeen, markAllChangesSeen, undoChange, acceptProposal, dismissProposal]) f.mockReset().mockResolvedValue({});
        setChanges([]);
    });
    afterEach(cleanup);

    it("shows nothing at all while there is nothing to look at", () => {
        render(() => <VaultChangesStrip />);
        expect(document.querySelector(".vault-changes")).toBeNull();
    });

    it("says how many changes wait, and opens the list behind Review, with no modal", async () => {
        setChanges([added]);
        render(() => <VaultChangesStrip />);
        expect(screen.getByRole("status").textContent).toContain("1 change by an agent since you last looked");
        expect(document.querySelector(".vault-change")).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Review" }));
        const row = await waitFor(() => document.querySelector<HTMLElement>('[data-seq="1"]')!);
        expect(row.textContent).toContain("builder added Mail key");
        expect(document.querySelector('[role="dialog"], [aria-modal="true"]')).toBeNull();
        expect(stripText(3)).toBe("3 changes by agents since you last looked.");
    });

    it("lists a proposal and a replaced value of an entry you made first, each saying what it would do", async () => {
        setChanges([proposal, replaced, added]);
        render(() => <VaultChangesStrip />);
        fireEvent.click(screen.getByRole("button", { name: "Review" }));
        const rows = await waitFor(() => {
            const all = [...document.querySelectorAll<HTMLElement>(".vault-change")];
            if (all.length !== 3) throw new Error("not yet");
            return all;
        });
        expect(rows.map((r) => r.dataset.seq)).toEqual(["3", "2", "1"]);
        expect(rows[0].textContent).toContain("wants to change the site of Intranet to https://intranet2.example.com");
        expect(rows[0].textContent).toContain("keeps working at https://intranet.example.com until you accept");
        expect(rows[1].textContent).toContain("replaced the value of GitHub, an entry you made");
        expect(rows[0].hasAttribute("data-first") && rows[1].hasAttribute("data-first") && !rows[2].hasAttribute("data-first")).toBe(true);
    });

    it("gives a proposal Accept and Dismiss and no Undo, and the others Undo and Seen", async () => {
        setChanges([proposal, replaced]);
        render(() => <VaultChangesStrip />);
        fireEvent.click(screen.getByRole("button", { name: "Review" }));
        const [p, r] = await waitFor(() => {
            const all = [...document.querySelectorAll<HTMLElement>(".vault-change")];
            if (all.length !== 2) throw new Error("not yet");
            return all;
        });
        const names = (row: HTMLElement) => [...row.querySelectorAll("button")].map((b) => b.textContent);
        expect(names(p)).toEqual(["Accept", "Dismiss"]);
        expect(names(r)).toEqual(["Undo", "Seen"]);
        fireEvent.click(screen.getByRole("button", { name: "Accept" }));
        await waitFor(() => expect(acceptProposal).toHaveBeenCalledWith("login-intranet", "site"));
        await waitFor(() => expect((screen.getByRole("button", { name: "Dismiss" }) as HTMLButtonElement).disabled).toBe(false));
        fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
        await waitFor(() => expect(dismissProposal).toHaveBeenCalledWith("login-intranet", "site"));
        await waitFor(() => expect((screen.getByRole("button", { name: "Undo" }) as HTMLButtonElement).disabled).toBe(false));
        fireEvent.click(screen.getByRole("button", { name: "Undo" }));
        await waitFor(() => expect(undoChange).toHaveBeenCalledWith(2));
        await waitFor(() => expect((screen.getByRole("button", { name: "Seen" }) as HTMLButtonElement).disabled).toBe(false));
        fireEvent.click(screen.getByRole("button", { name: "Seen" }));
        await waitFor(() => expect(markChangeSeen).toHaveBeenCalledWith(2));
    });

    it("confirms the removal of an added entry in its row before it is made", async () => {
        setChanges([added]);
        render(() => <VaultChangesStrip />);
        fireEvent.click(screen.getByRole("button", { name: "Review" }));
        fireEvent.click(await screen.findByRole("button", { name: "Undo" }));
        expect(undoChange).not.toHaveBeenCalled();
        expect(screen.getByRole("group", { name: /Remove Mail key\?/ })).toBeTruthy();
        fireEvent.click(screen.getByRole("button", { name: "Keep it" }));
        expect(undoChange).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Undo" }));
        fireEvent.click(screen.getByRole("button", { name: "Remove it" }));
        await waitFor(() => expect(undoChange).toHaveBeenCalledWith(1));
    });

    it("shows an agent's reason as text, quoted, whatever markup it holds", async () => {
        setChanges([change({ reason: '<img src=x onerror="alert(1)"> **bold** [click](https://evil.example)' })]);
        render(() => <VaultChangesStrip />);
        fireEvent.click(screen.getByRole("button", { name: "Review" }));
        const quoted = await waitFor(() => document.querySelector<HTMLElement>(".vault-change__reason q")!);
        expect(quoted.textContent).toBe('<img src=x onerror="alert(1)"> **bold** [click](https://evil.example)');
        expect(document.querySelector(".vault-change img, .vault-change a, .vault-change strong")).toBeNull();
    });

    it("Seen all asks once and says why when it fails", async () => {
        setChanges([added, replaced]);
        markAllChangesSeen.mockRejectedValue(new Error("the Vault is busy"));
        render(() => <VaultChangesStrip />);
        fireEvent.click(screen.getByRole("button", { name: "Review" }));
        fireEvent.click(await screen.findByRole("button", { name: "Seen all" }));
        expect((await screen.findByRole("alert")).textContent).toContain("the Vault is busy");
        expect(markAllChangesSeen).toHaveBeenCalledTimes(1);
    });

    it("marks an entry's row new, or changed by the agent, until it is seen", () => {
        expect(markOf("api-key-mail", [added])).toBe("new");
        expect(markOf("api-key-github", [replaced])).toBe("changed by builder");
        expect(markOf("api-key-other", [added, replaced])).toBeNull();
        expect(markOf("login-intranet", [proposal])).toBeNull();
    });
});
