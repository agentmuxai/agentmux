// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AttentionItem } from "@/app/notification/attention-list";
import type { WaitingApp } from "@/app/notification/waiting-for-you";
import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import { cleanup, fireEvent, render, screen, within } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const open = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: open }));
vi.mock("@/app/store/agent-runtime-live", () => ({ sharedAgentRuntime: () => ({ rows: () => [] }) }));
// The live Kanban count would start the board's feed; a test that wants tasks passes `tasks`.
vi.mock("@/app/view/kanban/kanban-held", () => ({ useKanbanNeeds: () => () => 0 }));
const goTo = vi.hoisted(() => vi.fn());
vi.mock("@/app/notification/attention-go", () => ({ goToAttention: goTo, isParkedItem: (i: { blockId: string }) => i.blockId === "parked" }));
vi.mock("@/app/notification/attention-list", () => ({ attentionItems: () => [], sharedAttentionRows: () => [], attentionTint: (hex?: string) => hex }));
vi.mock("@/app/notification/waiting-for-you", () => ({ waitingApps: () => [] }));
vi.mock("@/app/view/vault/vault-store", () => ({ vaultUses: () => [], vaultRequests: () => [] }));
const openVaultAt = vi.hoisted(() => vi.fn());
vi.mock("@/app/view/vault/open-vault", () => ({ openVaultAt }));
const openSettingsAt = vi.hoisted(() => vi.fn());
vi.mock("@/app/view/settings/open-settings", () => ({ openSettingsAt }));
const motion = vi.hoisted(() => ({ allowed: true, pulse: vi.fn() }));
vi.mock("@/app/motion/primitives", async (orig) => ({ ...(await orig<object>()), motionAllowed: () => motion.allowed, pulse: motion.pulse }));

import { SummaryStatus } from "./SummaryStatus";

afterEach(() => {
    cleanup();
    open.mockClear();
    goTo.mockClear();
    openVaultAt.mockClear();
    openSettingsAt.mockClear();
    motion.pulse.mockClear();
    motion.allowed = true;
});

const item = (blockId: string, over: Partial<AttentionItem> = {}): AttentionItem => ({
    blockId,
    name: `Agent-${blockId}`,
    what: "asks you a question",
    since: Date.now() - 125_000,
    kinds: [],
    local: true,
    ...over,
});

const agent = (id: string, state: AgentRuntime["state"]): AgentRuntime => ({
    agent_id: id,
    state,
    desired: "running",
    name: id,
    provider: "claude",
    icon: "",
    runtime: "host",
    block_id: id,
});

const use = (id: string, over: Partial<UseRequest> = {}): UseRequest =>
    ({ id, agent: "builder", block_id: "", entry_label: "Token", requested_ms: Date.now() - 60_000, goes_ahead: false, deadline_ms: 0, ...over }) as UseRequest;
const request = (id: string): VaultRequest => ({ id, agent: "builder", block_id: "", label: "", requested_ms: Date.now() - 60_000 }) as VaultRequest;

/** The live inputs, empty unless a test says so. */
const base = { rows: () => [] as AgentRuntime[], items: () => [] as AttentionItem[], apps: () => [] as WaitingApp[], uses: () => [] as UseRequest[], requests: () => [] as VaultRequest[], tasks: () => 0 };
const part = (c: HTMLElement, id: string) => c.querySelector(`[data-part="${id}"]`) as HTMLElement | null;

