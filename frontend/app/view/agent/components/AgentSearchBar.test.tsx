// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AgentSearchBar } from "./AgentSearchBar";

afterEach(cleanup);

function bar(count: number, index: number, searching = false) {
    const [c, setC] = createSignal(count);
    const [i, setI] = createSignal(index);
    const [s, setS] = createSignal(searching);
    const props = { onSearch: vi.fn(), onNext: vi.fn(), onPrev: vi.fn(), onClose: vi.fn() };
    const view = render(() => (
        <AgentSearchBar visible={() => true} matchIndex={i} matchCount={c} searching={s} {...props} />
    ));
    const counter = () => view.container.querySelector(".agent-search-counter")?.textContent;
    return { counter, setC, setI, setS, props, view };
}

describe("AgentSearchBar", () => {
    it("counts the current match against the total", () => {
        const { counter, setC, setI } = bar(41, 2);
        expect(counter()).toBe("3 of 41");
        setC(42);
        setI(0);
        expect(counter()).toBe("1 of 42");
    });

    it("says No matches when nothing was found", () => {
        expect(bar(0, -1).counter()).toBe("No matches");
    });

    it("says Searching… while a long pass runs, then the count", () => {
        const { counter, setS } = bar(3, 0, true);
        expect(counter()).toBe("Searching…");
        setS(false);
        expect(counter()).toBe("1 of 3");
    });

    it("steps and closes from the keyboard", () => {
        const { props, view } = bar(3, 0);
        const input = view.container.querySelector("input") as HTMLInputElement;
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true }));
        input.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
        expect(props.onNext).toHaveBeenCalledOnce();
        expect(props.onPrev).toHaveBeenCalledOnce();
        expect(props.onClose).toHaveBeenCalledOnce();
    });
});
