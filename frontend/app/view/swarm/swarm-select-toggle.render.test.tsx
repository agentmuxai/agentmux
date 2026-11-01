// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Select all / Select none toggle under the filter, beside Group by. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SwarmViewModel } from "./swarm-model";
import { SwarmSelectToggle } from "./swarm-select-toggle";

afterEach(() => cleanup());

function modelStub(selectedKeys: string[]) {
    const [selected, setSelected] = createSignal<Set<string>>(new Set(selectedKeys));
    return {
        selectedBlockIdsAtom: selected,
        selectAll: vi.fn((ids: string[]) => setSelected(new Set(ids))),
        clearSelection: vi.fn(() => setSelected(new Set<string>())),
        setManySelected: vi.fn(),
    } as unknown as SwarmViewModel;
}

describe("SwarmSelectToggle", () => {
    it("is reachable with nothing selected yet, and selects every listed agent; the label then flips", () => {
        const model = modelStub([]);
        const { getByText, queryByText } = render(() => <SwarmSelectToggle model={model} ids={() => ["a", "b", "c"]} />);
        fireEvent.click(getByText("Select all"));
        expect(model.selectAll).toHaveBeenCalledWith(["a", "b", "c"]);
        expect(queryByText("Select none")).not.toBeNull();
        expect(queryByText("Select all")).toBeNull();
    });

    it("reads Select none once everything is selected, and clears on click", () => {
        const model = modelStub(["a", "b"]);
        const { getByText } = render(() => <SwarmSelectToggle model={model} ids={() => ["a", "b"]} />);
        fireEvent.click(getByText("Select none"));
        expect(model.clearSelection).toHaveBeenCalled();
    });

    it("reads Select all while only some are selected", () => {
        const { queryByText } = render(() => <SwarmSelectToggle model={modelStub(["a"])} ids={() => ["a", "b"]} />);
        expect(queryByText("Select all")).not.toBeNull();
    });

    it("is one small button the keyboard reaches, in a box of its own so Group by sits beside it", () => {
        const { container } = render(() => <SwarmSelectToggle model={modelStub([])} ids={() => ["a"]} />);
        const button = container.querySelector<HTMLButtonElement>(".swarm-select-toggle button")!;
        expect(button.tabIndex).toBe(0);
        expect(button.classList.contains("ui-density-compact")).toBe(true);
    });

    it("is not there when no agent is listed", () => {
        const { container } = render(() => <SwarmSelectToggle model={modelStub([])} ids={() => []} />);
        expect(container.querySelector(".swarm-select-toggle")).toBeNull();
    });
});
