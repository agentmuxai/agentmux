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

import { SingleLinePlot, TimeChart } from "./sysinfo-plot-lazy";

afterEach(() => cleanup());

describe("sysinfo-plot-lazy", () => {
    it("shows a failed load in every chart's place, and one Try again draws them all", async () => {
        vi.spyOn(console, "error").mockImplementation(() => {});
        render(() => (
            <>
                <SingleLinePlot {...({} as any)} />
                <TimeChart {...({} as any)} />
            </>
        ));
        await waitFor(() => expect(screen.getAllByRole("alert")).toHaveLength(2));
        expect(screen.getAllByRole("alert")[0].textContent).toContain("Chart failed to load");
        fireEvent.click(screen.getAllByText("Try again")[0]);
        await waitFor(() => expect(screen.getByText("line plot")).toBeTruthy());
        expect(screen.getByText("time chart")).toBeTruthy();
        expect(screen.queryByRole("alert")).toBeNull();
    });
});