describe("SummaryStatus", () => {
    it("draws nothing while no agent runs and nothing waits", () => {
        const { container } = render(() => <SummaryStatus {...base} rows={() => [agent("a", "stopped"), agent("b", "error")]} />);
        expect(container.querySelector(".status-summary")).toBeNull();
    });

    it("counts an agent held by a sign-in or a limit as running: it has a process", () => {
        const { container } = render(() => <SummaryStatus {...base} rows={() => [agent("a", "signed_out"), agent("b", "limited")]} />);
        expect(part(container, "agents")?.textContent).toBe("2");
    });

    it("shows a part for each count, none for a zero, and says each in words in its tooltip", () => {
        const { container } = render(() => (
            <SummaryStatus
                {...base}
                rows={() => [agent("a", "working"), agent("b", "working")]}
                items={() => [item("q1"), item("q2"), item("w1", { kinds: ["browser"] })]}
                uses={() => [use("u1")]}
                requests={() => [request("r1")]}
                tasks={() => 4}
            />
        ));
        const shown = [...container.querySelectorAll(".status-summary__part")].map((e) => `${e.getAttribute("data-part")}:${e.textContent}`);
        expect(shown).toEqual(["agents:2", "asks:2", "browser:1", "vault:2", "tasks:4"]);
        expect(part(container, "vault")?.getAttribute("data-tip")).toBe("2 Vault requests wait on you");
        expect(part(container, "browser")?.getAttribute("data-tip")).toBe("1 browser pane needs you");
        expect(part(container, "tasks")?.getAttribute("data-tip")).toBe("4 Kanban tasks need you");
        expect(container.querySelector(".status-summary")?.getAttribute("aria-label")).toBe(
            "2 agents are running, 2 agents ask you something, 1 browser pane needs you, 2 Vault requests wait on you, 4 Kanban tasks need you. Click to see who"
        );
        expect(container.querySelector(".status-summary__badge")?.textContent).toBe("9");
    });

    it("follows the rows, and goes with the last agent", () => {
        const [rows, setRows] = createSignal([agent("a", "idle")]);
        const { container } = render(() => <SummaryStatus {...base} rows={rows} />);
        expect(part(container, "agents")?.textContent).toBe("1");
        expect(part(container, "asks")).toBeNull();
        expect(container.querySelector(".status-summary__badge")).toBeNull();
        setRows([]);
        expect(container.querySelector(".status-summary")).toBeNull();
    });

    it("opens the Swarm on a click while nothing waits", () => {
        const { container } = render(() => <SummaryStatus {...base} rows={() => [agent("a", "working")]} />);
        fireEvent.click(part(container, "agents")!);
        expect(open).toHaveBeenCalledWith("swarm");
        expect(screen.queryByRole("dialog")).toBeNull();
    });

    it("stays while only a Vault request or a task waits, with no agent running", () => {
        const { container } = render(() => <SummaryStatus {...base} uses={() => [use("u1")]} tasks={() => 2} />);
        expect(container.querySelector(".status-summary")).not.toBeNull();
        expect(part(container, "agents")).toBeNull();
        expect(part(container, "vault")?.textContent).toBe("1");
        expect(part(container, "tasks")?.textContent).toBe("2");
    });

    it("marks an item that has an ask, a browser wait or a Vault request, so a narrow bar sheds the tasks only while one is left", () => {
        const group = (c: HTMLElement) => c.querySelector(".status-summary") as HTMLElement;
        const a = render(() => <SummaryStatus {...base} items={() => [item("q1")]} tasks={() => 2} />);
        expect(group(a.container).hasAttribute("data-core")).toBe(true);
        cleanup();
        const b = render(() => <SummaryStatus {...base} uses={() => [use("u1")]} />);
        expect(group(b.container).hasAttribute("data-core")).toBe(true);
        cleanup();
        // Tasks alone, or with agents running: no core, so the badge takes over from tier 3 and the item is never blank.
        const c = render(() => <SummaryStatus {...base} rows={() => [agent("a", "working")]} tasks={() => 2} />);
        expect(group(c.container).hasAttribute("data-core")).toBe(false);
        expect(c.container.querySelector(".status-summary__badge")?.textContent).toBe("2");
    });

    it("keeps counting when the Vault's store has no list yet: a reply that leaves one out does not stop it", () => {
        const none = (() => undefined) as never;
        const { container } = render(() => <SummaryStatus {...base} uses={none} requests={none} items={() => [item("q1")]} />);
        expect(part(container, "asks")?.textContent).toBe("1");
        expect(part(container, "vault")).toBeNull();
    });

    it("keeps the one badge to the operator while a Vault request waits, as its part is", () => {
        const a = render(() => <SummaryStatus {...base} items={() => [item("q1")]} uses={() => [use("u1")]} />);
        expect(a.container.querySelector(".status-summary__badge")?.hasAttribute("data-operator-only")).toBe(true);
        cleanup();
        const b = render(() => <SummaryStatus {...base} items={() => [item("q1")]} />);
        expect(b.container.querySelector(".status-summary__badge")?.hasAttribute("data-operator-only")).toBe(false);
    });

    it("keeps the Vault part to the operator: a widget cannot read or click it", () => {
        const { container } = render(() => <SummaryStatus {...base} uses={() => [use("u1")]} items={() => [item("q1")]} />);
        expect(part(container, "vault")?.hasAttribute("data-operator-only")).toBe(true);
        expect(part(container, "asks")?.hasAttribute("data-operator-only")).toBe(false);
    });

    describe("the list of who waits", () => {
        it("has a section for each part that has rows, each row with who, what, how long and a button", () => {
            const { container } = render(() => (
                <SummaryStatus
                    {...base}
                    items={() => [item("b1", { name: "Korp", what: "asks to approve a Bash call" }), item("b2", { local: false })]}
                    uses={() => [use("u1", { agent: "Clare", entry_label: "Deploy key" })]}
                    tasks={() => 3}
                />
            ));
            fireEvent.click(part(container, "asks")!);
            const list = screen.getByRole("dialog", { name: "Waiting on you" });
            expect(within(list).getAllByRole("heading").map((h) => h.textContent)).toEqual(["Asks", "Vault", "Tasks"]);
            expect(within(list).getByText("Korp")).toBeTruthy();
            expect(within(list).getByText("asks to approve a Bash call")).toBeTruthy();
            expect(within(list).getByText("asks you a question (in another window)")).toBeTruthy();
            expect(within(list).getByText("Clare")).toBeTruthy();
            expect(within(list).getByText("asks to use Deploy key")).toBeTruthy();
            expect(within(list).getByText("3 tasks need you")).toBeTruthy();
            expect(within(list).getAllByText(/1? ?min/).length).toBeGreaterThan(0);
        });

        it("keeps the Vault's rows to the operator too: their agents, entries and Answer buttons are not for a widget to read or click", () => {
            const { container } = render(() => <SummaryStatus {...base} items={() => [item("b1")]} uses={() => [use("u1")]} />);
            fireEvent.click(part(container, "vault")!);
            const list = screen.getByRole("dialog");
            const sections = [...list.querySelectorAll("section")];
            expect(sections.map((x) => [x.getAttribute("aria-label"), x.hasAttribute("data-operator-only")])).toEqual([
                ["Vault", true],
                ["Asks", false],
            ]);
            expect(within(sections[0] as HTMLElement).getByText("asks to use Token")).toBeTruthy();
        });

        it("lists the part that was clicked first", () => {
            const { container } = render(() => <SummaryStatus {...base} items={() => [item("b1")]} uses={() => [use("u1")]} tasks={() => 1} />);
            fireEvent.click(part(container, "vault")!);
            expect(within(screen.getByRole("dialog")).getAllByRole("heading").map((h) => h.textContent)).toEqual(["Vault", "Asks", "Tasks"]);
        });

        it("takes the person to the pane from its row and closes the list", () => {
            const { container } = render(() => <SummaryStatus {...base} items={() => [item("b1", { name: "Korp" }), item("b2")]} />);
            fireEvent.click(part(container, "asks")!);
            fireEvent.click(screen.getByRole("button", { name: "Go to Korp" }));
            expect(goTo).toHaveBeenCalledWith(expect.objectContaining({ blockId: "b1" }));
            expect(screen.queryByRole("dialog")).toBeNull();
        });

        it("offers Open for an agent that runs with no pane", () => {
            const { container } = render(() => <SummaryStatus {...base} items={() => [item("parked", { name: "Clare" })]} />);
            fireEvent.click(part(container, "asks")!);
            fireEvent.click(screen.getByRole("button", { name: "Open Clare" }));
            expect(goTo).toHaveBeenCalledWith(expect.objectContaining({ blockId: "parked" }));
        });

        it("opens the Vault at a request that has no pane, and the Kanban from its row", () => {
            const { container } = render(() => <SummaryStatus {...base} uses={() => [use("u1")]} tasks={() => 2} />);
            fireEvent.click(part(container, "vault")!);
            fireEvent.click(screen.getByRole("button", { name: "Answer builder" }));
            expect(openVaultAt).toHaveBeenCalledWith("u1");
            fireEvent.click(part(container, "tasks")!);
            fireEvent.click(screen.getByRole("button", { name: "Open the Kanban" }));
            expect(open).toHaveBeenCalledWith("kanban");
        });

        it("says a Vault use that goes ahead on silence, with its countdown", () => {
            const { container } = render(() => <SummaryStatus {...base} uses={() => [use("u1", { goes_ahead: true, deadline_ms: Date.now() + 12_000 })]} />);
            fireEvent.click(part(container, "vault")!);
            expect(screen.getByText(/goes ahead in 0:1[12] unless you deny/)).toBeTruthy();
        });

        it("gives a request with no pane a row, with the place it is answered", () => {
            const apps: WaitingApp[] = [{ key: "w1", kind: "widget", what: "asks to install a widget", since: Date.now() - 5_000 }];
            const { container } = render(() => <SummaryStatus {...base} apps={() => apps} />);
            expect(part(container, "asks")?.textContent).toBe("1");
            fireEvent.click(part(container, "asks")!);
            fireEvent.click(screen.getByRole("button", { name: "Answer AgentMux" }));
            expect(openSettingsAt).toHaveBeenCalledWith("widgets");
        });

        it("opens the Swarm from its footer", () => {
            const { container } = render(() => <SummaryStatus {...base} items={() => [item("b1")]} />);
            fireEvent.click(part(container, "asks")!);
            fireEvent.click(screen.getByRole("button", { name: "Open the Swarm" }));
            expect(open).toHaveBeenCalledWith("swarm");
            expect(screen.queryByRole("dialog")).toBeNull();
        });

        it("toggles on a second click of the same part", () => {
            const { container } = render(() => <SummaryStatus {...base} items={() => [item("b1")]} />);
            fireEvent.click(part(container, "asks")!);
            expect(screen.getByRole("dialog")).toBeTruthy();
            fireEvent.click(part(container, "asks")!);
            expect(screen.queryByRole("dialog")).toBeNull();
        });

        it("closes the list when a narrow bar hides the item", () => {
            // The popover's own positioning watches its anchor too: every observer is told.
            const notify: (() => void)[] = [];
            vi.stubGlobal(
                "ResizeObserver",
                class {
                    constructor(cb: () => void) {
                        notify.push(cb);
                    }
                    observe() {}
                    disconnect() {}
                }
            );
            const { container } = render(() => <SummaryStatus {...base} items={() => [item("b1")]} />);
            fireEvent.click(part(container, "asks")!);
            expect(screen.getByRole("dialog")).toBeTruthy();
            // jsdom has no layout: an element with no boxes is one `display: none` took out of the bar.
            notify.forEach((cb) => cb());
            expect(screen.queryByRole("dialog")).toBeNull();
            vi.unstubAllGlobals();
        });

        it("closes for good when the last thing is answered, and the next one does not open it unasked", () => {
            const [items, setItems] = createSignal([item("b1")]);
            const { container } = render(() => <SummaryStatus {...base} items={items} />);
            fireEvent.click(part(container, "asks")!);
            expect(screen.getByRole("dialog")).toBeTruthy();
            setItems([]);
            expect(screen.queryByRole("dialog")).toBeNull();
            setItems([item("b2")]);
            expect(screen.queryByRole("dialog")).toBeNull();
            expect(part(container, "asks")?.getAttribute("aria-haspopup")).toBe("dialog");
        });
    });

    describe("a pane that waits on a Vault request", () => {
        it("is the Vault's row, not also an ask", () => {
            const { container } = render(() => (
                <SummaryStatus {...base} rows={() => [agent("a", "waiting_on_user")]} items={() => [item("a", { kinds: ["vault"] })]} uses={() => [use("u1", { block_id: "a" })]} />
            ));
            expect(part(container, "asks")).toBeNull();
            expect(part(container, "vault")?.textContent).toBe("1");
        });

        it("is still an ask when it waits on a question as well", () => {
            const { container } = render(() => <SummaryStatus {...base} items={() => [item("a", { kinds: ["vault", "consent"] })]} />);
            expect(part(container, "asks")?.textContent).toBe("1");
        });
    });

    describe("the pulse on a new Vault request", () => {
        it("flashes the Vault part once when one arrives, not for those there already", () => {
            const [uses, setUses] = createSignal([use("u1")]);
            render(() => <SummaryStatus {...base} uses={uses} />);
            expect(motion.pulse).not.toHaveBeenCalled();
            setUses([use("u1"), use("u2")]);
            expect(motion.pulse).toHaveBeenCalledTimes(1);
            expect((motion.pulse.mock.calls[0][0] as HTMLElement).getAttribute("data-part")).toBe("vault");
            setUses([use("u2")]);
            expect(motion.pulse).toHaveBeenCalledTimes(1);
        });

        it("does not flash when the Animations setting is off", () => {
            motion.allowed = false;
            const [uses, setUses] = createSignal([use("u1")]);
            render(() => <SummaryStatus {...base} uses={uses} />);
            setUses([use("u1"), use("u2")]);
            expect(motion.pulse).not.toHaveBeenCalled();
        });
    });
});
