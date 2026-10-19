// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const AgentSessionListArchivesCommand = vi.fn();
const HistoryAgentSessionsCommand = vi.fn();
const HistoryLocateCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentSessionListArchivesCommand: (...a: unknown[]) => AgentSessionListArchivesCommand(...a),
        HistoryAgentSessionsCommand: (...a: unknown[]) => HistoryAgentSessionsCommand(...a),
        HistoryLocateCommand: (...a: unknown[]) => HistoryLocateCommand(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { endOfDayMs, HistoryNavigator, type HistoryMode } from "./HistoryNavigator";

const session = (id: string, zone: string, first: number, end: number) => ({
    session_id: id,
    cwd: "",
    first_ms: first,
    last_ms: first + 1000,
    bytes: 10,
    archived: zone.includes(":archive:"),
    zone,
    end_byte: end,
});

describe("endOfDayMs", () => {
    it("is the last millisecond of that local day", () => {
        const end = endOfDayMs("2026-10-17")!;
        expect(new Date(end).getDate()).toBe(17);
        expect(new Date(end + 1).getDate()).toBe(18);
        expect(endOfDayMs("17/10/2026")).toBeNull();
    });
});

describe("HistoryNavigator", () => {
    const onJump = vi.fn();
    const onOpenArchive = vi.fn();
    const onLatest = vi.fn();
    const onShowNewer = vi.fn();

    beforeEach(() => {
        vi.clearAllMocks();
        AgentSessionListArchivesCommand.mockResolvedValue([
            { archive_zoneid: "agent:a1:archive:1000", archived_at_ms: 1000, preview: "the old one", node_count: 3 },
        ]);
        HistoryAgentSessionsCommand.mockResolvedValue({
            sessions: [
                session("cur-2", "agent:a1:current", 3000, 900),
                session("cur-1", "agent:a1:current", 2000, 400),
                session("arc-1", "agent:a1:archive:1000", 1000, 50),
            ],
            problems: [],
        });
        HistoryLocateCommand.mockResolvedValue({ line: 42, total: 100 });
    });
    afterEach(() => cleanup());

    function mount(mode: HistoryMode = "live", zone: string | null = null) {
        const [m] = createSignal<HistoryMode>(mode);
        const [z] = createSignal<string | null>(zone);
        return render(() => (
            <HistoryNavigator
                agentId="a1"
                readBlockId="tab-1"
                mode={m}
                archiveZone={z}
                hasNewer={() => true}
                onOpenArchive={onOpenArchive}
                onJump={onJump}
                onShowNewer={onShowNewer}
                onLatest={onLatest}
            />
        ));
    }

    it("counts the shown conversation's sessions and lists them oldest first", async () => {
        mount();
        await waitFor(() => expect(screen.getByText("2 sessions")).toBeInTheDocument());
        const jump = screen.getByRole("combobox", { name: "Jump to a session" }) as HTMLSelectElement;
        const labels = Array.from(jump.options).map((o) => o.textContent ?? "");
        expect(labels[1]).toMatch(/^Session cur-1/);
        expect(labels[2]).toMatch(/^Session cur-2/);
    });

    it("jumps to a session's end through history.locate", async () => {
        mount();
        await waitFor(() => expect(screen.getByText("2 sessions")).toBeInTheDocument());
        const jump = screen.getByRole("combobox", { name: "Jump to a session" });
        fireEvent.change(jump, { target: { value: "400" } });
        await waitFor(() => expect(onJump).toHaveBeenCalledWith(42));
        expect(HistoryLocateCommand).toHaveBeenCalledWith({}, { block_id: "tab-1", at_byte: 400 });
    });

    it("the jump list resets after a jump, so the same session can be picked again", async () => {
        mount();
        await waitFor(() => expect(screen.getByText("2 sessions")).toBeInTheDocument());
        const jump = screen.getByRole("combobox", { name: "Jump to a session" }) as HTMLSelectElement;
        fireEvent.change(jump, { target: { value: "400" } });
        await waitFor(() => expect(onJump).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(jump.value).toBe(""));
        fireEvent.change(jump, { target: { value: "400" } });
        await waitFor(() => expect(onJump).toHaveBeenCalledTimes(2));
    });

    it("jumps to the end of a date", async () => {
        mount();
        fireEvent.change(screen.getByLabelText("Jump to a date"), { target: { value: "2026-10-17" } });
        await waitFor(() => expect(onJump).toHaveBeenCalledWith(42));
        expect(HistoryLocateCommand).toHaveBeenCalledWith({}, { block_id: "tab-1", at_ms: endOfDayMs("2026-10-17") });
    });

    it("lists archived conversations and opens one", async () => {
        mount();
        const picker = (await screen.findByRole("combobox", { name: "Conversation" })) as HTMLSelectElement;
        expect(Array.from(picker.options).map((o) => o.textContent)).toEqual([
            "Current conversation",
            expect.stringMatching(/^Archived .* · the old one$/),
        ]);
        fireEvent.change(picker, { target: { value: "agent:a1:archive:1000" } });
        expect(onOpenArchive).toHaveBeenCalledWith("agent:a1:archive:1000");
    });

    it("in an archive, lists its sessions and jumps within it", async () => {
        mount("archive", "agent:a1:archive:1000");
        await waitFor(() => expect(screen.getByText("1 session")).toBeInTheDocument());
        fireEvent.change(screen.getByRole("combobox", { name: "Jump to a session" }), { target: { value: "50" } });
        await waitFor(() =>
            expect(HistoryLocateCommand).toHaveBeenCalledWith({}, { block_id: "tab-1", archive_zone: "agent:a1:archive:1000", at_byte: 50 })
        );
        fireEvent.click(screen.getByRole("button", { name: "Back to current" }));
        expect(onLatest).toHaveBeenCalled();
    });

    it("live shows no Show newer or Back to latest", async () => {
        mount("live");
        await waitFor(() => expect(screen.getByText("2 sessions")).toBeInTheDocument());
        expect(screen.queryByRole("button", { name: "Show newer" })).toBeNull();
        expect(screen.queryByRole("button", { name: /Back to/ })).toBeNull();
    });

    it("after a jump, Show newer and Back to latest", async () => {
        mount("jumped");
        fireEvent.click(screen.getByRole("button", { name: "Show newer" }));
        expect(onShowNewer).toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Back to latest" }));
        expect(onLatest).toHaveBeenCalled();
    });
});

describe("sessionLabel", () => {
    it("says when a session has no recorded times", async () => {
        const { sessionLabel } = await import("./HistoryNavigator");
        expect(sessionLabel({ ...session("abcdef123456", "z", 0, 1), last_ms: 0 })).toBe("Session abcdef12 · no time recorded");
    });
});
