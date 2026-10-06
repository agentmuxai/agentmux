// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * element/ui/ (SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md §5). The
 * look lives in SCSS; what these pin is the contract the SCSS keys off and
 * that assistive tech reads: tone classes, ARIA state, keyboard movement and
 * the label/control wiring.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Button, IconButton } from "./Button";
import { Field } from "./Field";
import { NumberInput, Select, Switch, TextInput } from "./inputs";
import { SegmentedControl } from "./SegmentedControl";
import { rovingTarget } from "./shared";
import { TabbedPane, tabbedPaneLayout, Tabs } from "./Tabs";

afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("rovingTarget", () => {
    it("moves along its own axis and wraps", () => {
        expect(rovingTarget("ArrowRight", 0, 3, "horizontal")).toBe(1);
        expect(rovingTarget("ArrowRight", 2, 3, "horizontal")).toBe(0);
        expect(rovingTarget("ArrowLeft", 0, 3, "horizontal")).toBe(2);
        expect(rovingTarget("ArrowDown", 0, 3, "horizontal")).toBeNull();
        expect(rovingTarget("ArrowDown", 0, 3, "vertical")).toBe(1);
    });

    it("jumps to the ends and skips disabled options", () => {
        const disabled = (i: number) => i === 0 || i === 2;
        expect(rovingTarget("Home", 3, 4, "horizontal", disabled)).toBe(1);
        expect(rovingTarget("End", 0, 4, "horizontal", disabled)).toBe(3);
        expect(rovingTarget("ArrowRight", 1, 4, "horizontal", disabled)).toBe(3);
    });

    it("ignores other keys", () => {
        expect(rovingTarget("a", 0, 3, "horizontal")).toBeNull();
    });
});

describe("Button", () => {
    it("is a neutral, non-submitting button by default", () => {
        render(() => <Button>Cancel</Button>);
        const b = screen.getByRole("button", { name: "Cancel" });
        expect(b.getAttribute("type")).toBe("button");
        expect(b.classList.contains("ui-button")).toBe(true);
        expect(b.classList.contains("ui-tone-neutral")).toBe(true);
        expect(b.hasAttribute("aria-pressed")).toBe(false);
    });

    it("takes its tone and density as classes", () => {
        render(() => (
            <Button tone="danger" density="compact">
                Close tab
            </Button>
        ));
        const b = screen.getByRole("button", { name: "Close tab" });
        expect(b.classList.contains("ui-tone-danger")).toBe(true);
        expect(b.classList.contains("ui-density-compact")).toBe(true);
    });

    it("is disabled and marked busy while busy", () => {
        const onClick = vi.fn();
        render(() => (
            <Button tone="accent" busy onClick={onClick}>
                Create
            </Button>
        ));
        const b = screen.getByRole("button", { name: "Create" });
        expect(b.getAttribute("aria-busy")).toBe("true");
        expect((b as HTMLButtonElement).disabled).toBe(true);
        expect(b.querySelector(".fa-spin")).not.toBeNull();
    });

    it("exposes a toggle's state as aria-pressed", () => {
        const [on, setOn] = createSignal(false);
        render(() => (
            <Button pressed={on()} onClick={() => setOn(!on())}>
                Shell
            </Button>
        ));
        const b = screen.getByRole("button", { name: "Shell" });
        expect(b.getAttribute("aria-pressed")).toBe("false");
        fireEvent.click(b);
        expect(b.getAttribute("aria-pressed")).toBe("true");
    });
});

describe("IconButton", () => {
    it("is named by its label and defaults to the quiet tone", () => {
        render(() => <IconButton icon="xmark" label="Close" />);
        const b = screen.getByRole("button", { name: "Close" });
        expect(b.classList.contains("ui-icon-button")).toBe(true);
        expect(b.classList.contains("ui-tone-quiet")).toBe(true);
    });
});

