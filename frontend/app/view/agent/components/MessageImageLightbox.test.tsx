// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The message image preview: the keys, the neighbours it loads, a file that can't load, and the labels. */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

const h = vi.hoisted(() => ({ fetch: vi.fn(), revoked: [] as string[] }));

vi.mock("@/app/element/modal", () => ({
    Modal: (p: { ariaLabel?: string; children?: unknown }) => <div data-stand="Modal" data-aria-label={p.ariaLabel}>{p.children as any}</div>,
}));
vi.mock("@/app/element/local-media", async (orig) => ({
    ...(await orig<typeof import("@/app/element/local-media")>()),
    fetchMediaBlob: (path: string) => h.fetch(path),
}));

import { MessageImageLightbox } from "./MessageImageLightbox";

const items = ["a.png", "b.png", "c.png"].map((name) => ({ path: `D:/p/${name}`, name, alt: "" }));

type Spies = { onClose: Mock<() => void>; onOpenEditor: Mock<(path: string) => void> };

function mount(index = 0, list = items): Spies & ReturnType<typeof render> {
    const spies: Spies = { onClose: vi.fn<() => void>(), onOpenEditor: vi.fn<(path: string) => void>() };
    const r = render(() => <MessageImageLightbox items={list} index={index} {...spies} />);
    return { ...spies, ...r };
}

const dialog = (c: HTMLElement) => c.querySelector<HTMLElement>(".agent-attachment-lightbox__dialog")!;
const label = (c: HTMLElement) => c.querySelector("[data-stand=Modal]")!.getAttribute("data-aria-label");
const key = (c: HTMLElement, k: string) => fireEvent.keyDown(dialog(c), { key: k });

beforeEach(() => {
    h.revoked = [];
    h.fetch.mockReset().mockImplementation(async (path: string) => new Blob([path], { type: "image/png" }));
    let n = 0;
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: () => `blob:${++n}`, revokeObjectURL: (u: string) => h.revoked.push(u) }));
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

describe("MessageImageLightbox", () => {
    it("opens on the clicked image, named by its place and its name", async () => {
        const { container } = mount(1);
        expect(label(container)).toBe("Image 2 of 3: b.png");
        expect(container.querySelector(".pos")?.textContent).toMatch(/2\s*\/\s*3/);
        await waitFor(() => expect(container.querySelector("img")?.getAttribute("src")).toBe("blob:1"));
        expect(h.fetch.mock.calls[0][0]).toBe("D:/p/b.png");
    });

    it("steps with the arrows, wrapping at both ends, and Home and End jump", () => {
        const { container } = mount(0);
        key(container, "ArrowLeft");
        expect(label(container)).toBe("Image 3 of 3: c.png");
        key(container, "ArrowRight");
        expect(label(container)).toBe("Image 1 of 3: a.png");
        key(container, "ArrowRight");
        expect(label(container)).toBe("Image 2 of 3: b.png");
        key(container, "End");
        expect(label(container)).toBe("Image 3 of 3: c.png");
        key(container, "Home");
        expect(label(container)).toBe("Image 1 of 3: a.png");
    });

    it("loads the shown image first and then the one on each side", async () => {
        mount(0);
        await waitFor(() => expect(h.fetch).toHaveBeenCalledTimes(3));
        expect(h.fetch.mock.calls.map((c) => c[0])).toEqual(["D:/p/a.png", "D:/p/b.png", "D:/p/c.png"]);
    });

    it("lets an image two steps away go", async () => {
        const five = ["a", "b", "c", "d", "e"].map((n) => ({ path: `D:/p/${n}.png`, name: `${n}.png`, alt: "" }));
        const { container } = mount(0, five);
        await waitFor(() => expect(h.fetch).toHaveBeenCalledTimes(3));
        key(container, "ArrowRight");
        key(container, "ArrowRight");
        await waitFor(() => expect(h.revoked.length).toBeGreaterThan(0));
    });

    it("closes on Escape, and opens the shown image in the Editor on E", () => {
        const { container, onClose, onOpenEditor } = mount(2);
        key(container, "e");
        expect(onOpenEditor).toHaveBeenCalledWith("D:/p/c.png");
        key(container, "Escape");
        expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("ignores the arrows with a modifier held, so a shortcut isn't taken", () => {
        const { container } = mount(0);
        fireEvent.keyDown(dialog(container), { key: "ArrowRight", ctrlKey: true });
        expect(label(container)).toBe("Image 1 of 3: a.png");
    });

    it("says an image that can't load is gone, and stepping past it still works", async () => {
        h.fetch.mockImplementation(async (path: string) => {
            if (path.endsWith("b.png")) throw new Error("404");
            return new Blob([path], { type: "image/png" });
        });
        const { container } = mount(1);
        await waitFor(() => expect(container.textContent).toContain("This image is no longer available."));
        key(container, "ArrowRight");
        await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
        expect(label(container)).toBe("Image 3 of 3: c.png");
    });

    it("names the alt text when it differs from the file name, and reads it out", () => {
        const { container } = mount(0, [{ path: "D:/p/a.png", name: "a.png", alt: "the toolbar" }, items[1]]);
        expect(label(container)).toBe("Image 1 of 2: the toolbar");
        expect(container.querySelector(".agent-attachment-lightbox__live")?.textContent).toBe("Image 1 of 2: the toolbar");
    });
});
