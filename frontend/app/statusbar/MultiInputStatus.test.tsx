// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// "Type into all terminals" shows once, in the status bar, while it's on, and
// a click there turns it off. It used to be a button in every terminal's
// header, beside the tab strip's "+".

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ signal: null as null | ReturnType<typeof import("solid-js").createSignal<boolean>> }));
vi.mock("@/store/global", () => ({
    isTermMultiInput: () => h.signal![0](),
    setIsTermMultiInput: (v: boolean) => h.signal![1](v),
}));
vi.mock("@/app/keybindings", () => ({ shortcutFor: () => "Ctrl+Shift+Alt+M" }));

import { MultiInputStatus } from "./MultiInputStatus";

afterEach(cleanup);

describe("MultiInputStatus", () => {
    it("shows nothing while the mode is off", () => {
        h.signal = createSignal(false);
        render(() => <MultiInputStatus />);
        expect(screen.queryByText("Multi Input")).toBeNull();
    });

    it("shows while on, says how to turn it off, and a click turns it off", () => {
        h.signal = createSignal(true);
        render(() => <MultiInputStatus />);
        const item = screen.getByRole("button");
        expect(screen.getByText("Multi Input")).toBeTruthy();
        expect(item.getAttribute("data-tip")).toContain("Click to turn off (Ctrl+Shift+Alt+M)");
        fireEvent.click(item);
        expect(h.signal[0]()).toBe(false);
        expect(screen.queryByText("Multi Input")).toBeNull();
    });
});
