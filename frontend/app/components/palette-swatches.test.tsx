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
        expect(container.querySelector('[role="group"]')?.getAttribute("aria-label")).toBe("Tab color");
    });

    it("marks the slot a stored hex, an id or an old Tailwind hex stands for", () => {
        for (const value of [PALETTE[11].hex, "azure", "#3b82f6"]) {
            const { container, unmount } = render(() => <PaletteSwatches label="c" value={value} onSelect={() => {}} />);
            const pressed = swatches(container).filter((b) => b.getAttribute("aria-pressed") === "true");
            expect(pressed.map((b) => b.getAttribute("aria-label")), String(value)).toEqual(["Azure"]);
            unmount();
        }
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
        const plain = render(() => <PaletteSwatches label="c" value="azure" onSelect={() => {}} />);
        expect(swatches(plain.container)).toHaveLength(16);
    });
});
