// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { createStore } from "solid-js/store";
import { describe, expect, it, vi } from "vitest";
import { useFocusRepoll, useHeldMessageDelivery } from "./useTurnReconciliation";

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("useFocusRepoll", () => {
    function mount(status: { turn_active?: boolean } | null | Error) {
        const [focused, setFocused] = createSignal(true);
        const track = vi.fn();
        const flushRefresh = vi.fn();
        const getStatus = vi.fn(async () => {
            if (status instanceof Error) throw status;
            return status;
        });
        const dispose = createRoot((d) => {
            useFocusRepoll({
                blockId: "b1",
                windowFocused: focused,
                trackTurnJustEnded: track,
                flushPendingControllerRefresh: flushRefresh,
                getControllerStatus: getStatus,
            });
            return d;
        });
        return { setFocused, track, flushRefresh, getStatus, dispose };
    }

    it("doesn't poll at mount (the one-shot covers that)", async () => {
        const m = mount({ turn_active: false });
        await flush();
        expect(m.getStatus).not.toHaveBeenCalled();
        m.dispose();
    });

    it("on refocus, feeds the reading to the edge detector and flushes on confirmed idle", async () => {
        const m = mount({ turn_active: false });
        m.setFocused(false);
        m.setFocused(true);
        await flush();
        expect(m.track).toHaveBeenCalledWith(false);
        expect(m.flushRefresh).toHaveBeenCalledTimes(1);
        m.dispose();
    });

    it("doesn't flush while the backend reports an active turn", async () => {
        const m = mount({ turn_active: true });
        m.setFocused(false);
        m.setFocused(true);
        await flush();
        expect(m.track).toHaveBeenCalledWith(true);
        expect(m.flushRefresh).not.toHaveBeenCalled();
        m.dispose();
    });

    it("ignores a missing status and a failed RPC", async () => {
        for (const status of [null, new Error("down")]) {
            const m = mount(status);
            m.setFocused(false);
            m.setFocused(true);
            await flush();
            expect(m.track).not.toHaveBeenCalled();
            expect(m.flushRefresh).not.toHaveBeenCalled();
            m.dispose();
        }
    });
});

describe("useHeldMessageDelivery", () => {
    function mount(held: boolean) {
        const [state, setState] = createStore<{ currentTool: string | null; turnPhase: { kind: string } }>({
            currentTool: null,
            turnPhase: { kind: "Streaming" },
        });
        const flushHeld = vi.fn();
        const flushRefresh = vi.fn();
        const dispose = createRoot((d) => {
            useHeldMessageDelivery({
                paneModel: { state } as never,
                hasHeldMessages: () => held,
                flushHeldMessages: flushHeld,
                flushPendingControllerRefresh: flushRefresh,
            });
            return d;
        });
        return { setState, flushHeld, flushRefresh, dispose };
    }

    it("delivers held messages at the next tool-call boundary", () => {
        const m = mount(true);
        m.setState("currentTool", "Bash");
        expect(m.flushHeld).toHaveBeenCalledTimes(1);
        m.setState("currentTool", "Bash"); // same tool: not a new boundary
        expect(m.flushHeld).toHaveBeenCalledTimes(1);
        m.dispose();
    });

    it("falls back to turn end, and runs the deferred refresh there even with nothing held", () => {
        const held = mount(true);
        held.setState("turnPhase", { kind: "Done" });
        expect(held.flushHeld).toHaveBeenCalled();
        expect(held.flushRefresh).toHaveBeenCalled();
        held.dispose();

        const none = mount(false);
        none.setState("turnPhase", { kind: "Idle" });
        expect(none.flushHeld).not.toHaveBeenCalled();
        expect(none.flushRefresh).toHaveBeenCalled();
        none.dispose();
    });

    it("does nothing mid-turn with nothing held", () => {
        const m = mount(false);
        m.setState("currentTool", "Read");
        expect(m.flushHeld).not.toHaveBeenCalled();
        expect(m.flushRefresh).not.toHaveBeenCalled();
        m.dispose();
    });
});
