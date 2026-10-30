// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Vault pane: lists entries without values, saves and changes them
 *  through the host, fills an agent's request, and marks itself hidden from
 *  agents' captures (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5). */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";
import type { VaultRequest } from "@/types/rpc/VaultRequest";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

let entries: VaultEntryInfo[] = [];
let requests: VaultRequest[] = [];
let keptRows: Array<{ label: string; what: string; managed_in: string }> = [];
const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "list") return { entries, problem: null, requests, backend: "keychain" };
    if (op === "create") return { id: "new-id" };
    if (op === "activity") return { activity: [] };
    if (op === "kept") return { kept: keptRows };
    return {};
});
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

import { setPlatform } from "@/util/platformutil";
import { VaultViewModel } from "./vault-model";
import { VaultView } from "./vault-view";

const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});
const [visibility, setVisibility] = createSignal<"active" | "dormant">("active");
const setMeta = vi.fn((patch: Record<string, unknown>) => {
    setBlockMeta((prev) => ({ ...prev, ...patch }));
    return Promise.resolve();
});

function ssn(over: Partial<VaultEntryInfo> = {}): VaultEntryInfo {
    return {
        id: "ssn-alex",
        kind: "ssn",
        section: "personal",
        label: "Alex",
        fields: { owner: "Alex Doe" },
        tags: [],
        parts_set: ["value"],
        grants: [],
        tail: "6789",
        versions: 0,
        created_at: Date.now(),
        updated_at: Date.now(),
        last_used_at: 0,
        ...over,
    };
}

function renderVault() {
    return render(() => {
        const model = new VaultViewModel({ blockId: "vault-block", meta: blockMeta as any, setMeta, isFocused: () => false, visibility } as any);
        return <VaultView model={model} />;
    });
}

const calls = (op: string) => vault.mock.calls.filter((c) => c[0] === op).map((c) => c[1] ?? {});

