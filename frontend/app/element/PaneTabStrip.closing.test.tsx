// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The closing look: a pill whose close was clicked dims and shows a spinner where its × was, and the
 * header's end buttons give way to "Closing…" / "Closing N tabs…".
 * SPEC_PANE_CLOSING_INDICATOR_AND_FASTER_CLOSE_2026_10_28.md §2.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ClosingLabel } from "@/app/block/closing-label";
import { PaneTabStrip } from "./PaneTabStrip";

afterEach(() => cleanup());

const TABS = [{ id: "a" }, { id: "b" }];

describe("a closing tab's pill", () => {
    it("dims, shows a spinner in place of the ×, and its close does nothing", () => {
        const onClose = vi.fn();
        const [closing, setClosing] = createSignal<string | null>(null);
        const { container } = render(() => (
            <PaneTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: { id: string }) => t.id}
                getLabel={(t: { id: string }) => t.id}
                onActivate={vi.fn()}
                onClose={onClose}
                isClosing={(t: { id: string }) => closing() === t.id}
            />
        ));
        const pills = () => [...container.querySelectorAll(".pane-tab")];
        expect(pills().some((p) => p.classList.contains("pane-tab--closing"))).toBe(false);
        expect(container.querySelector(".pane-tab-close .fa-xmark")).not.toBeNull();

        setClosing("b");
        const [a, b] = pills();
        expect(a.classList.contains("pane-tab--closing")).toBe(false);
        expect(b.classList.contains("pane-tab--closing")).toBe(true);
        const closeB = b.querySelector<HTMLButtonElement>(".pane-tab-close")!;
        expect(closeB.querySelector(".fa-spinner")).not.toBeNull();
        expect(closeB.querySelector(".fa-xmark")).toBeNull();
        expect(closeB.disabled).toBe(true);
        fireEvent.click(closeB);
        expect(onClose).not.toHaveBeenCalled();
        // The other pill still closes.
        fireEvent.click(a.querySelector(".pane-tab-close")!);
        expect(onClose).toHaveBeenCalledWith("a");
    });
});

describe("ClosingLabel", () => {
    it("says Closing… for one tab", () => {
        render(() => <ClosingLabel tabs={1} />);
        expect(screen.getByText("Closing…")).toBeInTheDocument();
    });

    it("counts the tabs of a stacked pane, and the title carries the words when the row clips the label", () => {
        const { container } = render(() => <ClosingLabel tabs={5} />);
        expect(screen.getByText("Closing 5 tabs…")).toBeInTheDocument();
        expect(container.querySelector(".block-frame-closing")?.getAttribute("title")).toBe("Closing 5 tabs…");
        expect(container.querySelector(".fa-spin")).not.toBeNull();
    });
});
