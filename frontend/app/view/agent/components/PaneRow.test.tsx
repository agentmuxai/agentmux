// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneRow — the shared auxiliary-pin row primitive.
 * Spec: docs/specs/SPEC_AGENT_PANE_FORKS_AND_AUX_PINS_2026_06_15.md §5.2.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PaneRow } from "./PaneRow";

afterEach(() => cleanup());

describe("PaneRow", () => {
    it("renders sigil, title, meta and tail", () => {
        render(() => (
            <PaneRow sigil="⑂" title="pr-422-review" meta="[0:42]" tail="ready on :5173" />
        ));
        expect(screen.getByText("⑂")).toBeInTheDocument();
        expect(screen.getByText("pr-422-review")).toBeInTheDocument();
        expect(screen.getByText("[0:42]")).toBeInTheDocument();
        // Tail is prefixed with the → glyph in the same element.
        expect(screen.getByText(/ready on :5173/)).toBeInTheDocument();
    });

    it("omits meta and tail when not provided", () => {
        const { container } = render(() => <PaneRow sigil="⟩" title="task dev" />);
        expect(container.querySelector(".pane-row-meta")).toBeNull();
        expect(container.querySelector(".pane-row-tail")).toBeNull();
    });

    it("applies the status accent modifier class", () => {
        const { container } = render(() => (
            <PaneRow sigil="⟩" title="build" accent="error" />
        ));
        expect(container.querySelector(".pane-row--error")).toBeInTheDocument();
    });

    it("defaults to the neutral accent", () => {
        const { container } = render(() => <PaneRow sigil="⟩" title="x" />);
        expect(container.querySelector(".pane-row--neutral")).toBeInTheDocument();
    });

    it("fires onActivate when the summary is clicked", async () => {
        const onActivate = vi.fn();
        render(() => <PaneRow sigil="⑂" title="fork" onActivate={onActivate} />);
        const user = userEvent.setup();
        await user.click(screen.getByText("fork"));
        expect(onActivate).toHaveBeenCalledTimes(1);
    });

    it("fires the action handler but NOT onActivate (stopPropagation)", async () => {
        const onActivate = vi.fn();
        const onStop = vi.fn();
        render(() => (
            <PaneRow
                sigil="⟩"
                title="task dev"
                onActivate={onActivate}
                actions={[{ glyph: "■", title: "Stop", onClick: onStop }]}
            />
        ));
        const user = userEvent.setup();
        await user.click(screen.getByRole("button", { name: "Stop" }));
        expect(onStop).toHaveBeenCalledTimes(1);
        expect(onActivate).not.toHaveBeenCalled();
    });

    it("renders the body slot only when expanded", () => {
        const [expanded, setExpanded] = createSignal(false);
        render(() => (
            <PaneRow sigil="⟩" title="task dev" expanded={expanded()}>
                <div data-testid="body">live log</div>
            </PaneRow>
        ));
        expect(screen.queryByTestId("body")).toBeNull();
        setExpanded(true);
        expect(screen.getByTestId("body")).toBeInTheDocument();
    });

    it("marks the expanded row with the expanded modifier", () => {
        const { container } = render(() => (
            <PaneRow sigil="⟩" title="x" expanded={true}><span>b</span></PaneRow>
        ));
        expect(container.querySelector(".pane-row--expanded")).toBeInTheDocument();
    });
});
