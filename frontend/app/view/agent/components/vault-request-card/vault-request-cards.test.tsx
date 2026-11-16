// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** An agent's Vault requests as cards in its own pane
 *  (docs/specs/SPEC_VAULT_REQUESTS_AS_CARDS_IN_THE_AGENT_PANE_2026_11_01.md §4, §14):
 *  drawn from srv's record in the pane that asked, answered through the host,
 *  gone when srv's list drops them, never taking the composer's focus. */

import { readFileSync } from "node:fs";
import { join } from "node:path";

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/notification/waiting-for-you", () => ({ startWaitingForYou: vi.fn(), endWaitingForYou: vi.fn() }));
vi.mock("@/app/store/global", async (orig) => ({ ...(await orig<object>()), MOS: { getMuxObjectAtom: () => () => ({ oid: "pane" }) } }));
const openedAt = vi.hoisted(() => vi.fn(async (_id?: string) => {}));
vi.mock("@/app/view/vault/open-vault", () => ({ openVault: vi.fn(async () => {}), openVaultAt: openedAt }));

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
import { VaultRequestsList } from "@/app/view/vault/vault-requests-list";
import { SummaryStatus } from "@/app/statusbar/SummaryStatus";
import { VaultRowAsks } from "./vault-row-asks";
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
    page: "",
    field: "GH_TOKEN (value)",
    offer_always: true,
    reason: "it is the first time Builder runs a command with this entry",
    agent_reason: "check the build output for task 27968",
    needs_value: false,
    kind: "api-key",
    command: "gh release create v1",
    cwd: "C:/work",
    replaces: false,
    signin: false,
    account: false,
    change: false,
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
    vault: "",
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
        const cut = [...dock.querySelectorAll<HTMLElement>(".vault-clipped[data-hover-tip]")];
        expect(cut.map((e) => e.dataset.hoverTip)).toEqual(expect.arrayContaining([name, label, "w".repeat(400)]));
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
        expect(title.dataset.hoverTip).toContain(name);
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

    it("lists a batch's rows before its account and Vault in the pane, and after them in the Vault pane's list", async () => {
        const items = ["Alpha", "Beta"].map((label, i) => ({ label, kind: "login", secret: `hosts/${label.toLowerCase()}`, key: "", replaces: false, link: i === 1 }));
        const batch = { name: "host passwords", service: "AWS Secrets Manager", region: "eu-west-1", profile: "work-read", grant: false, vault: "Fleet", agents: { mode: "all" as const }, vaults: [], items };
        const asked = (block_id: string) => use({ block_id, entry_id: "", entry_label: "host passwords", what: "secrets", command: "", offer_always: false, goes_ahead: false, batch });
        // The rows, the count of what is ticked and the paragraph that names the account, in the order the page reads them.
        const order = (card: HTMLElement) => [...card.querySelectorAll<HTMLElement>('.vault-batch-rows, [data-batch="mixed"], [data-batch="source"]')].map((e) => e.dataset.batch ?? "rows");

        // A card has a share of the pane's height: in a narrow, zoomed pane only its first lines are in view.
        uses = [asked(PANE)];
        await refreshVault();
        render(() => <PaneRequestDock blockId={PANE} />);
        const dock = await screen.findByTestId("pane-request-dock");
        expect(order(dock)).toEqual(["rows", "mixed", "source"]);
        expect(dock.querySelector('[data-batch="source"]')?.textContent).toContain("Read from AWS Secrets Manager in eu-west-1.");
        expect(within(dock).getAllByRole("checkbox")).toHaveLength(2);
        cleanup();

        // The Vault pane's list has the pane to itself: the account and the Vault come first, as they did.
        uses = [asked("")];
        await refreshVault();
        render(() => <VaultRequestsList onFill={vi.fn()} />);
        expect(order(await screen.findByRole("group"))).toEqual(["source", "mixed", "rows"]);
    });

    it("scrolls every card as a whole, keeps its answers in view and gives it more of the pane", () => {
        // What a narrow, zoomed pane needs is in the stylesheet: with the body as the scroll box, the title and
        // the answers could fill the card and leave what it says no room at all.
        const scss = readFileSync(join(__dirname, "vault-request-card.scss"), "utf8").replace(/\s+/g, " ");
        expect(scss).toMatch(/ \.vault-request-card \{[^{}]* overflow-y: auto;[^{}]*\}/);
        expect(scss).toMatch(/\.vault-request-card-title, \.vault-request-card-body \{ flex-shrink: 0; \}/);
        // The body is no scroll box of its own, for a batch's card or any other.
        const body = / \.vault-request-card-body \{([^{}]*) p \{/.exec(scss)?.[1];
        expect(body).toMatch(/display: flex;/);
        expect(body).not.toMatch(/overflow-y|min-height/);
        expect(scss).not.toContain(".vault-request-card:has(");
        expect(scss).toMatch(/ \.vault-request-card-actions \{ position: sticky; bottom: 0;/);
        // The larger share gives way before the status rows under the stack do, and no further than the old share.
        const stack = /\.agent-request-stack:has\(\.vault-request-card\) \{([^{}]*)\}/.exec(scss)?.[1];
        expect(stack).toMatch(/max-height: 70%;/);
        expect(stack).toMatch(/flex-shrink: 1000;/);
        expect(stack).toMatch(/min-height: min\(40%, 6rem\);/);
        // A scroll bar that comes or goes must not change where the card's lines wrap.
        expect(scss).toMatch(/ \.vault-request-card \{[^{}]* scrollbar-gutter: stable;[^{}]*\}/);
    });

    it("denies all from an agent waiting on more than one, uses and asks for entries alike, whatever the agent's case", async () => {
        uses = [use(), use({ id: "use-2", entry_label: "Stripe key" })];
        requests = [request({ agent: "builder" })];
        await refreshVault();
        render(() => <PaneRequestDock blockId={PANE} />);
        fireEvent.click(await screen.findByRole("button", { name: /^Deny all 3 from builder$/i }));
        await waitFor(() => expect(calls("answer_use")).toEqual(expect.arrayContaining([{ id: "use-1", decision: "deny" }, { id: "use-2", decision: "deny" }])));
        await waitFor(() => expect(calls("decline_request")).toEqual([{ request_id: "req-1" }]));
    });

    it("asks a request with no pane on screen in the Vault pane's list, not in any pane", async () => {
        uses = [use({ block_id: "", entry_label: "Parked token" })];
        requests = [request({ block_id: "", label: "Parked URL" })];
        await refreshVault();
        expect(paneRequests.of("")).toHaveLength(0);
        render(() => <VaultRequestsList onFill={vi.fn()} focused={() => "req-1"} />);
        const list = await screen.findByRole("region", { name: "Agents' requests" });
        expect(list.textContent).toContain("Builder wants to run a command with your value");
        expect(list.textContent).toContain("Parked URL");
        expect([...list.querySelectorAll(".is-focused")].map((e) => (e as HTMLElement).dataset.request), "the one Answer sent the person to").toEqual(["req-1"]);
        fireEvent.click(within(list).getByRole("button", { name: "Deny" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "deny" }]));
    });

    it("keeps what the person typed in the Vault pane's list when srv's lists refresh", async () => {
        const typed = () => use({ block_id: "", needs_value: true, goes_ahead: false, command: "", origin: "https://irs.gov", field: "SSN", kind: "ssn" });
        uses = [typed()];
        await refreshVault();
        render(() => <VaultRequestsList onFill={vi.fn()} />);
        const item = await screen.findByRole("group");
        const field = item.querySelector('input[type="password"]') as HTMLInputElement;
        fireEvent.input(field, { target: { value: "987-65-4321" } });
        // Another agent asks: srv sends both lists again, as fresh objects.
        uses = [typed(), use({ id: "use-2", agent: "Other", block_id: "" })];
        await refreshVault();
        await waitFor(() => expect(screen.getAllByRole("group")).toHaveLength(2));
        expect(screen.getAllByRole("group")[0], "the same item, not a new one").toBe(item);
        expect((item.querySelector('input[type="password"]') as HTMLInputElement).value).toBe("987-65-4321");
    });

    it("follows the countdown to the next parked request when the first is answered", async () => {
        uses = [use({ block_id: "", deadline_ms: Date.now() + 5_000 }), use({ id: "use-2", block_id: "", entry_label: "Second", deadline_ms: Date.now() + 30_000 })];
        await refreshVault();
        render(() => <VaultRowAsks agent="builder" />);
        const line = await screen.findByTestId("vault-row-asks");
        expect(line.textContent).toMatch(/goes ahead in 0:0[45]/);
        uses = [uses[1]];
        await refreshVault();
        await waitFor(() => expect(line.textContent).toMatch(/Second · goes ahead in 0:(29|30)/));
    });

    it("shows a parked agent's request on its My agents row and counts every request in the status bar", async () => {
        uses = [use({ block_id: "", entry_label: "Parked token" }), use({ id: "use-2" })];
        await refreshVault();
        render(() => (
            <>
                <VaultRowAsks agent="builder" />
                <SummaryStatus rows={() => []} items={() => []} apps={() => []} tasks={() => 0} />
            </>
        ));
        const line = await screen.findByTestId("vault-row-asks");
        expect(line.hasAttribute("data-operator-only")).toBe(true);
        expect(line.textContent).toMatch(/Waiting for you in the Vault: Parked token · goes ahead in 0:(29|30)/);
        expect(line.textContent, "the one in its pane is asked there").not.toContain("1 more");
        // The one asked in its pane is counted too: the status bar counts every request, whatever pane asked.
        const status = screen.getByRole("button", { name: "2 Vault requests wait on you" });
        expect(status.textContent).toBe("2");
        fireEvent.click(within(line).getByRole("button", { name: "Answer" }));
        // The status bar lists them: one list with a row for each, and a button that opens the Vault on it.
        fireEvent.click(status);
        const list = screen.getByRole("dialog", { name: "Waiting on you" });
        fireEvent.click(within(list).getAllByRole("button", { name: /^Answer/ })[0]);
        expect(openedAt.mock.calls, "both open the Vault on the parked request").toEqual([["use-1"], ["use-1"]]);
        fireEvent.click(within(line).getByRole("button", { name: "Deny" }));
        await waitFor(() => expect(calls("answer_use")).toEqual([{ id: "use-1", decision: "deny" }]));
    });
});
