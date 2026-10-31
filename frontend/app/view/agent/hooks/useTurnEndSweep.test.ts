// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
// A tool still running after its turn ended is scrubbed, unless a new turn began meanwhile.

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { ORPHAN_SCRUB_DELAY_MS, useTurnEndSweep } from "./useTurnEndSweep";

function fakePane(working: { value: boolean }) {
    const dispatchDoc = vi.fn();
    const paneModel = {
        dispatchDoc,
        get state() {
            return { turnPhase: working.value ? { kind: "Streaming" } : { kind: "Idle" } };
        },
    } as unknown as AgentPaneModel;
    return { paneModel, dispatchDoc };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("useTurnEndSweep", () => {
    // An effect runs once its root has finished, so each test mounts, then acts.
    const mount = (paneModel: AgentPaneModel, onTurnEnded?: () => void) => {
        const [n, setN] = createSignal(0);
        const dispose = createRoot((d) => {
            useTurnEndSweep({ turnEnded: n, paneModel, onTurnEnded });
            return d;
        });
        return { setN, dispose };
    };

    it("does nothing before a turn has ended", () => {
        const { paneModel, dispatchDoc } = fakePane({ value: false });
        const { dispose } = mount(paneModel);
        vi.advanceTimersByTime(ORPHAN_SCRUB_DELAY_MS * 2);
        expect(dispatchDoc).not.toHaveBeenCalled();
        dispose();
    });

    it("scrubs running tools after the delay, and tells the caller the turn ended", () => {
        const { paneModel, dispatchDoc } = fakePane({ value: false });
        const onTurnEnded = vi.fn();
        const { setN, dispose } = mount(paneModel, onTurnEnded);
        setN(1);
        expect(onTurnEnded).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(ORPHAN_SCRUB_DELAY_MS - 1);
        expect(dispatchDoc).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(dispatchDoc).toHaveBeenCalledWith(expect.objectContaining({ type: "ScrubOrphanedInProgress", scope: "tools-only" }));
        dispose();
    });

    it("skips the pass when a new turn started in the meantime", () => {
        const working = { value: false };
        const { paneModel, dispatchDoc } = fakePane(working);
        const { setN, dispose } = mount(paneModel);
        setN(1);
        working.value = true;
        vi.advanceTimersByTime(ORPHAN_SCRUB_DELAY_MS);
        expect(dispatchDoc).not.toHaveBeenCalled();
        dispose();
    });

    it("cancels the pass when the pane is disposed first", () => {
        const { paneModel, dispatchDoc } = fakePane({ value: false });
        const { setN, dispose } = mount(paneModel);
        setN(1);
        dispose();
        vi.advanceTimersByTime(ORPHAN_SCRUB_DELAY_MS * 2);
        expect(dispatchDoc).not.toHaveBeenCalled();
    });
});
