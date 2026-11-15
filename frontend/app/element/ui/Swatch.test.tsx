// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The kit's colour swatch and its grid: radio semantics, a ring on the chosen one, keys that move and a preview that follows. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Swatch, SwatchGrid, type SwatchOption } from "./Swatch";

afterEach(cleanup);

const options: SwatchOption<string>[] = Array.from({ length: 8 }, (_, i) => ({
    value: `c${i}`,
    label: `Colour ${i}`,
    color: `rgb(${i * 10}, 20, 30)`,
    ink: `rgb(240, ${i}, 0)`,
    detail: `text ${i}.0:1`,
}));

const radios = (c: HTMLElement) => [...c.querySelectorAll<HTMLButtonElement>("[role=radio]")];

describe("Swatch", () => {
    it("is a radio named by its label, with its colour and ink as custom properties, never as a stylesheet colour", () => {
        const { container } = render(() => (
            <Swatch color="#123456" ink="#fedcba" label="Teal" detail="text 9.4:1" selected={false} onSelect={() => {}} />
        ));
        const el = radios(container)[0];
        expect(el.getAttribute("aria-label")).toBe("Teal");
        expect(el.getAttribute("aria-description")).toBe("text 9.4:1");
        expect(el.getAttribute("aria-checked")).toBe("false");
        expect(el.style.getPropertyValue("--swatch-surface")).toBe("#123456");
        expect(el.style.getPropertyValue("--swatch-ink")).toBe("#fedcba");
        expect(el.textContent).toBe("Aa");
    });

    it("says it is checked when selected, and calls onSelect when pressed", () => {
        const onSelect = vi.fn();
        const { container } = render(() => <Swatch color="#111" label="A" selected onSelect={onSelect} />);
        const el = radios(container)[0];
        expect(el.getAttribute("aria-checked")).toBe("true");
        fireEvent.click(el);
        expect(onSelect).toHaveBeenCalledTimes(1);
    });

    it("draws no ink property when it has none, so the theme's text colour applies", () => {
        const { container } = render(() => <Swatch color="#111" label="A" selected={false} onSelect={() => {}} />);
        expect(radios(container)[0].style.getPropertyValue("--swatch-ink")).toBe("");
    });
});

