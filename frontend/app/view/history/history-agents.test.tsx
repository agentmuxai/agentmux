// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { HistoryAgent } from "@/types/rpc/HistoryAgent";
import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const HistoryAgentsCommand = vi.fn();
const HistoryAgentSessionsCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        HistoryAgentsCommand: (...a: unknown[]) => HistoryAgentsCommand(...a),
        HistoryAgentSessionsCommand: (...a: unknown[]) => HistoryAgentSessionsCommand(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
// srv's events, fired by the test.
const eventHandlers = new Map<string, Set<() => void>>();
vi.mock("@/app/store/mps", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/mps")>()),
    onMuxEvent: (event: string, handler: () => void) => {
        if (!eventHandlers.has(event)) eventHandlers.set(event, new Set());
        eventHandlers.get(event)!.add(handler);
        return () => eventHandlers.get(event)?.delete(handler);
    },
}));
vi.mock("@/app/store/ws", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/ws")>()),
    addWSReconnectHandler: () => {},
}));
const fireMux = (event: string) => eventHandlers.get(event)?.forEach((h) => h());
const openOrFocusHistoryTab = vi.fn();
vi.mock("@/app/view/agent/open-history-tab", () => ({
    openOrFocusHistoryTab: (...a: unknown[]) => openOrFocusHistoryTab(...a),
}));

import { HistoryAgents } from "./history-agents";

const NOW = new Date("2026-11-08T12:00:00Z").getTime();

const agent = (id: string, extra: Partial<HistoryAgent> = {}): HistoryAgent => ({
    definition_id: id,
    name: id,
    provider: "claude",
    fork_of: "",
    fork_of_name: "",
    deleted: false,
    current_bytes: 2048,
    archives: 0,
    archive_bytes: 0,
    last_active_ms: NOW - 60_000,
    ...extra,
});

const sessionsAnswer = {
    sessions: [{ session_id: "s1234567890", cwd: "/work/a", first_ms: 1, last_ms: 2, bytes: 512, archived: true }],
    problems: [],
};

const mountAgents = (visible = true) => render(() => <HistoryAgents paneBlockId="pane-1" visible={visible} />);

const rowOf = (container: HTMLElement, id: string) => container.querySelector<HTMLElement>(`tr[data-agent="${id}"]`)!;
const cells = (row: HTMLElement) => Array.from(row.querySelectorAll("td")).map((c) => c.textContent);

