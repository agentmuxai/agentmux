// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
// The View popover: presets, one control per switch, Follow default and Make this the default.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AgentViewControls } from "../agent-view-controls";
import { VIEW_PRESETS, type AgentView, type ResolvedView } from "../virtualization/verbosity";
import { MAKE_DEFAULT_DONE_TIP, MAKE_DEFAULT_TIP } from "../virtualization/view-options";
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

describe("AgentViewPopover in a narrow pane", () => {
    const scss = readFileSync(resolve(__dirname, "AgentViewPopover.scss"), "utf8");

    it("is 280px wide, or as wide as its pane when the pane is narrower, and scrolls rather than overflows", () => {
        const rule = scss.slice(scss.indexOf("menu.agent-view-popover"), scss.indexOf("}", scss.indexOf("menu.agent-view-popover")));
        expect(rule).toContain("min-width: min(280px, var(--popover-room-width, 280px))");
        expect(rule).toContain("max-width: var(--popover-room-width, none)");
        expect(rule).toContain("overflow-x: auto");
        const foot = scss.slice(scss.indexOf(".agent-view-popover-foot"), scss.indexOf("}", scss.indexOf(".agent-view-popover-foot")));
        expect(foot, "the footer's two links wrap rather than push the panel sideways").toContain("flex-wrap: wrap");
        expect(foot, "and a link's own label may break").toContain("white-space: normal");
    });
});

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
        const custom = radio("Custom", "Preset");
        expect(custom.getAttribute("aria-disabled")).toBe("true");
        fireEvent.click(custom);
        expect(c.calls.setPreset).toHaveBeenCalledTimes(1);
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
        const same = controls({ view: VIEW_PRESETS.full, source: "default" });
        mount(same);
        const nothing = screen.getByText("Make this the default") as HTMLButtonElement;
        expect(nothing.getAttribute("aria-disabled")).toBe("true");
        fireEvent.click(nothing);
        expect(same.calls.makeDefault).not.toHaveBeenCalled();
    });

    it("gives every button and both links a tooltip that is also its accessible description", () => {
        const controlsOf = (source: "pane" | "default") => controls({ view: source === "pane" ? VIEW_PRESETS.replies : VIEW_PRESETS.full, source });
        for (const source of ["pane", "default"] as const) {
            mount(controlsOf(source));
            const all = Array.from(document.body.querySelectorAll<HTMLElement>("button"));
            // 4 presets + 4+4+4+2+2 switch values, "Make this the default", and "Follow default" for a pane of its own.
            expect(all.length).toBe(4 + 16 + 1 + (source === "pane" ? 1 : 0));
            for (const el of all) {
                const text = el.getAttribute("data-hover-tip");
                expect(text, `${el.textContent} has a tooltip`).toBeTruthy();
                expect(el.hasAttribute("data-hover-tip-instant"), `${el.textContent}'s tooltip is instant`).toBe(true);
                expect(el.getAttribute("aria-description")).toBe(text);
                expect(text!.trim().toLowerCase(), "it says the effect, not the label").not.toBe(el.textContent!.trim().toLowerCase());
            }
            cleanup();
        }
    });

    it("tells why Make this the default does nothing when there is nothing to change", () => {
        mount(controls({ view: VIEW_PRESETS.full, source: "default" }));
        const nothing = screen.getByText("Make this the default");
        expect(nothing.getAttribute("data-hover-tip")).toBe(MAKE_DEFAULT_DONE_TIP);
        cleanup();
        mount(controls({ view: VIEW_PRESETS.replies, source: "pane" }));
        expect(screen.getByText("Make this the default").getAttribute("data-hover-tip")).toBe(MAKE_DEFAULT_TIP);
    });

    it("closes on Esc", () => {
        const c = controls({ view: VIEW_PRESETS.full, source: "default" });
        mount(c);
        fireEvent.keyDown(document, { key: "Escape" });
        expect(c.popoverOpen()).toBe(false);
    });
});