describe("the Vault pane", () => {
    beforeEach(() => {
        entries = [ssn()];
        requests = [];
        vault.mockClear();
    });
    afterEach(() => {
        cleanup();
        setBlockMeta({});
        setVisibility("active");
        setPlatform("win32");
    });

    it("lists an entry by label and last characters, and marks itself hidden from agents", async () => {
        const { container } = renderVault();
        await screen.findByText("Alex");
        expect(screen.getByText("•••• 6789")).toBeInTheDocument();
        expect(screen.getByText("Alex Doe")).toBeInTheDocument();
        expect(container.querySelector('[data-agent-hidden="vault"]')).not.toBeNull();
        // Shown: srv is told, so agents' whole-window captures of AgentMux are refused.
        await waitFor(() => expect(calls("on_screen")[0]).toEqual({ pane: "vault-block", on: true }));
        const rail = screen.getByRole("tablist", { name: "Vault section" });
        expect(Array.from(rail.querySelectorAll('[role="tab"]')).map((t) => t.textContent)).toEqual([
            "Personal data",
            "Cards and banks",
            "API keys",
            "Logins",
            "Notes",
            "Kept by AgentMux",
            "Activity",
        ]);
    });

    it("saves a new entry through the host, and the typed value doesn't stay in the page", async () => {
        const { container } = renderVault();
        await screen.findByText("Alex");
        const personal = container.querySelector(".bundle-manager-pane:not(.is-hidden)") as HTMLElement;
        fireEvent.click(within(personal).getByRole("button", { name: /Add/ }));
        const dialog = await screen.findByRole("dialog");
        const [label] = within(dialog).getAllByRole("textbox");
        fireEvent.input(label, { target: { value: "Sam" } });
        const secret = dialog.querySelector('input[type="password"]') as HTMLInputElement;
        fireEvent.input(secret, { target: { value: "987-65-4321" } });
        fireEvent.click(within(dialog).getByRole("button", { name: "Save to Vault" }));
        await waitFor(() => expect(calls("create")).toHaveLength(1));
        expect(calls("create")[0]).toMatchObject({ kind: "ssn", label: "Sam", parts: { value: "987-65-4321" } });
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
        expect(container.innerHTML).not.toContain("987-65-4321");
    });

    it("changes an entry without sending the parts left empty", async () => {
        renderVault();
        await screen.findByText("Alex");
        fireEvent.click(screen.getByRole("button", { name: "Change" }));
        const dialog = await screen.findByRole("dialog");
        expect(within(dialog).getByText("Set. Type to replace it.")).toBeInTheDocument();
        const [label] = within(dialog).getAllByRole("textbox");
        fireEvent.input(label, { target: { value: "Alex (work)" } });
        fireEvent.click(within(dialog).getByRole("button", { name: "Save changes" }));
        await waitFor(() => expect(calls("update")).toHaveLength(1));
        expect(calls("update")[0]).toMatchObject({ id: "ssn-alex", label: "Alex (work)", parts: {}, clear: [] });
    });

    it("deletes after confirming", async () => {
        renderVault();
        await screen.findByText("Alex");
        fireEvent.click(screen.getByRole("button", { name: "Delete" }));
        const dialog = await screen.findByRole("dialog");
        fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
        await waitFor(() => expect(calls("delete")).toEqual([{ id: "ssn-alex" }]));
    });

    it("fills an agent's request with its id, on the request's kind", async () => {
        requests = [{ id: "req-1", agent: "Agent1", kind: "api-key", label: "Stripe test key", why: "run the payment tests", requested_ms: Date.now() }];
        renderVault();
        await screen.findByText("Stripe test key");
        fireEvent.click(screen.getByRole("button", { name: "Fill in" }));
        const dialog = await screen.findByRole("dialog");
        expect(within(dialog).getByText("Agent1 asks for: Stripe test key")).toBeInTheDocument();
        expect(within(dialog).getByText("Why: run the payment tests")).toBeInTheDocument();
        const secret = dialog.querySelector('input[type="password"]') as HTMLInputElement;
        fireEvent.input(secret, { target: { value: "sk_test_123456" } });
        fireEvent.click(within(dialog).getByRole("button", { name: "Save to Vault" }));
        await waitFor(() => expect(calls("create")).toHaveLength(1));
        expect(calls("create")[0]).toMatchObject({ kind: "api-key", label: "Stripe test key", request_id: "req-1" });
        expect(blockMeta()["vault:section"]).toBe("api-keys");
    });

    it("closes an open Show or copy dialog when the Vault stops being shown (its tab goes to the background)", async () => {
        renderVault();
        fireEvent.click(await screen.findByRole("button", { name: "Show or copy" }));
        expect(await screen.findByRole("dialog")).toBeTruthy();
        // Another tab coming forward makes the Vault's host dormant.
        setVisibility("dormant");
        await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    });

    it("tells srv when it is no longer shown", async () => {
        const { unmount } = renderVault();
        const on = { pane: "vault-block", on: true };
        const off = { pane: "vault-block", on: false };
        await waitFor(() => expect(calls("on_screen")).toEqual([on]));
        unmount();
        await waitFor(() => expect(calls("on_screen")).toEqual([on, off]));
    });

    it("sends a quick hide and show in order, one at a time", async () => {
        let release: () => void = () => {};
        const held = new Promise<void>((r) => (release = r));
        const base = vault.getMockImplementation();
        renderVault();
        await waitFor(() => expect(calls("on_screen")).toHaveLength(1));
        // The next note to srv (the hide) is slow to answer: the one after it waits for it. Reading the
        // list again when the Vault reappears is not a note, and isn't held up.
        let slow = true;
        vault.mockImplementation(async (op: string, args?: Record<string, unknown>) => {
            if (op === "on_screen" && slow) {
                slow = false;
                await held;
                return {};
            }
            return base ? base(op, args) : {};
        });
        setVisibility("dormant");
        setVisibility("active");
        await waitFor(() => expect(calls("on_screen").map((c) => c.on)).toEqual([true, false]));
        // The show waits until the hide has answered.
        await new Promise((r) => setTimeout(r, 20));
        expect(calls("on_screen").map((c) => c.on)).toEqual([true, false]);
        release();
        await waitFor(() => expect(calls("on_screen").map((c) => c.on)).toEqual([true, false, true]));
        vault.mockImplementation(base as never);
    });

    it("says what may ask the person until they dismiss it", async () => {
        setPlatform("darwin");
        renderVault();
        const note = await screen.findByRole("note", { name: "What the Vault may ask you" });
        expect(note.textContent).toContain("Always Allow");
        expect(note.textContent).toContain("never asks for administrator rights");
        fireEvent.click(within(note).getByRole("button", { name: "Got it" }));
        expect(blockMeta()["vault:prompts-read"]).toBe(true);
        await waitFor(() => expect(screen.queryByRole("note", { name: "What the Vault may ask you" })).toBeNull());
    });

    it("warns on macOS before the first save, which creates the key", async () => {
        setPlatform("darwin");
        entries = [];
        const { container } = renderVault();
        await waitFor(() => expect(screen.getAllByText(/Nothing here yet/).length).toBeGreaterThan(0));
        const personal = container.querySelector(".bundle-manager-pane:not(.is-hidden)") as HTMLElement;
        fireEvent.click(within(personal).getByRole("button", { name: /Add/ }));
        const dialog = await screen.findByRole("dialog");
        expect(within(dialog).getByText(/macOS asks whether AgentMux may use the keychain/)).toBeInTheDocument();
    });

    it("doesn't warn before a save once the Vault has entries", async () => {
        setPlatform("darwin");
        const { container } = renderVault();
        await screen.findByText("Alex");
        const personal = container.querySelector(".bundle-manager-pane:not(.is-hidden)") as HTMLElement;
        fireEvent.click(within(personal).getByRole("button", { name: /Add/ }));
        const dialog = await screen.findByRole("dialog");
        expect(within(dialog).queryByText(/macOS asks whether AgentMux may use the keychain/)).toBeNull();
    });

    it("lists the sign-ins browser panes saved, by site and masked user name, and says which ones it can't list", async () => {
        keptRows = [{ label: "https://intranet.example.com (Admin)", what: "sign-in for ab*** saved by a browser pane", managed_in: "browser" }];
        try {
            renderVault();
            await screen.findByText("Alex");
            fireEvent.click(screen.getByRole("tab", { name: "Kept by AgentMux" }));
            const row = (await screen.findByText("https://intranet.example.com (Admin)")).closest("tr") as HTMLElement;
            expect(row.textContent).toContain("sign-in for ab*** saved by a browser pane");
            expect(row.textContent).toContain("The browser pane that asked for it");
            expect(screen.getByText(/Only sign-ins a browser pane saves from now on are listed/)).toBeInTheDocument();
        } finally {
            keptRows = [];
        }
    });

    it("dismisses an agent's request through the host", async () => {
        requests = [{ id: "req-2", agent: "Agent1", kind: "ssn", label: "Their SSN", why: "", requested_ms: Date.now() }];
        renderVault();
        await screen.findByText("Their SSN");
        fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
        await waitFor(() => expect(calls("decline_request")).toEqual([{ request_id: "req-2" }]));
    });
});
