// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
// The View popover: presets, one control per switch, Follow default and Make this the default.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AgentViewControls } from "../agent-view-controls";
import { VIEW_PRESETS, type AgentView, type ResolvedView } from "../virtualization/verbosity";
import { AgentViewPopover } from "./AgentViewPopover";

function controls(resolved: ResolvedView, defaultView: AgentView = VIEW_PRESETS.full): AgentViewControls & { calls: Record<string, ReturnType<typeof vi.fn>> } {
    const [open, setOpen] = createSignal(true);
    const calls = {
        setPreset: vi.fn(),
        setSwitch: vi.fn(),
        followDefault: vi.fn(),
        makeDefault: vi.fn(),
        cycle: vi.fn(),
    };
    return {
        resolved: () => resolved,
        defaultView: () => defaultView,
        ...calls,
        popoverOpen: open,
        setPopoverOpen: setOpen,
        calls,
    } as never;
}

const mount = (c: AgentViewControls) => render(() => <AgentViewPopover controls={c} anchor={document.body} />);
const radio = (name: string, group?: string) => {
    const scope = group ? screen.getByRole("radiogroup", { name: group }) : document.body;
    return Array.from(scope.querySelectorAll<HTMLElement>("[role=radio]")).find((el) => el.textContent === name)!;
};

afterEach(cleanup);

describe("AgentViewPopover", () => {
    it("lights the preset the five switches make, and Custom when they match none", () => {
        mount(controls({ view: VIEW_PRESETS.standard, source: "pane" }));
        expect(radio("Standard", "Preset").getAttribute("aria-checked")).toBe("true");
        cleanup();
        mount(controls({ view: { ...VIEW_PRESETS.standard, tools: "hidden" }, source: "pane" }));
        expect(radio("Custom", "Preset").getAttribute("aria-checked")).toBe("true");
    });

    it("sets a preset on a click, but never offers Custom as a choice", () => {
        const c = controls({ view: VIEW_PRESETS.full, source: "default" });
        mount(c);
        fireEvent.click(radio("Replies", "Preset"));
        expect(c.calls.setPreset).toHaveBeenCalledWith("replies");
        expect((radio("Custom", "Preset") as HTMLButtonElement).disabled).toBe(true);
    });

    it("changes one switch at a time", () => {
        const c = controls({ view: VIEW_PRESETS.full, source: "default" });
        mount(c);
        fireEvent.click(radio("Hidden", "Tool calls"));
        expect(c.calls.setSwitch).toHaveBeenCalledWith("tools", "hidden");
        fireEvent.click(radio("Hidden", "AgentMux notes"));
        expect(c.calls.setSwitch).toHaveBeenCalledWith("notes", "hidden");
    });

    it("offers Follow default only to a pane that keeps its own view", () => {
        const own = controls({ view: VIEW_PRESETS.replies, source: "pane" });
        mount(own);
        fireEvent.click(screen.getByText("Follow default"));
        expect(own.calls.followDefault).toHaveBeenCalledTimes(1);
        cleanup();
        mount(controls({ view: VIEW_PRESETS.full, source: "default" }));
        expect(screen.queryByText("Follow default")).toBeNull();
        expect(screen.getByText("Follows the default")).toBeTruthy();
    });

    it("makes the pane's view the default, unless that is already so", () => {
        const c = controls({ view: VIEW_PRESETS.standard, source: "pane" });
        mount(c);
        fireEvent.click(screen.getByText("Make this the default"));
        expect(c.calls.makeDefault).toHaveBeenCalledTimes(1);
        cleanup();
        mount(controls({ view: VIEW_PRESETS.full, source: "default" }));
        expect((screen.getByText("Make this the default") as HTMLButtonElement).disabled).toBe(true);
    });

    it("closes on Esc", () => {
        const c = controls({ view: VIEW_PRESETS.full, source: "default" });
        mount(c);
        fireEvent.keyDown(document, { key: "Escape" });
        expect(c.popoverOpen()).toBe(false);
    });
});
