// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The backdrop picker: what it offers, what choosing writes, and the preview that comes and goes. */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ set: vi.fn(async (..._a: unknown[]) => {}), light: false, theme: "default" }));
vi.mock("@/app/store/block-meta", () => ({ setBlockMeta: (...a: unknown[]) => h.set(...a) }));
vi.mock("../block/pane-identity", () => ({ isLightThemeActive: () => h.light }));
vi.mock("../store/window-appearance", () => ({ currentWindowTheme: () => h.theme }));

import { resetThemeAnsi } from "./ansi";
import { BackdropPicker } from "./BackdropPicker";
import { BUILTINS, deriveBuiltin } from "./builtins";
import { deriveInk } from "./ink";
import { getPaneScope, registerPaneScope, releasePaneScope, scopeVars } from "./scope";

const realMatchMedia = Object.getOwnPropertyDescriptor(window, "matchMedia");
afterEach(() => {
    h.theme = "default";
    if (realMatchMedia) Object.defineProperty(window, "matchMedia", realMatchMedia);
    else delete (window as { matchMedia?: unknown }).matchMedia;
    cleanup();
    h.set.mockClear();
    h.light = false;
    releasePaneScope("pk");
});

type Meta = { backdrop?: unknown; bg?: unknown };
function mount(meta: Meta = {}) {
    const [m, setM] = createSignal<Meta>(meta);
    const onClose = vi.fn();
    // The pane's frame is what the picker's Esc is scoped to; a composer stands in for what a swatch click hands focus to.
    const view = render(() => (
        <div class="block-frame-default">
            <input data-testid="composer" />
            <BackdropPicker blockId="pk" meta={m} onClose={onClose} />
        </div>
    ));
    return { ...view, onClose, setMeta: setM };
}
const radios = (c: HTMLElement) => [...c.querySelectorAll<HTMLButtonElement>("[role=radiogroup][aria-label='Backdrop colours'] [role=radio]")];
const button = (c: HTMLElement, name: string) => [...c.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent?.trim() === name)!;
const written = () => h.set.mock.calls.map((c) => (c[1] as { backdrop: unknown }).backdrop);
const teal = { kind: "builtin", id: "teal", polarity: "follow", version: 1 };

