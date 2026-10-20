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
const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "list") return { entries, problem: null, requests, backend: "keychain" };
    if (op === "create") return { id: "new-id" };
    if (op === "activity") return { activity: [] };
    if (op === "kept") return { kept: [] };
    return {};
});
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

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
        tail: "6789",
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
        // The first note is slow to answer: the next ones wait for it.
        vault.mockImplementationOnce(async (op: string) => {
            if (op === "list") return { entries, problem: null, requests, backend: "keychain" };
            return {};
        });
        renderVault();
        await waitFor(() => expect(calls("on_screen")).toHaveLength(1));
        vault.mockImplementationOnce(async () => {
            await held;
            return {};
        });
        setVisibility("dormant");
        setVisibility("active");
        await waitFor(() => expect(calls("on_screen").map((c) => c.on)).toEqual([true, false]));
        // The show waits until the hide has answered.
        await new Promise((r) => setTimeout(r, 20));
        expect(calls("on_screen").map((c) => c.on)).toEqual([true, false]);
        release();
        await waitFor(() => expect(calls("on_screen").map((c) => c.on)).toEqual([true, false, true]));
    });

    it("dismisses an agent's request through the host", async () => {
        requests = [{ id: "req-2", agent: "Agent1", kind: "ssn", label: "Their SSN", why: "", requested_ms: Date.now() }];
        renderVault();
        await screen.findByText("Their SSN");
        fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
        await waitFor(() => expect(calls("decline_request")).toEqual([{ request_id: "req-2" }]));
    });
});
