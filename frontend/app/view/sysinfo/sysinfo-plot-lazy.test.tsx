// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const plot = vi.hoisted(() => ({ fail: true }));
vi.mock("./sysinfo-plot", () => {
    if (plot.fail) {
        plot.fail = false;
        throw new Error("chunk missing");
    }
    return { TimeChart: () => <div>time chart</div>, SingleLinePlot: () => <div>line plot</div> };
});

import { SingleLinePlot } from "./sysinfo-plot-lazy";

afterEach(() => cleanup());

describe("sysinfo-plot-lazy", () => {
    it("shows a failed chart load in its place, and Try again loads it anew", async () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        render(() => <SingleLinePlot {...({} as any)} />);
        await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("Chart failed to load"));
        fireEvent.click(screen.getByText("Try again"));
        await waitFor(() => expect(screen.getByText("line plot")).toBeTruthy());
    });
});
