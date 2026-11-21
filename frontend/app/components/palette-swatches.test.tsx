// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PALETTE } from "@/app/theme/palette";
import { PaletteSwatches } from "./palette-swatches";

afterEach(() => cleanup());

const swatches = (root: HTMLElement) => Array.from(root.querySelectorAll<HTMLButtonElement>("button.palette-swatch"));

describe("PaletteSwatches", () => {
    it("offers every palette slot by name, none of them as a bare colour", () => {
        const { container } = render(() => <PaletteSwatches label="Tab color" value={null} onSelect={() => {}} />);
        const names = swatches(container).map((b) => b.getAttribute("aria-label"));
        expect(names).toEqual(PALETTE.map((s) => s.name));
        expect(names).toHaveLength(14);
        expect(names.slice(12)).toEqual(["Black", "White"]);
        expect(container.querySelector('[role="group"]')?.getAttribute("aria-label")).toBe("Tab color");
    });

    const pressedNames = (value: string | number) => {
        const { container, unmount } = render(() => <PaletteSwatches label="c" value={value} onSelect={() => {}} />);
        const names = swatches(container)
            .filter((b) => b.getAttribute("aria-pressed") === "true")
            .map((b) => b.getAttribute("aria-label"));
        unmount();
        return names;
    };

    it("marks the slot a stored hex, an id, a retired id, one of the 16's hexes or an old Tailwind hex stands for", () => {
        for (const value of [PALETTE[9].hex, "blue", "azure", "#53a3f2", "#3b82f6"]) expect(pressedNames(value), String(value)).toEqual(["Blue"]);
        expect(pressedNames("indigo")).toEqual(["Violet"]);
        expect(pressedNames("#000000")).toEqual(["Black"]);
        expect(pressedNames("white")).toEqual(["White"]);
    });

    it("paints a hue in its identity tone, Black and White as themselves, each neutral with a check mark of the other's colour", () => {
        const { container } = render(() => <PaletteSwatches label="c" value={null} onSelect={() => {}} />);
        const all = swatches(container);
        const paint = (b: HTMLElement) => b.style.backgroundColor;
        const ink = (b: HTMLElement) => b.style.getPropertyValue("--palette-swatch-ink");
        expect(paint(all[12])).toBe("rgb(0, 0, 0)");
        expect(paint(all[13])).toBe("rgb(255, 255, 255)");
        expect([ink(all[12]), ink(all[13])]).toEqual(["#ffffff", "#000000"]);
        for (const hue of all.slice(0, 12)) {
            expect(paint(hue)).not.toBe("");
            expect(ink(hue)).toBe("");
        }
        expect(new Set(all.map(paint)).size).toBe(14);
    });

    it("draws a sample of what is picked (its fill and its bar) when it is given one, and a plain swatch otherwise", () => {
        const sample = (slot: { id: string }) => ({ fill: `#00000${PALETTE.findIndex((s) => s.id === slot.id) % 10}`, bar: "#ffffff" });
        const { container } = render(() => <PaletteSwatches label="c" value={null} onSelect={() => {}} sample={sample} />);
        const all = swatches(container);
        expect(all).toHaveLength(14);
        all.forEach((b, i) => {
            expect(b.classList.contains("palette-swatch--sample")).toBe(true);
            expect(b.style.getPropertyValue("--swatch-fill")).toBe(`#00000${i % 10}`);
            expect(b.style.getPropertyValue("--swatch-bar")).toBe("#ffffff");
            expect(b.style.backgroundColor).toBe("");
        });
        const plain = render(() => <PaletteSwatches label="c" value={null} onSelect={() => {}} />);
        for (const b of swatches(plain.container)) expect(b.classList.contains("palette-swatch--sample")).toBe(false);
    });

    it("leaves the columns to its styles (two rows of 7, with the none swatch in a column of its own) unless asked", () => {
        const grid = (c: HTMLElement) => c.querySelector(".palette-swatches") as HTMLElement;
        const plain = render(() => <PaletteSwatches label="c" value={null} onSelect={() => {}} />);
        expect(grid(plain.container).style.getPropertyValue("--palette-cols")).toBe("");
        expect(grid(plain.container).classList.contains("palette-swatches--with-none")).toBe(false);
        plain.unmount();
        const none = render(() => <PaletteSwatches label="c" value={null} onSelect={() => {}} noneLabel="None" />);
        expect(grid(none.container).style.getPropertyValue("--palette-cols")).toBe("");
        expect(grid(none.container).classList.contains("palette-swatches--with-none")).toBe(true);
        none.unmount();
        const fixed = render(() => <PaletteSwatches label="c" value={null} onSelect={() => {}} columns={5} />);
        expect(grid(fixed.container).style.getPropertyValue("--palette-cols")).toBe("5");
    });
    it("reports the slot picked", () => {
        const onSelect = vi.fn();
        const { container } = render(() => <PaletteSwatches label="c" value={null} onSelect={onSelect} />);
        fireEvent.click(swatches(container)[3]);
        expect(onSelect).toHaveBeenCalledWith(PALETTE[3]);
    });

    it("has a leading none swatch only when asked, pressed when the value is null, reporting null", () => {
        const onSelect = vi.fn();
        const none = render(() => <PaletteSwatches label="c" value={null} onSelect={onSelect} noneLabel="No color" />);
        const first = swatches(none.container)[0];
        expect(first.getAttribute("aria-label")).toBe("No color");
        expect(first.getAttribute("aria-pressed")).toBe("true");
        fireEvent.click(first);
        expect(onSelect).toHaveBeenCalledWith(null);
        none.unmount();
        const plain = render(() => <PaletteSwatches label="c" value="blue" onSelect={() => {}} />);
        expect(swatches(plain.container)).toHaveLength(14);
    });
});
