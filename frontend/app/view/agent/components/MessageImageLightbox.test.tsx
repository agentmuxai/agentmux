// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The message image preview: the keys, the neighbours it loads, a file that can't load, and the labels. */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

const h = vi.hoisted(() => ({ fetch: vi.fn(), revoked: [] as string[] }));

vi.mock("@/app/element/modal", () => ({
    Modal: (p: { ariaLabel?: string; scope?: string; children?: unknown }) => (
        <div data-stand="Modal" data-scope={p.scope} data-aria-label={p.ariaLabel}>
            {p.children as any}
        </div>
    ),
}));
vi.mock("@/app/element/local-media", async (orig) => ({
    ...(await orig<typeof import("@/app/element/local-media")>()),
    fetchMediaBlob: (path: string) => h.fetch(path),
}));

import type { MediaGalleryItem, VideoHandoff } from "@/app/element/markdown-media";
import { MessageImageLightbox } from "./MessageImageLightbox";

const items = ["a.png", "b.png", "c.png"].map((name) => ({ path: `D:/p/${name}`, name, alt: "" }));

type Spies = { onClose: Mock<(back?: unknown) => void>; onOpenEditor: Mock<(path: string) => void> };

function mount(index = 0, list: MediaGalleryItem[] = items, start?: VideoHandoff): Spies & ReturnType<typeof render> {
    const spies: Spies = { onClose: vi.fn<(back?: unknown) => void>(), onOpenEditor: vi.fn<(path: string) => void>() };
    const r = render(() => <MessageImageLightbox items={list} index={index} start={start} {...spies} />);
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

    it("keeps the shown image when the message repeats it and its first place is far away", async () => {
        const at = (n: string) => ({ path: `D:/p/${n}.png`, name: `${n}.png`, alt: "" });
        const repeated = [at("a"), at("b"), at("c"), at("a"), at("d"), at("e")];
        const { container } = mount(3, repeated);
        await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
        key(container, "ArrowRight");
        key(container, "ArrowLeft");
        await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
        expect(container.textContent).not.toContain("Loading");
        expect(label(container)).toBe("Image 4 of 6: a.png");
    });

    it("makes no blob URL for a file that finished loading after the preview let it go", async () => {
        const five = ["a", "b", "c", "d", "e"].map((n) => ({ path: `D:/p/${n}.png`, name: `${n}.png`, alt: "" }));
        const finish: Record<string, () => void> = {};
        h.fetch.mockImplementation(
            (path: string) => new Promise((done) => (finish[path] = () => done(new Blob([path], { type: "image/png" })))),
        );
        const create = vi.spyOn(URL, "createObjectURL");
        const { container } = mount(0, five);
        await waitFor(() => expect(finish["D:/p/a.png"]).toBeDefined());
        // Two steps on, "a" is let go while its fetch is still in flight.
        key(container, "ArrowRight");
        key(container, "ArrowRight");
        finish["D:/p/a.png"]();
        await new Promise((r) => setTimeout(r, 0));
        expect(create).not.toHaveBeenCalled();
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

    it("is drawn over the whole tab, with the shared preview shell", () => {
        const { container } = mount(0);
        expect(container.querySelector("[data-stand=Modal]")?.getAttribute("data-scope")).toBe("tab");
        expect(container.querySelector(".agent-attachment-lightbox__dialog--fit")).not.toBeNull();
    });

    it("never draws a picture past its own size: its box only shrinks, and nothing sets it a width", () => {
        const scss = readFileSync(join(__dirname, "../styles/_attachments.scss"), "utf8");
        const fit = scss.slice(scss.indexOf("&__image--fit {"), scss.indexOf("// What the keys are"));
        expect(fit).toContain("max-width: 100%");
        expect(fit).toContain("flex: 0 1 auto");
        // Only the actual-size state lifts the cap, and that is the person's choice.
        expect(fit.split("max-width: none")).toHaveLength(2);
        expect(fit.indexOf("max-width: none")).toBeGreaterThan(fit.indexOf("&.is-actual"));
    });
});

describe("MessageImageLightbox: actual size", () => {
    const actual = (c: HTMLElement) => c.querySelector(".agent-attachment-lightbox__image")!.classList.contains("is-actual");

    it("a click on the picture, or the 1 key, switches between fitting the tab and its actual size", async () => {
        const { container } = mount(0);
        await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
        expect(actual(container)).toBe(false);
        fireEvent.click(container.querySelector("img")!);
        expect(actual(container)).toBe(true);
        key(container, "1");
        expect(actual(container)).toBe(false);
        key(container, "1");
        expect(actual(container)).toBe(true);
    });

    it("goes back to fitting when another picture is shown", async () => {
        const { container } = mount(0);
        await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
        key(container, "1");
        key(container, "ArrowRight");
        expect(actual(container)).toBe(false);
    });

    it("a click on the picture does not close the preview", async () => {
        const { container, onClose } = mount(0);
        await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
        fireEvent.click(container.querySelector("img")!);
        expect(onClose).not.toHaveBeenCalled();
    });
});

describe("MessageImageLightbox: videos", () => {
    const media: MediaGalleryItem[] = [
        { path: "D:/p/a.png", name: "a.png", alt: "", kind: "image" },
        { path: "D:/p/clip.mp4", name: "clip.mp4", alt: "", kind: "video" },
        { path: "D:/p/b.png", name: "b.png", alt: "", kind: "image" },
    ];
    const video = (c: HTMLElement) => c.querySelector<HTMLVideoElement>("video")!;
    let play: ReturnType<typeof vi.fn>;
    let pause: ReturnType<typeof vi.fn>;

    beforeEach(() => {
        play = vi.fn(async () => {});
        pause = vi.fn();
        vi.spyOn(HTMLMediaElement.prototype, "play").mockImplementation(play as never);
        vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(pause as never);
    });

    it("steps through images and videos in order, naming the kind", async () => {
        const { container } = mount(0, media);
        expect(label(container)).toBe("Image 1 of 3: a.png");
        key(container, "ArrowRight");
        expect(label(container)).toBe("Video 2 of 3: clip.mp4");
        await waitFor(() => expect(video(container)).not.toBeNull());
        expect(video(container).hasAttribute("controls")).toBe(true);
        expect(container.querySelector("img")).toBeNull();
        key(container, "ArrowRight");
        expect(label(container)).toBe("Image 3 of 3: b.png");
    });

    it("reads a video only when it is shown, and lets it go when stepped away from", async () => {
        const { container } = mount(0, media);
        await waitFor(() => expect(h.fetch).toHaveBeenCalledTimes(2));
        expect(h.fetch.mock.calls.map((c) => c[0])).toEqual(["D:/p/a.png", "D:/p/b.png"]);
        key(container, "ArrowRight");
        await waitFor(() => expect(h.fetch).toHaveBeenCalledWith("D:/p/clip.mp4"));
        key(container, "ArrowRight");
        await waitFor(() => expect(h.revoked.length).toBeGreaterThan(0));
    });

    it("starts from the inline player's time and sound, and plays on when it was playing", async () => {
        const { container } = mount(1, media, { path: "D:/p/clip.mp4", time: 42, muted: false, paused: false });
        await waitFor(() => expect(video(container)).not.toBeNull());
        const el = video(container);
        expect(el.muted).toBe(false);
        Object.defineProperty(el, "currentTime", { value: 0, writable: true, configurable: true });
        el.dispatchEvent(new Event("loadedmetadata"));
        expect(el.currentTime).toBe(42);
        expect(play).toHaveBeenCalledTimes(1);
    });

    it("starts a video that was not playing muted and paused, and never plays by itself", async () => {
        const { container } = mount(1, media);
        await waitFor(() => expect(video(container)).not.toBeNull());
        expect(video(container).muted).toBe(true);
        expect(play).not.toHaveBeenCalled();
    });

    it("hands the position and sound back when it closes", async () => {
        const { container, onClose } = mount(1, media);
        await waitFor(() => expect(video(container)).not.toBeNull());
        const el = video(container);
        Object.defineProperty(el, "currentTime", { value: 7.5, writable: true, configurable: true });
        el.muted = false;
        key(container, "Escape");
        expect(onClose).toHaveBeenLastCalledWith({ path: "D:/p/clip.mp4", time: 7.5, muted: false });
    });

    it("hands back nothing when an image was on show", async () => {
        const { container, onClose } = mount(0, media);
        await waitFor(() => expect(container.querySelector("img")).not.toBeNull());
        key(container, "Escape");
        expect(onClose).toHaveBeenLastCalledWith(undefined);
    });

    it("Space plays and pauses, J and L skip ten seconds, and the arrows still step", async () => {
        const { container } = mount(1, media);
        await waitFor(() => expect(video(container)).not.toBeNull());
        const el = video(container);
        Object.defineProperty(el, "paused", { value: true, configurable: true });
        Object.defineProperty(el, "duration", { value: 100, configurable: true });
        Object.defineProperty(el, "currentTime", { value: 50, writable: true, configurable: true });
        key(container, " ");
        expect(play).toHaveBeenCalledTimes(1);
        Object.defineProperty(el, "paused", { value: false, configurable: true });
        key(container, " ");
        expect(pause).toHaveBeenCalledTimes(1);
        key(container, "l");
        expect(el.currentTime).toBe(60);
        key(container, "j");
        key(container, "J");
        expect(el.currentTime).toBe(40);
        key(container, "ArrowRight");
        expect(label(container)).toBe("Image 3 of 3: b.png");
    });

    it("leaves Space to the player when it has the focus, so it never toggles twice", async () => {
        const { container } = mount(1, media);
        await waitFor(() => expect(video(container)).not.toBeNull());
        fireEvent.keyDown(video(container), { key: " " });
        expect(play).not.toHaveBeenCalled();
        expect(pause).not.toHaveBeenCalled();
    });

    it("says a video that can't load is gone, and the 1 key does nothing to it", async () => {
        h.fetch.mockImplementation(async (path: string) => {
            if (path.endsWith("clip.mp4")) throw new Error("404");
            return new Blob([path], { type: "image/png" });
        });
        const { container } = mount(1, media);
        await waitFor(() => expect(container.textContent).toContain("This video is no longer available."));
        key(container, "1");
        expect(container.querySelector(".agent-attachment-lightbox__image")!.classList.contains("is-actual")).toBe(false);
    });

    it("tells the keys that apply to what is shown", () => {
        const { container } = mount(0, media);
        expect(container.querySelector(".agent-attachment-lightbox__hint")?.textContent).toContain("actual size");
        key(container, "ArrowRight");
        expect(container.querySelector(".agent-attachment-lightbox__hint")?.textContent).toContain("Space play");
    });
});
