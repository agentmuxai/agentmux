// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it } from "vitest";

import type { AmbientOutcomes } from "@/app/store/ambient-outcomes";
import type { SwarmViewModel } from "./swarm-model";
import { SwarmStatsPanel } from "./swarm-stats";

afterEach(() => cleanup());

function modelStub(outcomes: AmbientOutcomes | null) {
    const [open, setOpen] = createSignal(false);
    const model = { ambientOutcomesAtom: () => outcomes, statsOpenAtom: open } as unknown as SwarmViewModel;
    return { model, setOpen };
}

describe("SwarmStatsPanel", () => {
    it("lists every kind of call, with the total, and opens and closes in place", () => {
        const { model, setOpen } = modelStub({
            activity_summary: { accepted: 10, kept: 30 },
            next_prompt_suggestion: { accepted: 5, timeout: 1 },
        });
        const { container, getByText } = render(() => <SwarmStatsPanel model={model} />);
        const panel = container.querySelector(".swarm-stats")!;
        // Solid sets `inert` as a property; Chromium reflects it, jsdom doesn't.
        const inner = container.querySelector(".swarm-stats-inner") as HTMLElement & { inert?: boolean };
        expect(panel.hasAttribute("data-open")).toBe(false);
        expect(inner.inert).toBe(true);
        expect(getByText(/since its server started · 46/)).not.toBeNull();
        expect(getByText("Session titles")).not.toBeNull();
        expect(getByText("10 accepted · 30 kept")).not.toBeNull();
        expect(getByText("5 accepted · 0 kept · 1 failed")).not.toBeNull();

        setOpen(true);
        expect(panel.hasAttribute("data-open")).toBe(true);
        expect(inner.inert).toBe(false);
    });

    it("marks a failing kind of call", () => {
        const { model } = modelStub({ activity_summary_pushed: { accepted: 1, cli_failed: 4 } });
        const { container } = render(() => <SwarmStatsPanel model={model} />);
        expect(container.querySelector(".swarm-stats-counts--warn")?.textContent).toBe("1 accepted · 0 kept · 4 failed");
    });

    it("renders nothing before any call has ended", () => {
        for (const outcomes of [null, {}]) {
            const { container } = render(() => <SwarmStatsPanel model={modelStub(outcomes).model} />);
            expect(container.querySelector(".swarm-stats")).toBeNull();
            cleanup();
        }
    });
});
