// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentSearchBar, LIVE_COUNT_MS } from "./AgentSearchBar";

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

    it("counts the matches when none is current yet", () => {
        expect(bar(5, -1).counter()).toBe("5 matches");
        expect(bar(1, -1).counter()).toBe("1 match");
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

describe("AgentSearchBar: for assistive technology", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("is a labelled search region with a labelled field", () => {
        const { view } = bar(3, 0);
        const region = view.container.querySelector("[role=search]");
        expect(region?.getAttribute("aria-label")).toBe("Find in conversation");
        expect(view.container.querySelector("input")?.getAttribute("aria-label")).toBe("Find in conversation");
    });

    it("announces the count in a polite status region, and hides the visible counter from readers", () => {
        const { view } = bar(41, 2);
        const live = view.container.querySelector(".agent-search-live") as HTMLElement;
        expect(live.getAttribute("role")).toBe("status");
        expect(live.getAttribute("aria-live")).toBe("polite");
        expect(live.textContent).toBe("3 of 41");
        expect(view.container.querySelector(".agent-search-counter")?.getAttribute("aria-hidden")).toBe("true");
    });

    it("changes the announcement at most once per interval, ending on the latest count", () => {
        const { view, setC, setI } = bar(10, 0);
        const live = () => view.container.querySelector(".agent-search-live")?.textContent;
        vi.advanceTimersByTime(LIVE_COUNT_MS);
        // Typing: the count changes five times inside the interval.
        for (let n = 9; n >= 5; n--) {
            setC(n);
            setI(0);
            vi.advanceTimersByTime(20);
        }
        expect(view.container.querySelector(".agent-search-counter")?.textContent).toBe("1 of 5");
        // The first change is announced at once, the rest wait for the interval.
        expect(live()).toBe("1 of 9");
        vi.advanceTimersByTime(LIVE_COUNT_MS);
        expect(live()).toBe("1 of 5");
    });
});

describe("AgentSearchBar dim switch", () => {
    function withDim(initial: boolean) {
        const [dim, setDim] = createSignal(initial);
        const onDimChange = vi.fn((next: boolean) => setDim(next));
        const view = render(() => (
            <AgentSearchBar
                visible={() => true}
                matchIndex={() => 0}
                matchCount={() => 3}
                onSearch={vi.fn()}
                onNext={vi.fn()}
                onPrev={vi.fn()}
                onClose={vi.fn()}
                dim={dim}
                onDimChange={onDimChange}
            />
        ));
        const toggle = () => view.container.querySelector('button[aria-label="Dim rows that do not match"]') as HTMLButtonElement;
        return { view, toggle, onDimChange };
    }

    it("shows the state as pressed and flips it on a click", () => {
        const { toggle, onDimChange } = withDim(true);
        expect(toggle().getAttribute("aria-pressed")).toBe("true");
        toggle().click();
        expect(onDimChange).toHaveBeenCalledWith(false);
        expect(toggle().getAttribute("aria-pressed")).toBe("false");
    });

    it("sits between the step buttons and close, in the focus order", () => {
        const { view } = withDim(true);
        const labels = [...view.container.querySelectorAll("button")].map((b) => b.getAttribute("aria-label"));
        const at = (text: string) => labels.findIndex((l) => l?.startsWith(text));
        expect(at("Next match")).toBeLessThan(at("Dim rows"));
        expect(at("Dim rows")).toBeLessThan(at("Close search"));
    });

    it("is absent where the pane gives no dim state", () => {
        const view = render(() => (
            <AgentSearchBar visible={() => true} matchIndex={() => 0} matchCount={() => 1} onSearch={vi.fn()} onNext={vi.fn()} onPrev={vi.fn()} onClose={vi.fn()} />
        ));
        expect(view.container.querySelector('button[aria-label="Dim rows that do not match"]')).toBeNull();
    });
});
