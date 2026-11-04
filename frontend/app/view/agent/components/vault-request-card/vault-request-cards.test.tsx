// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An agent's Vault requests as cards in its own pane
 *  (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4, §14):
 *  drawn from srv's record in the pane that asked, answered through the host,
 *  gone when srv's list drops them, never taking the composer's focus. */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/notification/waiting-for-you", () => ({ startWaitingForYou: vi.fn(), endWaitingForYou: vi.fn() }));
vi.mock("@/app/store/global", async (orig) => ({ ...(await orig<object>()), MOS: { getMuxObjectAtom: () => () => ({ oid: "pane" }) } }));

let uses: UseRequest[] = [];
let requests: VaultRequest[] = [];
const vault = vi.fn(async (op: string, _args?: Record<string, unknown>): Promise<unknown> => {
    if (op === "list") return { entries: [], problem: null, requests, uses, backend: "keychain" };
    return {};
});
vi.mock("@/util/app-api", () => ({ getApi: () => ({ approvals: { vault } }) }));

import { __resetPaneRequests, paneRequests } from "@/app/store/pane-requests";
import { PaneRequestDock } from "@/app/view/agent/pane-requests/PaneRequestDock";
import { refreshVault } from "@/app/view/vault/vault-store";
import { askedInPane } from "@/app/view/vault/vault-use-prompts";
import { startVaultRequestCards } from "./vault-request-cards";

const PANE = "agent-pane-1";
const use = (over: Partial<UseRequest> = {}): UseRequest => ({
    id: "use-1",
    agent: "Builder",
    block_id: PANE,
    entry_id: "api-key-github",
    entry_label: "GitHub token",
    what: "value",
    origin: "",
    field: "GH_TOKEN (value)",
    offer_always: true,
    reason: "it is the first time Builder runs a command with this entry",
    needs_value: false,
    kind: "api-key",
    command: "gh release create v1",
    cwd: "C:/work",
    replaces: false,
    signin: false,
    requested_ms: Date.now(),
    deadline_ms: Date.now() + 30_000,
    expires_ms: Date.now() + 120_000,
    goes_ahead: true,
    ...over,
});
const request = (over: Partial<VaultRequest> = {}): VaultRequest => ({
    id: "req-1",
    agent: "Builder",
    kind: "api-key",
    label: "Postgres URL",
    why: "run the migrations",
    requested_ms: Date.now(),
    block_id: PANE,
    ...over,
});
const calls = (op: string) => vault.mock.calls.filter((c) => c[0] === op).map((c) => c[1] ?? {});

// The cards follow srv's lists for the life of the window, as in the app.
startVaultRequestCards();

