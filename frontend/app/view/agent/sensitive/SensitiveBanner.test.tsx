// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Handler = (event: MuxEvent) => void;
const handlers = new Map<string, Handler>();
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (eventType: string, handler: (data: unknown) => void) => {
        handlers.set(eventType, (e: any) => {
            if (e?.data != null) handler(e.data);
        });
        return () => handlers.delete(eventType);
    },
}));
let resolveHistory: Array<(events: MuxEvent[]) => void> = [];
const decide = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        EventReadHistoryCommand: () => new Promise<MuxEvent[]>((r) => resolveHistory.push(r)),
        SensitiveDecideCommand: (...a: unknown[]) => decide(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const tone = vi.fn();
vi.mock("@/app/notification/sound/sound-service", () => ({ playShutdownPendingTone: () => tone() }));

import { SensitiveBanner } from "./SensitiveBanner";
import { CHIME_DELAY_MS } from "./sensitive-pending";
import type { MuxEvent } from "@/types/rpc/MuxEvent";

const flush = () => new Promise((r) => setTimeout(r, 0));
const afterChimeDelay = () => new Promise((r) => setTimeout(r, CHIME_DELAY_MS + 100));

const pending = (id: string, over: Record<string, unknown> = {}) =>
    ({
        event: "sensitive:pending",
        data: {
            request_id: id,
            op: "upgrade",
            summary: "upgrade AgentMux (version 0.61.0)",
            requester: "Parlo",
            approver: "Camper",
            deadline_ms: Date.now() + 30_000,
            opened_ms: Date.now(),
            status: "pending",
            ...over,
        },
    }) as unknown as MuxEvent;
const cleared = (id: string) => ({ event: "sensitive:cleared", data: { request_id: id, outcome: "vetoed" } }) as unknown as MuxEvent;

describe("SensitiveBanner (SPEC_SUPER_AGENT §5)", () => {
    beforeEach(() => {
        handlers.clear();
        resolveHistory = [];
        vi.clearAllMocks();
    });
    afterEach(cleanup);

    it("shows the countdown with who asked and who approved, and chimes once for two panes", async () => {
        render(() => (
            <>
                <SensitiveBanner />
                <SensitiveBanner />
            </>
        ));
        handlers.get("sensitive:pending")!(pending("r1"));
        expect(screen.getAllByText(/Parlo asked: upgrade AgentMux \(version 0\.61\.0\)\. Approved by Camper\. Starts in (29|30|31) s\./)).toHaveLength(2);
        await afterChimeDelay();
        expect(tone).toHaveBeenCalledTimes(1);
    });

    it("marks itself for the person, so an agent's UI click, query and key presses leave it alone", () => {
        const { container } = render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1"));
        expect(container.querySelector(".pane-row")?.hasAttribute("data-operator-only")).toBe(true);
    });

    it("shows srv's words as text: markup in them creates no element", () => {
        const { container } = render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1", { summary: "x <b>y</b><img src=x onerror=alert(1)>" }));
        expect(screen.getByText(/x <b>y<\/b><img src=x onerror=alert\(1\)>/)).toBeTruthy();
        expect(container.querySelector("b, img")).toBeNull();
    });

    it("says when no super agent reviewed it", () => {
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1", { approver: undefined }));
        expect(screen.getByText(/No super agent reviewed this\./)).toBeTruthy();
    });

    it("a countdown already past its deadline, replayed on connect, is not shown or sounded", async () => {
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("old", { deadline_ms: Date.now() - 1000 }));
        await afterChimeDelay();
        expect(screen.queryByText(/Parlo asked/)).toBeNull();
        expect(tone).not.toHaveBeenCalled();
    });

    it("a replayed countdown whose clear follows right behind it stays silent", async () => {
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1"));
        handlers.get("sensitive:cleared")!(cleared("r1"));
        await afterChimeDelay();
        expect(tone).not.toHaveBeenCalled();
    });

    it("a countdown replayed on connect, begun a while ago and still running, is shown but not sounded", async () => {
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1", { opened_ms: Date.now() - 20_000 }));
        await afterChimeDelay();
        expect(screen.queryByText(/Parlo asked/)).toBeTruthy();
        expect(tone).not.toHaveBeenCalled();
    });

    it("a history read that lands after the last banner went doesn't touch the next one", async () => {
        const first = render(() => <SensitiveBanner />);
        first.unmount();
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r2"));
        handlers.get("sensitive:cleared")!(cleared("r2"));
        // The first banner's history read, answered late, still lists r2 as pending.
        resolveHistory[0]([pending("r2")]);
        resolveHistory[1]([]);
        await flush();
        expect(screen.queryByText(/Parlo asked/)).toBeNull();
    });

    it("a clear that arrives first keeps its countdown from showing", () => {
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:cleared")!(cleared("r1"));
        handlers.get("sensitive:pending")!(pending("r1"));
        expect(screen.queryByText(/Parlo asked/)).toBeNull();
    });

    it("a cleared countdown goes away", () => {
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1"));
        handlers.get("sensitive:cleared")!(cleared("r1"));
        expect(screen.queryByText(/Parlo asked/)).toBeNull();
    });

    it("a pane opened mid-countdown shows every open countdown from history, without a chime", async () => {
        render(() => <SensitiveBanner />);
        resolveHistory[0]([pending("r1", { summary: "first thing" }), pending("r2", { summary: "second thing" })]);
        resolveHistory[1]([cleared("r2")]);
        await flush();
        expect(screen.queryByText(/first thing/)).toBeTruthy();
        expect(screen.queryByText(/second thing/)).toBeNull();
        expect(tone).not.toHaveBeenCalled();
    });

    it("a live event while history loads doesn't cost the countdowns it didn't mention", async () => {
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r2", { summary: "second thing" }));
        handlers.get("sensitive:cleared")!(cleared("r2"));
        resolveHistory[0]([pending("r1", { summary: "first thing" }), pending("r2", { summary: "second thing" })]);
        resolveHistory[1]([]);
        await flush();
        expect(screen.queryByText(/first thing/)).toBeTruthy();
        expect(screen.queryByText(/second thing/), "cleared live, not brought back by the replay").toBeNull();
    });

    it("Veto asks srv once and hides the banner", async () => {
        decide.mockResolvedValue({ outcome: "vetoed" });
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1"));
        const button = screen.getByText("Veto");
        fireEvent.click(button);
        fireEvent.click(button);
        expect(decide).toHaveBeenCalledTimes(1);
        expect(decide.mock.calls[0][1]).toEqual({ request_id: "r1", decision: "veto" });
        await flush();
        expect(screen.queryByText(/Parlo asked/)).toBeNull();
    });

    it("Approve now asks srv to skip the wait and leaves the banner to the clear event", async () => {
        decide.mockResolvedValue({ outcome: "approved_now" });
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1"));
        fireEvent.click(screen.getByText("Approve now"));
        expect(decide.mock.calls[0][1]).toEqual({ request_id: "r1", decision: "approve_now" });
        await flush();
        expect(screen.queryByText(/Parlo asked/)).toBeTruthy();
    });

    it("the subscription ends when the last banner goes", () => {
        const { unmount } = render(() => <SensitiveBanner />);
        expect(handlers.has("sensitive:pending")).toBe(true);
        unmount();
        expect(handlers.has("sensitive:pending")).toBe(false);
    });
});
