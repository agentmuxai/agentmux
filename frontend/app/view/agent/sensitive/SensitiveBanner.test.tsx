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
import type { MuxEvent } from "@/types/rpc/MuxEvent";

const flush = () => new Promise((r) => setTimeout(r, 0));

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

    it("shows the countdown with who asked and who approved, and chimes once for two panes", () => {
        render(() => (
            <>
                <SensitiveBanner />
                <SensitiveBanner />
            </>
        ));
        handlers.get("sensitive:pending")!(pending("r1"));
        expect(screen.getAllByText(/Parlo asked: upgrade AgentMux \(version 0\.61\.0\)\. Approved by Camper\. Starts in (29|30|31) s\./)).toHaveLength(2);
        expect(tone).toHaveBeenCalledTimes(1);
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

    it("a cleared countdown goes away", () => {
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1"));
        handlers.get("sensitive:cleared")!(cleared("r1"));
        expect(screen.queryByText(/Parlo asked/)).toBeNull();
    });

    it("a pane opened mid-countdown shows it from history, without a chime", async () => {
        render(() => <SensitiveBanner />);
        resolveHistory[0]([pending("r1")]);
        resolveHistory[1]([]);
        await flush();
        expect(screen.queryByText(/Parlo asked/)).toBeTruthy();
        expect(tone).not.toHaveBeenCalled();
    });

    it("a history replay that lands after a live clear doesn't bring it back", async () => {
        render(() => <SensitiveBanner />);
        handlers.get("sensitive:pending")!(pending("r1"));
        handlers.get("sensitive:cleared")!(cleared("r1"));
        resolveHistory[0]([pending("r1")]);
        resolveHistory[1]([]);
        await flush();
        expect(screen.queryByText(/Parlo asked/)).toBeNull();
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
