// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { SidePanel } from "./SidePanel";

afterEach(cleanup);

describe("SidePanel", () => {
    it("is a labelled panel with a heading, a lead and its content", () => {
        render(() => (
            <SidePanel label="Task 12" lead={<span>#12</span>} title="Fix the thing" onClose={() => {}}>
                <p>the body</p>
            </SidePanel>
        ));
        const panel = screen.getByLabelText("Task 12");
        expect(panel.tagName).toBe("ASIDE");
        expect(panel.querySelector(".ui-side-panel__head")?.textContent).toContain("#12");
        expect(screen.getByRole("heading", { name: "Fix the thing" })).toBeTruthy();
        expect(screen.getByText("the body")).toBeTruthy();
    });

    it("closes from its head's close button", () => {
        const onClose = vi.fn();
        render(() => <SidePanel label="Panel" title="Title" onClose={onClose} />);
        fireEvent.click(screen.getByLabelText("Close"));
        expect(onClose).toHaveBeenCalledOnce();
    });

    it("takes a class of its own beside the kit's", () => {
        render(() => <SidePanel label="Panel" title="Title" class="kanban-drawer" onClose={() => {}} />);
        const panel = screen.getByLabelText("Panel");
        expect(panel.classList.contains("ui-side-panel")).toBe(true);
        expect(panel.classList.contains("kanban-drawer")).toBe(true);
    });

    it("has no lead when none is given", () => {
        render(() => <SidePanel label="Panel" title="Title" onClose={() => {}} />);
        expect(screen.getByLabelText("Panel").querySelector(".ui-side-panel__head")?.children.length).toBe(2);
    });
});