describe("SegmentedControl", () => {
    const options = [
        { value: "preview", label: "Preview" },
        { value: "source", label: "Source" },
        { value: "split", label: "Split", disabled: true },
    ] as const;

    it("is a radio group with the selected option checked and the only tab stop", () => {
        render(() => (
            <SegmentedControl ariaLabel="Mode" options={[...options]} value="source" onChange={() => {}} />
        ));
        expect(screen.getByRole("radiogroup", { name: "Mode" })).toBeTruthy();
        const radios = screen.getAllByRole("radio");
        expect(radios.map((r) => r.getAttribute("aria-checked"))).toEqual(["false", "true", "false"]);
        expect(radios.map((r) => r.tabIndex)).toEqual([-1, 0, -1]);
    });

    it("selects with arrow keys, skipping disabled options", () => {
        const [value, setValue] = createSignal<string>("source");
        render(() => <SegmentedControl ariaLabel="Mode" options={[...options]} value={value()} onChange={setValue} />);
        fireEvent.keyDown(screen.getByRole("radio", { name: "Source" }), { key: "ArrowRight" });
        expect(value()).toBe("preview");
        fireEvent.keyDown(screen.getByRole("radio", { name: "Preview" }), { key: "ArrowDown" });
        expect(value()).toBe("source");
    });
});

describe("Tabs", () => {
    const items = [
        { id: "general", label: "General", icon: "gear" },
        { id: "appearance", label: "Appearance", icon: "palette" },
        { id: "terminal", label: "Terminal", icon: "terminal" },
    ];

    it("renders a labelled tablist tied to one panel", () => {
        render(() => <Tabs items={items} value="appearance" onChange={() => {}} idPrefix="s" ariaLabel="Settings" />);
        const list = screen.getByRole("tablist", { name: "Settings" });
        expect(list.getAttribute("aria-orientation")).toBe("horizontal");
        const tabs = screen.getAllByRole("tab");
        expect(tabs.map((t) => t.getAttribute("aria-selected"))).toEqual(["false", "true", "false"]);
        expect(tabs.map((t) => t.tabIndex)).toEqual([-1, 0, -1]);
        expect(tabs[1].id).toBe("s-tab-appearance");
        expect(tabs[1].getAttribute("aria-controls")).toBe("s-panel");
    });

    it("moves with the arrow keys of its orientation, and Home/End", () => {
        const [value, setValue] = createSignal("general");
        render(() => (
            <Tabs items={items} value={value()} onChange={setValue} orientation="vertical" idPrefix="s" ariaLabel="Settings" />
        ));
        const first = screen.getByRole("tab", { name: "General" });
        fireEvent.keyDown(first, { key: "ArrowRight" });
        expect(value()).toBe("general");
        fireEvent.keyDown(first, { key: "ArrowDown" });
        expect(value()).toBe("appearance");
        expect(document.activeElement).toBe(screen.getByRole("tab", { name: "Appearance" }));
        fireEvent.keyDown(document.activeElement!, { key: "End" });
        expect(value()).toBe("terminal");
        fireEvent.keyDown(document.activeElement!, { key: "Home" });
        expect(value()).toBe("general");
    });

    it("keeps each tab's name when only icons show", () => {
        render(() => (
            <Tabs items={items} value="general" onChange={() => {}} iconOnly idPrefix="s" ariaLabel="Settings" />
        ));
        expect(screen.getByRole("tablist").classList.contains("ui-tabs--icon-only")).toBe(true);
        expect(screen.getByRole("tab", { name: "Terminal" }).getAttribute("aria-label")).toBe("Terminal");
    });
});

describe("TabbedPane", () => {
    it("picks its layout from its width", () => {
        expect(tabbedPaneLayout(900, 768, 480)).toBe("rail");
        expect(tabbedPaneLayout(767, 768, 480)).toBe("rail-icons");
        expect(tabbedPaneLayout(479, 768, 480)).toBe("top");
    });

    it("labels its panel with the selected tab", () => {
        render(() => (
            <TabbedPane
                items={[
                    { id: "a", label: "A" },
                    { id: "b", label: "B" },
                ]}
                value="b"
                onChange={() => {}}
                idPrefix="p"
                ariaLabel="Sections"
            >
                <p>content</p>
            </TabbedPane>
        ));
        const panel = screen.getByRole("tabpanel");
        expect(panel.id).toBe("p-panel");
        expect(panel.getAttribute("aria-labelledby")).toBe("p-tab-b");
        expect(screen.getByRole("tablist").getAttribute("aria-orientation")).toBe("vertical");
    });
});