describe("SwatchGrid", () => {
    const grid = (props: Partial<Parameters<typeof SwatchGrid<string>>[0]> = {}) =>
        render(() => <SwatchGrid ariaLabel="Backdrop" options={options} value="c1" onChange={() => {}} {...props} />);

    it("is a radiogroup with a radio per option, the chosen one checked, and the columns it was given", () => {
        const { container } = grid({ columns: 4 });
        const group = container.querySelector("[role=radiogroup]") as HTMLElement;
        expect(group.getAttribute("aria-label")).toBe("Backdrop");
        expect(group.style.getPropertyValue("--swatch-columns")).toBe("4");
        expect(radios(container).map((r) => r.getAttribute("aria-checked"))).toEqual(
            options.map((o) => (o.value === "c1" ? "true" : "false"))
        );
    });

    it("is one Tab stop: the chosen swatch, or the first when none is chosen", () => {
        const withChoice = grid();
        expect(radios(withChoice.container).map((r) => r.tabIndex)).toEqual([-1, 0, -1, -1, -1, -1, -1, -1]);
        cleanup();
        const none = grid({ value: "none" });
        expect(radios(none.container).map((r) => r.tabIndex)).toEqual([0, -1, -1, -1, -1, -1, -1, -1]);
    });

    it("never makes a disabled swatch the Tab stop: the first enabled one is, when the first or the chosen one is disabled", () => {
        const stops = (opts: SwatchOption<string>[], value: string) => radios(grid({ options: opts, value }).container).map((r) => r.tabIndex);
        const off = (...at: number[]) => options.map((o, i) => (at.includes(i) ? { ...o, disabled: true } : o));
        expect(stops(off(0), "none")).toEqual([-1, 0, -1, -1, -1, -1, -1, -1]);
        cleanup();
        expect(stops(off(0, 1), "none")).toEqual([-1, -1, 0, -1, -1, -1, -1, -1]);
        cleanup();
        expect(stops(off(1), "c1")).toEqual([0, -1, -1, -1, -1, -1, -1, -1]);
        cleanup();
        expect(stops(off(3), "c3")).toEqual([0, -1, -1, -1, -1, -1, -1, -1]);
    });

    it("keeps a Tab stop when the focused swatch becomes disabled", () => {
        const [opts, setOpts] = createSignal(options);
        const { container } = render(() => <SwatchGrid ariaLabel="Backdrop" options={opts()} value="none" onChange={() => {}} />);
        const r = radios(container);
        r[0].focus();
        fireEvent.keyDown(r[0], { key: "ArrowRight" });
        expect(r.map((x) => x.tabIndex)).toEqual([-1, 0, -1, -1, -1, -1, -1, -1]);
        // New option objects rebuild the buttons, so they are read again.
        setOpts(options.map((o, i) => (i === 1 ? { ...o, disabled: true } : o)));
        expect(radios(container).map((x) => x.tabIndex)).toEqual([0, -1, -1, -1, -1, -1, -1, -1]);
    });

    it("chooses the one pressed", () => {
        const onChange = vi.fn();
        const { container } = grid({ onChange });
        fireEvent.click(radios(container)[5]);
        expect(onChange).toHaveBeenCalledWith("c5");
    });

    it("moves focus with the arrow keys without choosing: Left and Right by one, Up and Down by a row, stopping at the edges", () => {
        const onChange = vi.fn();
        const { container } = grid({ onChange, columns: 4, value: "none" });
        const r = radios(container);
        r[0].focus();
        fireEvent.keyDown(r[0], { key: "ArrowRight" });
        expect(document.activeElement).toBe(r[1]);
        fireEvent.keyDown(r[1], { key: "ArrowDown" });
        expect(document.activeElement).toBe(r[5]);
        fireEvent.keyDown(r[5], { key: "ArrowDown" });
        expect(document.activeElement).toBe(r[5]);
        fireEvent.keyDown(r[5], { key: "ArrowUp" });
        expect(document.activeElement).toBe(r[1]);
        fireEvent.keyDown(r[1], { key: "ArrowUp" });
        expect(document.activeElement).toBe(r[1]);
        fireEvent.keyDown(r[1], { key: "End" });
        expect(document.activeElement).toBe(r[7]);
        fireEvent.keyDown(r[7], { key: "Home" });
        expect(document.activeElement).toBe(r[0]);
        expect(onChange).not.toHaveBeenCalled();
    });

    it("moves the Tab stop with focus", () => {
        const { container } = grid({ columns: 4, value: "none" });
        const r = radios(container);
        r[0].focus();
        fireEvent.keyDown(r[0], { key: "ArrowRight" });
        expect(r.map((x) => x.tabIndex)).toEqual([-1, 0, -1, -1, -1, -1, -1, -1]);
    });

    it("skips a disabled swatch", () => {
        const withDisabled = options.map((o, i) => (i === 1 ? { ...o, disabled: true } : o));
        const { container } = grid({ options: withDisabled, value: "none" });
        const r = radios(container);
        r[0].focus();
        fireEvent.keyDown(r[0], { key: "ArrowRight" });
        expect(document.activeElement).toBe(r[2]);
    });

    it("reports the option being looked at, on hover and on focus, and none when it leaves, blurs or Escape is pressed", () => {
        const seen: Array<string | undefined> = [];
        const { container } = grid({ onPreview: (v) => seen.push(v) });
        const r = radios(container);
        fireEvent.mouseEnter(r[3]);
        fireEvent.mouseLeave(r[3]);
        fireEvent.focus(r[4]);
        fireEvent.keyDown(r[4], { key: "Escape" });
        fireEvent.blur(r[4]);
        expect(seen).toEqual(["c3", undefined, "c4", undefined, undefined]);
    });

    it("shows a chosen value the owner changes", () => {
        const [value, setValue] = createSignal<string | undefined>("c0");
        const { container } = render(() => <SwatchGrid ariaLabel="Backdrop" options={options} value={value()} onChange={setValue} />);
        fireEvent.click(radios(container)[6]);
        expect(radios(container).map((x) => x.getAttribute("aria-checked")).indexOf("true")).toBe(6);
    });
});
