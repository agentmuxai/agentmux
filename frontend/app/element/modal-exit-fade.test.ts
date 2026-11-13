// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.unmock("@/app/element/modal-exit-fade");

import { fadeOutBackdrop } from "./modal-exit-fade";

const motion = vi.hoisted(() => ({ reduced: false, ms: 120 }));
vi.mock("@/app/motion/motion", () => ({
    shouldReduceMotion: () => motion.reduced,
    scaledMs: () => motion.ms,
}));

function modalRoot(mount: HTMLElement): HTMLElement {
    const root = document.createElement("div");
    root.className = "modal-root";
    root.setAttribute("role", "dialog");
    root.setAttribute("aria-modal", "true");
    root.setAttribute("aria-label", "Sign in");
    root.setAttribute("data-scope", "pane");
    root.tabIndex = -1;
    const panel = document.createElement("div");
    panel.className = "modal-panel";
    root.append(panel);
    mount.append(root);
    return root;
}

describe("fadeOutBackdrop", () => {
    let mount: HTMLElement;
    beforeEach(() => {
        vi.useFakeTimers();
        motion.reduced = false;
        motion.ms = 120;
        mount = document.createElement("div");
        document.body.append(mount);
    });
    afterEach(() => {
        vi.useRealTimers();
        mount.remove();
    });

    it("leaves a hidden stand-in with only a backdrop where the modal was", () => {
        const root = modalRoot(mount);
        fadeOutBackdrop(root, mount);
        const leaving = mount.querySelector(".modal-leaving") as HTMLElement;
        expect(leaving).not.toBeNull();
        expect(leaving.getAttribute("data-scope")).toBe("pane");
        expect(leaving.getAttribute("aria-hidden")).toBe("true");
        expect(leaving.getAttribute("role")).toBeNull();
        expect(leaving.getAttribute("aria-label")).toBeNull();
        expect(leaving.classList.contains("modal-root")).toBe(false);
        expect(leaving.querySelector(".modal-backdrop")).not.toBeNull();
        expect(leaving.querySelector(".modal-panel")).toBeNull();
        // The modal's own root is left alone.
        expect(root.getAttribute("role")).toBe("dialog");
    });

    it("removes the copy when the fade ends, or after the fade's length if it never reports", () => {
        fadeOutBackdrop(modalRoot(mount), mount);
        mount.querySelector(".modal-leaving .modal-backdrop")!.dispatchEvent(new Event("animationend"));
        expect(mount.querySelector(".modal-leaving")).toBeNull();

        fadeOutBackdrop(modalRoot(mount), mount);
        expect(mount.querySelector(".modal-leaving")).not.toBeNull();
        vi.advanceTimersByTime(200);
        expect(mount.querySelector(".modal-leaving")).toBeNull();
    });

    it("draws nothing when transitions are reduced, the duration is nothing, or there is no root", () => {
        motion.reduced = true;
        fadeOutBackdrop(modalRoot(mount), mount);
        expect(mount.querySelector(".modal-leaving")).toBeNull();

        motion.reduced = false;
        motion.ms = 0;
        fadeOutBackdrop(modalRoot(mount), mount);
        expect(mount.querySelector(".modal-leaving")).toBeNull();

        motion.ms = 120;
        fadeOutBackdrop(null, mount);
        fadeOutBackdrop(modalRoot(mount), null);
        expect(mount.querySelector(".modal-leaving")).toBeNull();
    });
});
