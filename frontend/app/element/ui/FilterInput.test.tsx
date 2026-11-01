// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { FilterInput, SearchIcon } from "./FilterInput";

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

    it("carries the caller's test id on the input and the clear button", () => {
        renderFilter({ testId: "my-filter" }, "x");
        expect(screen.getByTestId("my-filter-input")).not.toBeNull();
        expect(screen.getByTestId("my-filter-clear")).not.toBeNull();
    });

    it("starts with the magnifier cell, which is the same one a search row outside the kit uses", () => {
        const { container } = renderFilter();
        const box = container.querySelector(".ui-filter")!;
        expect(box.firstElementChild?.classList.contains("ui-search-icon")).toBe(true);
        render(() => <SearchIcon class="mine" />);
        const loose = document.querySelectorAll(".ui-search-icon.mine");
        expect(loose).toHaveLength(1);
        expect(loose[0].getAttribute("aria-hidden")).toBe("true");
    });

    it("gives the caller the input, and lets its onKeyDown claim a key before Escape clears", () => {
        let el: HTMLInputElement | undefined;
        const seen: string[] = [];
        const { input, value } = renderFilter(
            {
                inputRef: (e) => (el = e),
                onKeyDown: (e) => {
                    seen.push(e.key);
                    if (e.key === "Escape") e.preventDefault();
                },
            },
            "split",
        );
        expect(el).toBe(input);
        fireEvent.keyDown(input, { key: "Escape" });
        expect(seen).toEqual(["Escape"]);
        expect(value()).toBe("split");
    });
});