describe("BackdropPicker", () => {
    it("is a panel named Backdrop with the 16 built-ins as swatches, each named and carrying its contrast readout", () => {
        const { container } = mount();
        expect(container.querySelector("[role=dialog]")?.getAttribute("aria-label")).toBe("Backdrop");
        const r = radios(container);
        expect(r.map((x) => x.getAttribute("aria-label"))).toEqual(BUILTINS.map((b) => b.name));
        expect(r[0].getAttribute("aria-description")).toMatch(/^text \d+\.\d:1, muted \d+\.\d:1$/);
    });

    it("draws each swatch as the surface and ink the app theme gives it, and the other side's when the app theme is light", () => {
        const dark = mount();
        const d = deriveBuiltin(BUILTINS[0].id, "dark")!;
        expect(radios(dark.container)[0].style.getPropertyValue("--swatch-surface")).toBe(d.surfaces.surface);
        expect(radios(dark.container)[0].style.getPropertyValue("--swatch-ink")).toBe(d.ink.text);
        cleanup();
        h.light = true;
        const light = mount();
        expect(radios(light.container)[0].style.getPropertyValue("--swatch-surface")).toBe(deriveBuiltin(BUILTINS[0].id, "light")!.surfaces.surface);
    });

    it("marks the pane's built-in as checked", () => {
        const { container } = mount({ backdrop: teal });
        const checked = radios(container).filter((x) => x.getAttribute("aria-checked") === "true");
        expect(checked.map((x) => x.getAttribute("aria-label"))).toEqual(["Teal"]);
    });

    it("writes a built-in record when one is chosen, with the polarity as set", () => {
        const { container } = mount();
        fireEvent.click(radios(container)[2]);
        expect(written()).toEqual([{ kind: "builtin", id: BUILTINS[2].id, polarity: "follow", version: 1 }]);
        expect(h.set.mock.calls[0][0]).toBe("pk");
    });

    it("rewrites the record with the new polarity when the polarity changes, and only changes the setting when there is no record", () => {
        const withRecord = mount({ backdrop: teal });
        fireEvent.click(button(withRecord.container, "Dark"));
        expect(written()).toEqual([{ ...teal, polarity: "dark" }]);
        cleanup();
        h.set.mockClear();
        const none = mount();
        fireEvent.click(button(none.container, "Light"));
        expect(written()).toEqual([]);
        fireEvent.click(radios(none.container)[0]);
        expect(written()).toEqual([{ kind: "builtin", id: BUILTINS[0].id, polarity: "light", version: 1 }]);
    });

    it("under a fixed polarity draws the swatches for that side whatever the app theme", () => {
        const { container } = mount({ backdrop: { ...teal, polarity: "light" } });
        expect(radios(container)[0].style.getPropertyValue("--swatch-surface")).toBe(deriveBuiltin(BUILTINS[0].id, "light")!.surfaces.surface);
    });

    it("writes a typed colour on Enter, lower-cased and with #rrggbb, and says so when it isn't one", () => {
        const { container } = mount();
        const input = container.querySelector<HTMLInputElement>("input[placeholder='#rrggbb']")!;
        fireEvent.input(input, { target: { value: "ABC" } });
        fireEvent.keyDown(input, { key: "Enter" });
        expect(written()).toEqual([{ kind: "color", hex: "#aabbcc", polarity: "follow", version: 1 }]);
        h.set.mockClear();
        fireEvent.input(input, { target: { value: "teal" } });
        fireEvent.keyDown(input, { key: "Enter" });
        expect(written()).toEqual([]);
        expect(container.textContent).toContain("Use #rrggbb or #rgb.");
    });

    it("writes the colour picked in the native colour input", () => {
        const { container } = mount();
        const color = container.querySelector<HTMLInputElement>("input[type=color]")!;
        fireEvent.change(color, { target: { value: "#336699" } });
        expect(written()).toEqual([{ kind: "color", hex: "#336699", polarity: "follow", version: 1 }]);
    });

    it("tells the user when a custom colour was adjusted to read, and not otherwise", () => {
        const mid = "#7a7a7a";
        expect(deriveInk(mid).nudged).toBe(true);
        const nudged = mount({ backdrop: { kind: "color", hex: mid, polarity: "follow", version: 1 } });
        expect(nudged.container.textContent).toContain("Adjusted a little so the text reads.");
        cleanup();
        expect(deriveInk("#101820").nudged).toBe(false);
        const fine = mount({ backdrop: { kind: "color", hex: "#101820", polarity: "follow", version: 1 } });
        expect(fine.container.textContent).not.toContain("Adjusted a little");
    });

    it("shows the contrast of what the pane has, and nothing when it has none", () => {
        expect(mount().container.querySelector(".backdrop-picker-readout")).toBeNull();
        cleanup();
        const { container } = mount({ backdrop: teal });
        expect(container.querySelector(".backdrop-picker-readout")?.textContent).toMatch(/^text \d+\.\d:1, muted \d+\.\d:1$/);
        expect(container.querySelector(".backdrop-picker-readout")?.classList.contains("is-missed")).toBe(false);
    });

    it("resets by clearing the record, and has nothing to reset on a pane without one", () => {
        const none = mount();
        expect(button(none.container, "Reset").disabled).toBe(true);
        cleanup();
        const { container } = mount({ backdrop: teal });
        fireEvent.click(button(container, "Reset"));
        expect(written()).toEqual([null]);
    });

    it("follows the record when it changes elsewhere", () => {
        const t = mount({ backdrop: teal });
        t.setMeta({ backdrop: { ...teal, id: "crimson", polarity: "dark" } });
        expect(radios(t.container).filter((x) => x.getAttribute("aria-checked") === "true").map((x) => x.getAttribute("aria-label"))).toEqual(["Crimson"]);
        expect(button(t.container, "Dark").getAttribute("aria-checked")).toBe("true");
    });

    it("previews a swatch on the pane while it is looked at, and puts the pane's own back after", () => {
        const { container } = mount();
        const r = radios(container);
        fireEvent.focus(r[1]);
        expect(getPaneScope("pk")()?.["--surface"]).toBe(deriveBuiltin(BUILTINS[1].id, "dark")!.surfaces.surface);
        fireEvent.blur(r[1]);
        expect(getPaneScope("pk")()).toBeUndefined();
        // With a backdrop already set, leaving puts that one back.
        cleanup();
        const own = mount({ backdrop: teal });
        registerPaneScope("pk", scopeVars(deriveBuiltin("teal", "dark")!));
        fireEvent.mouseEnter(radios(own.container)[5]);
        expect(getPaneScope("pk")()?.["--surface"]).not.toBe(deriveBuiltin("teal", "dark")!.surfaces.surface);
        fireEvent.mouseLeave(radios(own.container)[5]);
        expect(getPaneScope("pk")()?.["--surface"]).toBe(deriveBuiltin("teal", "dark")!.surfaces.surface);
    });

    it("keeps its clicks, presses and focus to itself, so the pane's own handlers don't take focus from a swatch", () => {
        const seen: string[] = [];
        const { container } = render(() => (
            <div class="block-frame-default" onClick={() => seen.push("click")} onMouseDown={() => seen.push("down")} onFocusIn={() => seen.push("focus")}>
                <BackdropPicker blockId="pk" meta={() => ({})} onClose={() => {}} />
            </div>
        ));
        const swatch = radios(container)[0];
        fireEvent.mouseDown(swatch);
        fireEvent.click(swatch);
        fireEvent.focusIn(swatch);
        expect(seen).toEqual([]);
    });

    it("registers no scope while backdrops are suppressed, not for a preview and not when it closes: High Contrast, and forced colours", () => {
        for (const how of ["high-contrast", "forced-colors"] as const) {
            if (how === "high-contrast") h.theme = "high-contrast";
            else Object.defineProperty(window, "matchMedia", { configurable: true, writable: true, value: (q: string) => ({ matches: q === "(forced-colors: active)", addEventListener() {}, removeEventListener() {} }) });
            const t = mount({ backdrop: teal });
            expect(t.container.textContent, how).toContain("A backdrop isn't shown while High Contrast or forced colours are on.");
            fireEvent.focus(radios(t.container)[1]);
            fireEvent.mouseEnter(radios(t.container)[2]);
            expect(getPaneScope("pk")(), `${how} preview`).toBeUndefined();
            fireEvent.keyDown(radios(t.container)[1], { key: "Escape" });
            expect(t.onClose, how).toHaveBeenCalledTimes(1);
            expect(getPaneScope("pk")(), `${how} closed`).toBeUndefined();
            cleanup();
            h.theme = "default";
            delete (window as { matchMedia?: unknown }).matchMedia;
        }
    });

    it("says nothing about suppression, and previews as usual, when backdrops are showing", () => {
        const t = mount({ backdrop: teal });
        expect(t.container.textContent).not.toContain("isn't shown while High Contrast");
        fireEvent.focus(radios(t.container)[1]);
        expect(getPaneScope("pk")()?.["--surface"]).toMatch(/^#[0-9a-f]{6}$/);
    });

    it("previews and restores the pane with the theme's terminal palette moved too, as the pane's own scope has it", () => {
        const root = document.documentElement;
        resetThemeAnsi();
        root.style.setProperty("--term-green", "#4e9a06");
        try {
            const t = mount({ backdrop: teal });
            fireEvent.focus(radios(t.container)[1]);
            expect(getPaneScope("pk")()?.["--term-green"]).toMatch(/^#[0-9a-f]{6}$/);
            fireEvent.keyDown(radios(t.container)[1], { key: "Escape" });
            expect(getPaneScope("pk")()?.["--term-green"], "after it closes").toMatch(/^#[0-9a-f]{6}$/);
        } finally {
            root.style.removeProperty("--term-green");
            resetThemeAnsi();
        }
    });

    it("takes focus when it opens, so Esc works at once", () => {
        const { container } = mount();
        expect(document.activeElement).toBe(container.querySelector("[role=dialog]"));
    });

    it("gives focus back to where it was when it closes", () => {
        const opener = document.createElement("input");
        document.body.appendChild(opener);
        opener.focus();
        const t = mount();
        expect(document.activeElement).toBe(t.container.querySelector("[role=dialog]"));
        t.unmount();
        expect(document.activeElement).toBe(opener);
        opener.remove();
    });

    it("leaves focus where the user moved it, when it closes", () => {
        const opener = document.createElement("input");
        const other = document.createElement("input");
        document.body.append(opener, other);
        opener.focus();
        const t = mount();
        other.focus();
        t.unmount();
        expect(document.activeElement).toBe(other);
        opener.remove();
        other.remove();
    });

    it("closes on Escape from anywhere in its own pane, ending any preview: a swatch, the panel, or the composer a click handed focus to", () => {
        for (const from of ["swatch", "panel", "composer"] as const) {
            const t = mount();
            fireEvent.focus(radios(t.container)[1]);
            const el =
                from === "swatch" ? radios(t.container)[1] : from === "panel" ? t.container.querySelector("[role=dialog]")! : t.container.querySelector("[data-testid=composer]")!;
            fireEvent.keyDown(el, { key: "Escape" });
            expect(t.onClose, from).toHaveBeenCalledTimes(1);
            expect(getPaneScope("pk")(), from).toBeUndefined();
            cleanup();
        }
    });

    it("consumes the Escape that closes it, so the layout's own window-level Esc (un-magnify) does not also run; and leaves one it didn't use alone", () => {
        const layout = vi.fn();
        window.addEventListener("keydown", layout);
        try {
            const t = mount();
            const composer = t.container.querySelector("[data-testid=composer]")!;
            const closing = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
            composer.dispatchEvent(closing);
            expect(t.onClose).toHaveBeenCalledTimes(1);
            expect(closing.defaultPrevented).toBe(true);
            expect(layout).not.toHaveBeenCalled();
            cleanup();

            // No picker open: the layout's handler sees the Esc as before.
            const bare = document.createElement("input");
            document.body.appendChild(bare);
            bare.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
            expect(layout).toHaveBeenCalledTimes(1);

            // A picker open in another pane does not take an Esc pressed outside its own pane.
            const t2 = mount();
            bare.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
            expect(t2.onClose).not.toHaveBeenCalled();
            expect(layout).toHaveBeenCalledTimes(2);
            bare.remove();
        } finally {
            window.removeEventListener("keydown", layout);
        }
    });

    it("leaves an Escape pressed in another pane, or one another handler used, alone", () => {
        const t = mount();
        const outside = document.createElement("input");
        document.body.appendChild(outside);
        fireEvent.keyDown(outside, { key: "Escape" });
        expect(t.onClose).not.toHaveBeenCalled();
        const composer = t.container.querySelector("[data-testid=composer]")!;
        composer.addEventListener("keydown", (e) => e.preventDefault());
        fireEvent.keyDown(composer, { key: "Escape" });
        expect(t.onClose).not.toHaveBeenCalled();
        outside.remove();
    });

    it("stops listening for Escape when it closes: the keydown listener it added to the document is removed", () => {
        const added = vi.spyOn(document, "addEventListener");
        const removed = vi.spyOn(document, "removeEventListener");
        const t = mount();
        const keydowns = added.mock.calls.filter((c) => c[0] === "keydown").map((c) => c[1]);
        expect(keydowns.length).toBeGreaterThan(0);
        t.unmount();
        const gone = removed.mock.calls.filter((c) => c[0] === "keydown").map((c) => c[1]);
        for (const listener of keydowns) expect(gone).toContain(listener);
        added.mockRestore();
        removed.mockRestore();
    });

    it("says in a warning notice, with an icon, when the readout missed a target, and in plain text when it did not", () => {
        const fine = mount({ backdrop: teal });
        expect(fine.container.querySelector(".ui-notice")).toBeNull();
        cleanup();
        const hard = mount({ backdrop: { kind: "color", hex: "#336699", polarity: "follow", version: 1 } });
        const notice = hard.container.querySelector(".ui-notice--warning");
        expect(notice?.textContent).toMatch(/text \d+\.\d:1, muted \d+\.\d:1/);
        expect(notice?.querySelector("svg, i, .ui-icon, [class*=fa-]")).not.toBeNull();
    });

    it("closes on its close button, ending any preview", () => {
        const x = mount();
        fireEvent.click(x.container.querySelector("button[aria-label='Close the backdrop picker']")!);
        expect(x.onClose).toHaveBeenCalledTimes(1);
    });
});