describe("HistoryAgents", () => {
    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        vi.setSystemTime(NOW);
        vi.clearAllMocks();
        eventHandlers.clear();
        HistoryAgentsCommand.mockResolvedValue({
            agents: [
                agent("parent-1", { name: "Parent", archives: 2, archive_bytes: 1024 }),
                agent("fork-1", { name: "Parent 2", fork_of: "parent-1", fork_of_name: "Parent" }),
                agent("gone-123456", { name: "", provider: "", deleted: true, last_active_ms: 0 }),
            ],
        });
        HistoryAgentSessionsCommand.mockResolvedValue(sessionsAnswer);
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    describe("loading and reading the list", () => {
        it("asks srv for the list with an empty request", async () => {
            mountAgents();
            await waitFor(() => expect(HistoryAgentsCommand).toHaveBeenCalledTimes(1));
            expect(HistoryAgentsCommand).toHaveBeenCalledWith({}, {});
        });

        it("says Loading… and shows no summary before the first answer", async () => {
            let finish!: (v: unknown) => void;
            HistoryAgentsCommand.mockReturnValue(new Promise((r) => (finish = r)));
            const { container } = mountAgents();
            expect(screen.getByText("Loading…")).toBeInTheDocument();
            expect(container.querySelector(".warden-manager-summary")).toBeNull();
            expect(container.querySelector("table")).toBeNull();
            finish({ agents: [] });
            await waitFor(() => expect(screen.queryByText("Loading…")).toBeNull());
        });

        it("an empty list says there is no history yet, with a zero summary and no table", async () => {
            HistoryAgentsCommand.mockResolvedValue({ agents: [] });
            const { container } = mountAgents();
            await screen.findByText("No agent history yet.");
            expect(container.querySelector(".warden-manager-summary")!.textContent).toBe(
                "0 agents · 0 B kept · 0 archived conversations"
            );
            expect(container.querySelector("table")).toBeNull();
        });

        it("a failed first load shows the error instead of Loading…, and no summary", async () => {
            HistoryAgentsCommand.mockRejectedValue(new Error("srv is down"));
            const { container } = mountAgents();
            await waitFor(() => expect(container.querySelector(".warden-manager-error")).not.toBeNull());
            expect(container.querySelector(".warden-manager-error")!.textContent).toBe("⚠ srv is down");
            expect(screen.queryByText("Loading…")).toBeNull();
            expect(screen.getByText("No agent history yet.")).toBeInTheDocument();
            expect(container.querySelector(".warden-manager-summary")).toBeNull();
        });

        it("shows no error while the load is fine", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            expect(container.querySelector(".warden-manager-error")).toBeNull();
        });

        it("sums the whole list in the summary", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            // 3 agents; 2048 + 1024 + 2048 + 2048 bytes = 7168 (7.0 KB); 2 archived conversations.
            expect(container.querySelector(".warden-manager-summary")!.textContent).toBe(
                "3 agents · 7.0 KB kept · 2 archived conversations"
            );
        });

        it("explains the list and the Open history button in a footnote", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            expect(container.querySelector(".warden-manager-footnote")!.textContent).toBe(
                'Kept forever. "Open history" shows an agent\'s whole recorded conversation in this pane.'
            );
        });

        it("is named for the dev tools", () => {
            expect(HistoryAgents.displayName).toBe("HistoryAgents");
        });
    });

    describe("the table", () => {
        it("has the six column headings, the two numeric ones aligned", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            const heads = Array.from(container.querySelectorAll("thead th"));
            expect(heads.map((h) => h.textContent)).toEqual([
                "Agent",
                "Provider",
                "History",
                "Archived",
                "Last active",
                "",
            ]);
            expect(heads.map((h) => h.classList.contains("warden-history-num"))).toEqual([
                false,
                false,
                true,
                true,
                false,
                false,
            ]);
        });

        it("names each data cell's column, so a narrow pane can stack a row as a card", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            const row = rowOf(container, "parent-1");
            expect(Array.from(row.querySelectorAll("td")).map((c) => c.getAttribute("data-label"))).toEqual([
                null,
                "Provider",
                "History",
                "Archived",
                "Last active",
                null,
            ]);
            expect(row.querySelectorAll("td.warden-history-cell-wide")).toHaveLength(2);
        });

        it("shows an agent's name, short id, provider, kept size, archives and last activity", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            const row = rowOf(container, "parent-1");
            expect(row.querySelector(".warden-history-id")!.textContent).toBe("parent-1".slice(0, 8));
            expect(cells(row).slice(1, 5)).toEqual(["claude", "3.0 KB", "2", "1m ago"]);
            expect(row.querySelector("td")!.textContent).toContain("Parent");
        });

        it("cuts the id to eight characters", async () => {
            HistoryAgentsCommand.mockResolvedValue({ agents: [agent("abcdefghijkl", { name: "Long" })] });
            const { container } = mountAgents();
            await screen.findByText("Long");
            expect(rowOf(container, "abcdefghijkl").querySelector(".warden-history-id")!.textContent).toBe("abcdefgh");
        });

        it("sizes the history cell as the current and the archived bytes together", async () => {
            HistoryAgentsCommand.mockResolvedValue({
                agents: [agent("a1", { name: "Sized", current_bytes: 1024 * 1024, archive_bytes: 3 * 1024 * 1024 })],
            });
            const { container } = mountAgents();
            await screen.findByText("Sized");
            expect(cells(rowOf(container, "a1"))[2]).toBe("4.0 MB");
        });

        it("shows a dash for no provider, no archives, and an agent never active", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            const gone = cells(rowOf(container, "gone-123456"));
            expect(gone[1]).toBe("—");
            expect(gone[3]).toBe("—");
            expect(gone[4]).toBe("—");
        });

        it("counts the last activity in seconds, minutes and hours ago", async () => {
            HistoryAgentsCommand.mockResolvedValue({
                agents: [
                    agent("s1", { name: "Sec", last_active_ms: NOW - 5_000 }),
                    agent("m1", { name: "Min", last_active_ms: NOW - 5 * 60_000 }),
                    agent("h1", { name: "Hour", last_active_ms: NOW - 3 * 3_600_000 }),
                ],
            });
            const { container } = mountAgents();
            await screen.findByText("Sec");
            expect(cells(rowOf(container, "s1"))[4]).toBe("5s ago");
            expect(cells(rowOf(container, "m1"))[4]).toBe("5m ago");
            expect(cells(rowOf(container, "h1"))[4]).toBe("3h ago");
        });

        it("marks a deleted agent's row, and says Deleted agent instead of its name", async () => {
            HistoryAgentsCommand.mockResolvedValue({
                agents: [agent("live-1", { name: "Alive" }), agent("dead-1", { name: "Was Named", deleted: true })],
            });
            const { container } = mountAgents();
            await screen.findByText("Alive");
            expect(rowOf(container, "live-1").hasAttribute("data-deleted")).toBe(false);
            expect(rowOf(container, "dead-1").hasAttribute("data-deleted")).toBe(true);
            expect(rowOf(container, "dead-1").querySelector("td")!.textContent).toContain("Deleted agent");
            expect(rowOf(container, "dead-1").querySelector("td")!.textContent).not.toContain("Was Named");
            expect(rowOf(container, "live-1").querySelector("td")!.textContent).not.toContain("Deleted agent");
        });

        it("indents a fork under its parent by 1.25em a level", async () => {
            HistoryAgentsCommand.mockResolvedValue({
                agents: [
                    agent("p", { name: "Root" }),
                    agent("c", { name: "Child", fork_of: "p", fork_of_name: "Root" }),
                    agent("g", { name: "Grand", fork_of: "c", fork_of_name: "Child" }),
                ],
            });
            const { container } = mountAgents();
            await screen.findByText("Grand");
            const pad = (id: string) =>
                rowOf(container, id).querySelector<HTMLElement>(".warden-history-agent")!.style.paddingLeft;
            expect([pad("p"), pad("c"), pad("g")]).toEqual(["0em", "1.25em", "2.5em"]);
        });

        it("says what a fork was forked from: the name, else the start of its id", async () => {
            HistoryAgentsCommand.mockResolvedValue({
                agents: [
                    agent("f1", { name: "Named fork", fork_of: "parent-id-0123456789", fork_of_name: "Parent name" }),
                    agent("f2", { name: "Unnamed fork", fork_of: "parent-id-0123456789", fork_of_name: "" }),
                    agent("plain", { name: "Plain" }),
                ],
            });
            const { container } = mountAgents();
            await screen.findByText("Plain");
            expect(rowOf(container, "f1").querySelector(".warden-history-fork")!.textContent).toBe(
                "fork of Parent name"
            );
            expect(rowOf(container, "f2").querySelector(".warden-history-fork")!.textContent).toBe("fork of parent-i");
            expect(rowOf(container, "plain").querySelector(".warden-history-fork")).toBeNull();
        });

        it("keeps the list in the order the tree gives, forks under their parent", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            const ids = Array.from(container.querySelectorAll("tr[data-agent]")).map((r) =>
                r.getAttribute("data-agent")
            );
            expect(ids).toEqual(["parent-1", "fork-1", "gone-123456"]);
        });
    });

    describe("filtering", () => {
        const input = () => screen.getByPlaceholderText("Filter agents") as HTMLInputElement;

        it("narrows the list to the matches, flat, and keeps the summary of the whole list", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.input(input(), { target: { value: "fork" } });
            const ids = Array.from(container.querySelectorAll("tr[data-agent]")).map((r) =>
                r.getAttribute("data-agent")
            );
            expect(ids).toEqual(["fork-1"]);
            expect(container.querySelector(".warden-manager-summary")!.textContent).toMatch(
                /^3 agents · 7\.0 KB kept · 2 archived/
            );
            expect(
                rowOf(container, "fork-1").querySelector<HTMLElement>(".warden-history-agent")!.style.paddingLeft
            ).toBe("0em");
        });

        it("drops a row that the filter removes while its sessions are open, without a stale read", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.click(rowOf(container, "fork-1").querySelector(".warden-history-agent button")!);
            await screen.findByText("s1234567");
            const errors = vi.spyOn(console, "error").mockImplementation(() => {});
            fireEvent.input(input(), { target: { value: "gone" } });
            const ids = Array.from(container.querySelectorAll("tr[data-agent]")).map((r) =>
                r.getAttribute("data-agent")
            );
            expect(ids).toEqual(["gone-123456"]);
            expect(container.querySelector(".warden-history-sessions-row")).toBeNull();
            fireEvent.input(input(), { target: { value: "" } });
            expect(container.querySelectorAll("tr[data-agent]")).toHaveLength(3);
            expect(errors).not.toHaveBeenCalled();
            errors.mockRestore();
        });

        it("says no agent matches when the filter excludes everything, and no table", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.input(input(), { target: { value: "zzz-nothing" } });
            expect(screen.getByText("No agent matches.")).toBeInTheDocument();
            expect(screen.queryByText("No agent history yet.")).toBeNull();
            expect(container.querySelector("table")).toBeNull();
        });

        it("shows the typed text, and the clear button empties it and brings the list back", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.input(input(), { target: { value: "fork" } });
            expect(input().value).toBe("fork");
            fireEvent.click(screen.getByLabelText("Clear filter"));
            expect(input().value).toBe("");
            expect(container.querySelectorAll("tr[data-agent]")).toHaveLength(3);
        });
    });

    describe("opening history", () => {
        it("opens the agent's history tab from the pane, by its id and provider", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.click(rowOf(container, "fork-1").querySelector(".warden-history-actions button")!);
            expect(openOrFocusHistoryTab).toHaveBeenCalledWith({
                currentBlockId: "pane-1",
                agentId: "fork-1",
                provider: "claude",
                agentName: "Parent 2",
            });
        });

        it("names a deleted agent by its id when it has no name", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.click(rowOf(container, "gone-123456").querySelector(".warden-history-actions button")!);
            expect(openOrFocusHistoryTab).toHaveBeenCalledWith({
                currentBlockId: "pane-1",
                agentId: "gone-123456",
                provider: "",
                agentName: "gone-123456",
            });
        });
    });

    describe("a row's sessions", () => {
        it("opens under its row, flips the chevron and the label, and closes again", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            const chevron = () => rowOf(container, "parent-1").querySelector("i, svg, .ui-icon");
            expect(screen.getAllByRole("button", { name: "Show sessions" })).toHaveLength(3);
            fireEvent.click(screen.getAllByRole("button", { name: "Show sessions" })[0]);
            await screen.findByText("s1234567");
            expect(HistoryAgentSessionsCommand).toHaveBeenCalledWith(
                {},
                { definition_id: "parent-1" },
                { timeout: 300_000 }
            );
            expect(screen.getAllByRole("button", { name: "Hide sessions" })).toHaveLength(1);
            expect(
                rowOf(container, "parent-1").nextElementSibling!.classList.contains("warden-history-sessions-row")
            ).toBe(true);
            expect(container.querySelector(".warden-history-sessions-row td")!.getAttribute("colspan")).toBe("6");
            expect(chevron()).not.toBeNull();
            fireEvent.click(screen.getByRole("button", { name: "Hide sessions" }));
            expect(container.querySelector(".warden-history-sessions-row")).toBeNull();
            expect(screen.getAllByRole("button", { name: "Show sessions" })).toHaveLength(3);
        });

        it("uses the chevron that points down while open and right while closed", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            const icon = () => rowOf(container, "parent-1").querySelector(".warden-history-agent button")!.innerHTML;
            const closed = icon();
            fireEvent.click(screen.getAllByRole("button", { name: "Show sessions" })[0]);
            await screen.findByText("s1234567");
            const open = icon();
            expect(open).not.toBe(closed);
            expect(closed).toMatch(/chevron-right/);
            expect(open).toMatch(/chevron-down/);
        });

        it("opens the clicked agent's sessions, not another's", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.click(rowOf(container, "fork-1").querySelector(".warden-history-agent button")!);
            await screen.findByText("s1234567");
            expect(HistoryAgentSessionsCommand).toHaveBeenCalledWith(
                {},
                { definition_id: "fork-1" },
                { timeout: 300_000 }
            );
            expect(
                rowOf(container, "fork-1").nextElementSibling!.classList.contains("warden-history-sessions-row")
            ).toBe(true);
        });

        it("says Reading the record… while the sessions are on their way", async () => {
            let finish!: (v: unknown) => void;
            HistoryAgentSessionsCommand.mockReturnValue(new Promise((r) => (finish = r)));
            mountAgents();
            await screen.findByText("Parent");
            fireEvent.click(screen.getAllByRole("button", { name: "Show sessions" })[0]);
            expect(await screen.findByText("Reading the record…")).toBeInTheDocument();
            finish(sessionsAnswer);
            await screen.findByText("s1234567");
            expect(screen.queryByText("Reading the record…")).toBeNull();
        });

        it("keeps an open row through a reload that drops another agent", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.click(rowOf(container, "parent-1").querySelector(".warden-history-agent button")!);
            await screen.findByText("s1234567");
            expect(HistoryAgentSessionsCommand).toHaveBeenCalledTimes(1);
            HistoryAgentsCommand.mockResolvedValue({
                agents: [
                    agent("parent-1", { name: "Parent", archives: 2, archive_bytes: 1024 }),
                    agent("other-1", { name: "Other" }),
                ],
            });
            fireMux("agents:changed");
            await vi.advanceTimersByTimeAsync(600);
            await waitFor(() => expect(container.querySelectorAll("tr[data-agent]")).toHaveLength(2));
            expect(rowOf(container, "fork-1")).toBeNull();
            expect(
                rowOf(container, "parent-1").nextElementSibling!.classList.contains("warden-history-sessions-row")
            ).toBe(true);
            expect(screen.getByText("s1234567")).toBeInTheDocument();
            expect(HistoryAgentSessionsCommand).toHaveBeenCalledTimes(1);
        });

        it("drops the sessions of a row that leaves the list", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.click(rowOf(container, "fork-1").querySelector(".warden-history-agent button")!);
            await screen.findByText("s1234567");
            HistoryAgentsCommand.mockResolvedValue({ agents: [agent("parent-1", { name: "Parent" })] });
            fireMux("agents:changed");
            await vi.advanceTimersByTimeAsync(600);
            await waitFor(() => expect(container.querySelectorAll("tr[data-agent]")).toHaveLength(1));
            expect(container.querySelector(".warden-history-sessions-row")).toBeNull();
        });
    });

    describe("Refresh", () => {
        it("reads the list again, shows busy while it does, and closes the open sessions", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            fireEvent.click(screen.getAllByRole("button", { name: "Show sessions" })[0]);
            await screen.findByText("s1234567");
            expect(HistoryAgentsCommand).toHaveBeenCalledTimes(1);

            let finish!: (v: unknown) => void;
            HistoryAgentsCommand.mockReturnValue(new Promise((r) => (finish = r)));
            const refresh = screen.getByRole("button", { name: "Refresh" });
            expect(refresh.getAttribute("aria-busy")).toBeNull();
            fireEvent.click(refresh);
            await waitFor(() => expect(refresh.getAttribute("aria-busy")).toBe("true"));
            expect((refresh as HTMLButtonElement).disabled).toBe(true);
            expect(container.querySelector(".warden-history-sessions-row")).not.toBeNull();

            finish({ agents: [agent("parent-1", { name: "Parent" })] });
            await waitFor(() => expect(refresh.getAttribute("aria-busy")).toBeNull());
            expect(HistoryAgentsCommand).toHaveBeenCalledTimes(2);
            expect(container.querySelector(".warden-history-sessions-row")).toBeNull();
            expect(screen.getAllByRole("button", { name: "Show sessions" })).toHaveLength(1);
        });

        it("stops being busy when the read fails, and shows the error", async () => {
            const { container } = mountAgents();
            await screen.findByText("Parent");
            HistoryAgentsCommand.mockRejectedValue(new Error("no route"));
            const refresh = screen.getByRole("button", { name: "Refresh" });
            fireEvent.click(refresh);
            await waitFor(() => expect(container.querySelector(".warden-manager-error")).not.toBeNull());
            expect(refresh.getAttribute("aria-busy")).toBeNull();
            expect(container.querySelector(".warden-manager-error")!.textContent).toBe("⚠ no route");
        });
    });

    describe("staying current", () => {
        it("loads nothing while hidden, follows once shown, and stops when hidden again", async () => {
            const [visible, setVisible] = createSignal(false);
            render(() => <HistoryAgents paneBlockId="pane-1" visible={visible()} />);
            await vi.advanceTimersByTimeAsync(1000);
            expect(HistoryAgentsCommand).not.toHaveBeenCalled();
            setVisible(true);
            await waitFor(() => expect(HistoryAgentsCommand).toHaveBeenCalledTimes(1));
            setVisible(false);
            fireMux("history:sessions:changed");
            await vi.advanceTimersByTimeAsync(40_000);
            expect(HistoryAgentsCommand).toHaveBeenCalledTimes(1);
        });

        it("reads again 500 ms after a session or an agent changes, not before", async () => {
            mountAgents();
            await waitFor(() => expect(HistoryAgentsCommand).toHaveBeenCalledTimes(1));
            fireMux("history:sessions:changed");
            await vi.advanceTimersByTimeAsync(499);
            expect(HistoryAgentsCommand).toHaveBeenCalledTimes(1);
            await vi.advanceTimersByTimeAsync(1);
            await waitFor(() => expect(HistoryAgentsCommand).toHaveBeenCalledTimes(2));
            fireMux("agents:changed");
            await vi.advanceTimersByTimeAsync(499);
            expect(HistoryAgentsCommand).toHaveBeenCalledTimes(2);
            await vi.advanceTimersByTimeAsync(1);
            await waitFor(() => expect(HistoryAgentsCommand).toHaveBeenCalledTimes(3));
        });

        it("reads again every 30 seconds as a safety net, for a write srv did not announce", async () => {
            mountAgents();
            await waitFor(() => expect(HistoryAgentsCommand).toHaveBeenCalledTimes(1));
            await vi.advanceTimersByTimeAsync(29_999);
            expect(HistoryAgentsCommand).toHaveBeenCalledTimes(1);
            await vi.advanceTimersByTimeAsync(1);
            await waitFor(() => expect(HistoryAgentsCommand).toHaveBeenCalledTimes(2));
            await vi.advanceTimersByTimeAsync(30_000);
            await waitFor(() => expect(HistoryAgentsCommand).toHaveBeenCalledTimes(3));
        });
    });
});