describe("an agent's Vault requests as cards in its pane", () => {
    beforeEach(() => vault.mockClear());
    afterEach(async () => {
        cleanup();
        uses = [];
        requests = [];
        await refreshVault();
        __resetPaneRequests();
    });

    it("shows a use in the pane that asked, with srv's countdown, and nowhere else", async () => {
        uses = [use(), use({ id: "use-2", block_id: "another-pane", entry_label: "Stripe key" })];
        await refreshVault();
        render(() => <PaneRequestDock blockId={PANE} />);
        const dock = await screen.findByTestId("pane-request-dock");
        expect(dock.hasAttribute("data-operator-only")).toBe(true);
        expect(dock.textContent).toContain("Builder wants to run a command with your value");
        expect(dock.textContent).toContain("gh release create v1");
        expect(screen.getByRole("timer").textContent).toMatch(/Goes ahead in 0:(29|30) unless you deny/);
        expect(dock.textContent).not.toContain("Stripe key");
        expect(paneRequests.of("another-pane")).toHaveLength(1);
    });

    it("answers through the host, and goes when srv's list drops it", async () => {
        uses = [use()];
        await refreshVault();
        render(() => <PaneRequestDock blockId={PANE} />);
        fireEvent.click(await screen.findByRole("button", { name: "Deny" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "deny" }]));
        uses = [];
        await refreshVault();
        await waitFor(() => expect(screen.queryByTestId("pane-request-dock")).toBeNull());
    });

    it("says an ID entry waits for a yes, with no countdown", async () => {
        uses = [use({ kind: "ssn", entry_label: "Alex", what: "Social Security number", goes_ahead: false, command: "", origin: "https://irs.gov", field: "SSN" })];
        await refreshVault();
        render(() => <PaneRequestDock blockId={PANE} />);
        const dock = await screen.findByTestId("pane-request-dock");
        expect(dock.textContent).toContain("ID and card details are never used without a yes");
        expect(screen.queryByRole("timer")).toBeNull();
    });

    it("never takes the focus from the composer, even for a value the person types", async () => {
        render(() => (
            <>
                <textarea data-testid="composer" />
                <PaneRequestDock blockId={PANE} />
            </>
        ));
        const composer = screen.getByTestId("composer") as HTMLTextAreaElement;
        composer.focus();
        uses = [use({ needs_value: true, goes_ahead: false, command: "", origin: "https://irs.gov", field: "SSN", kind: "ssn" })];
        await refreshVault();
        await screen.findByTestId("pane-request-dock");
        expect(document.activeElement).toBe(composer);
    });

    it("asks for a new entry in the agent's words, declines on Not now, and opens the form in the pane on Add it", async () => {
        requests = [request()];
        await refreshVault();
        render(() => <PaneRequestDock blockId={PANE} />);
        const dock = await screen.findByTestId("pane-request-dock");
        expect(dock.textContent).toContain("Builder asks you to add Postgres URL");
        expect(dock.textContent).toContain("run the migrations");
        fireEvent.click(screen.getByRole("button", { name: "Add it" }));
        expect(paneRequests.of(PANE).filter((r) => r.presentation === "modal")).toHaveLength(1);
        fireEvent.click(screen.getByRole("button", { name: "Not now" }));
        await waitFor(() => expect(calls("decline_request")).toEqual([{ request_id: "req-1" }]));
    });

    it("cuts an agent's long words, with the whole of them in the tooltip", async () => {
        const name = "Builder".repeat(10);
        const label = "Approved by AgentMux: ".repeat(6);
        requests = [request({ agent: name, label, why: "w".repeat(400) })];
        await refreshVault();
        const { unmount } = render(() => <PaneRequestDock blockId={PANE} />);
        let dock = await screen.findByTestId("pane-request-dock");
        const cut = [...dock.querySelectorAll<HTMLElement>(".vault-clipped[title]")];
        expect(cut.map((e) => e.title)).toEqual(expect.arrayContaining([name, label, "w".repeat(400)]));
        for (const e of cut) {
            expect(e.textContent!.endsWith("…")).toBe(true);
            expect(e.textContent!.length).toBeLessThanOrEqual(240);
        }
        expect(dock.textContent).not.toContain(label);
        unmount();

        // A use's title, cut the same way.
        requests = [];
        uses = [use({ agent: name, entry_label: label })];
        await refreshVault();
        render(() => <PaneRequestDock blockId={PANE} />);
        dock = await screen.findByTestId("pane-request-dock");
        const title = dock.querySelector<HTMLElement>(".vault-request-card-title")!;
        expect(title.title).toContain(name);
        expect(title.textContent).not.toContain(name);
    });

    it("keeps Deny first and the yeses together after it, so wrapping never puts Always beside Deny", async () => {
        uses = [use()];
        await refreshVault();
        render(() => <PaneRequestDock blockId={PANE} />);
        const deny = await screen.findByRole("button", { name: "Deny" });
        const actions = deny.closest(".vault-request-card-actions")!;
        expect(actions.firstElementChild).toBe(deny);
        const yeses = actions.querySelector(".vault-use-allow")!;
        expect([...yeses.querySelectorAll("button")].map((b) => b.textContent)).toEqual(["Allow once", "Always for Builder"]);
        expect(deny.nextElementSibling).toBe(yeses);
    });

    it("leaves a request with no pane to the window's prompt", () => {
        expect(askedInPane(use())).toBe(true);
        expect(askedInPane(use({ block_id: "" }))).toBe(false);
        expect(askedInPane(request({ block_id: "" }))).toBe(false);
    });
});
