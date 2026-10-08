// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FilterInput } from "./FilterInput";

afterEach(() => cleanup());

function renderFilter(extra: Partial<Parameters<typeof FilterInput>[0]> = {}, initial = "") {
    const [value, setValue] = createSignal(initial);
    const onInput = vi.fn((q: string) => setValue(q));
    const result = render(() => <FilterInput value={value()} onInput={onInput} placeholder="Filter things" {...extra} />);
    return { ...result, value, onInput, input: screen.getByLabelText("Filter things") as HTMLInputElement };
}

describe("FilterInput", () => {
    it("is the pane's typing target by default, in a bordered box", () => {
        const { input, container } = renderFilter();
        expect(input.hasAttribute("data-pane-focus")).toBe(true);
        expect(container.querySelector(".ui-filter")).not.toBeNull();
    });

    it("can opt out of being the pane's typing target", () => {
        const { input } = renderFilter({ paneFocus: false });
        expect(input.hasAttribute("data-pane-focus")).toBe(false);
    });

    it("reports what is typed", () => {
        const { input, value } = renderFilter();
        fireEvent.input(input, { target: { value: "split" } });
        expect(value()).toBe("split");
    });

    it("shows a clear button only while there is text, and it clears", () => {
        const { value } = renderFilter({}, "split");
        fireEvent.click(screen.getByLabelText("Clear filter"));
        expect(value()).toBe("");
        expect(screen.queryByLabelText("Clear filter")).toBeNull();
    });

    it("Escape clears a non-empty filter (handled); on an empty one it is left to others", () => {
        const { input, value } = renderFilter({}, "split");
        expect(fireEvent.keyDown(input, { key: "Escape" })).toBe(false); // defaultPrevented
        expect(value()).toBe("");
        expect(fireEvent.keyDown(input, { key: "Escape" })).toBe(true);
    });

    it("bare: just the parts, carrying the caller's classes and test ids", () => {
        const { container } = renderFilter({ bare: true, inputClass: "my-input", testId: "my-filter" }, "x");
        expect(container.querySelector(".ui-filter")).toBeNull();
        expect(screen.getByTestId("my-filter-input").classList.contains("my-input")).toBe(true);
        expect(screen.getByTestId("my-filter-clear")).not.toBeNull();
    });
});
