// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Sysinfo's control row, the same in the simple and the Advanced views. */
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SysinfoBar, type SysinfoBarProps } from "./sysinfo-bar";
import { FOCUS_CHOICES } from "./sysinfo-overview-data";

afterEach(() => cleanup());

function mount(over: Partial<SysinfoBarProps> = {}) {
    const props: SysinfoBarProps = {
        views: FOCUS_CHOICES,
        view: "cpu",
        onView: vi.fn(),
        windowSecs: 120,
        intervalSecs: 1,
        onWindow: vi.fn(),
        paused: false,
        onPause: vi.fn(),
        table: false,
        onTable: vi.fn(),
        ...over,
    };
    render(() => <SysinfoBar {...props} />);
    return props;
}

describe("SysinfoBar", () => {
    it("offers the mode's views, then the window, the one shown selected", () => {
        mount();
        const radios = screen.getAllByRole("radio");
        expect(radios.map((r) => r.textContent)).toEqual(["CPU", "Memory", "Disk", "Network", "Everything", "1m", "2m", "5m", "15m"]);
        expect(screen.getByRole("radio", { name: "CPU" }).getAttribute("aria-checked")).toBe("true");
        expect(screen.getByRole("radio", { name: "2m" }).getAttribute("aria-checked")).toBe("true");
    });

    it("switches the view and the window in one click", () => {
        const props = mount();
        fireEvent.click(screen.getByRole("radio", { name: "Network" }));
        expect(props.onView).toHaveBeenCalledWith("net");
        fireEvent.click(screen.getByRole("radio", { name: "15m" }));
        expect(props.onWindow).toHaveBeenCalledWith(900);
    });

    it("pauses, says so, and offers to resume", () => {
        const props = mount();
        fireEvent.click(screen.getByRole("button", { name: "Pause" }));
        expect(props.onPause).toHaveBeenCalledWith(true);
        cleanup();
        mount({ paused: true });
        expect(screen.getByText("Paused")).toBeTruthy();
        expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
    });

    it("turns the table on and off", () => {
        const props = mount();
        fireEvent.click(screen.getByRole("button", { name: "Show as a table" }));
        expect(props.onTable).toHaveBeenCalledWith(true);
        cleanup();
        mount({ table: true });
        expect(screen.getByRole("button", { name: "Show charts" })).toBeTruthy();
    });
});