describe("Field", () => {
    it("labels its control and describes it with the description and error", () => {
        render(() => (
            <Field label="Scrollback" description="Lines kept per terminal" error="Too large">
                <TextInput />
            </Field>
        ));
        const input = screen.getByLabelText("Scrollback");
        expect(input.tagName).toBe("INPUT");
        expect(input.getAttribute("aria-invalid")).toBe("true");
        const describedBy = input.getAttribute("aria-describedby")!.split(" ");
        const texts = describedBy.map((id) => document.getElementById(id)!.textContent);
        expect(texts).toEqual(["Lines kept per terminal", "Too large"]);
    });

    it("names a switch and a segmented control inside it", () => {
        render(() => (
            <>
                <Field label="Word wrap">
                    <Switch checked={false} onChange={() => {}} />
                </Field>
                <Field label="Scope">
                    <SegmentedControl
                        options={[
                            { value: "once", label: "Once" },
                            { value: "always", label: "Always" },
                        ]}
                        value="once"
                        onChange={() => {}}
                    />
                </Field>
            </>
        ));
        expect(screen.getByRole("switch", { name: "Word wrap" })).toBeTruthy();
        expect(screen.getByRole("radiogroup", { name: "Scope" })).toBeTruthy();
    });
});

describe("Switch", () => {
    it("toggles and reports its state as aria-checked", () => {
        const [on, setOn] = createSignal(false);
        render(() => <Switch ariaLabel="Enabled" checked={on()} onChange={setOn} />);
        const s = screen.getByRole("switch", { name: "Enabled" });
        expect(s.getAttribute("aria-checked")).toBe("false");
        fireEvent.click(s);
        expect(on()).toBe(true);
        expect(s.getAttribute("aria-checked")).toBe("true");
    });
});

describe("Select", () => {
    it("shows the current value and reports a change", () => {
        const onChange = vi.fn();
        render(() => (
            <Select
                ariaLabel="Theme"
                options={[
                    { value: "dark", label: "Dark" },
                    { value: "light", label: "Light" },
                ]}
                value="light"
                onChange={onChange}
            />
        ));
        const select = screen.getByRole("combobox", { name: "Theme" }) as HTMLSelectElement;
        expect(select.value).toBe("light");
        fireEvent.change(select, { target: { value: "dark" } });
        expect(onChange).toHaveBeenCalledWith("dark");
    });
});

describe("NumberInput", () => {
    it("commits after a pause in typing, only in range", () => {
        vi.useFakeTimers();
        const onChange = vi.fn();
        render(() => <NumberInput value={10} min={1} max={100} step={1} parse="int" onChange={onChange} />);
        const input = screen.getByRole("spinbutton") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "50" } });
        vi.advanceTimersByTime(399);
        expect(onChange).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(onChange).toHaveBeenCalledWith(50);
        fireEvent.input(input, { target: { value: "500" } });
        vi.advanceTimersByTime(400);
        expect(onChange).toHaveBeenCalledTimes(1);
    });

    it("flushes a pending commit on blur, once", () => {
        vi.useFakeTimers();
        const onChange = vi.fn();
        render(() => <NumberInput value={10} min={1} step={1} onChange={onChange} />);
        const input = screen.getByRole("spinbutton") as HTMLInputElement;
        fireEvent.input(input, { target: { value: "20" } });
        fireEvent.blur(input);
        expect(onChange).toHaveBeenCalledWith(20);
        vi.advanceTimersByTime(1000);
        fireEvent.blur(input);
        expect(onChange).toHaveBeenCalledTimes(1);
    });
});
