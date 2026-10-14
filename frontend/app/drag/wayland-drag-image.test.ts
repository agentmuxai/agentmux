// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";

const getWindowSystem = vi.fn<() => Promise<string>>();

vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ getWindowSystem }) }));

import { isNativeWayland, loadWindowSystem, resetWindowSystemForTest } from "@/app/host/window-system";
import { hideDragImageOnNativeWayland } from "./wayland-drag-image";

afterEach(() => {
    resetWindowSystemForTest();
    getWindowSystem.mockReset();
});

describe("window system", () => {
    it("is native Wayland only once the host says so", async () => {
        getWindowSystem.mockResolvedValue("wayland");
        expect(isNativeWayland()).toBe(false);
        await loadWindowSystem();
        expect(isNativeWayland()).toBe(true);
    });

    it("is not native Wayland on X11 or XWayland", async () => {
        getWindowSystem.mockResolvedValue("x11");
        await loadWindowSystem();
        expect(isNativeWayland()).toBe(false);
    });

    it("is not native Wayland when the host can't answer", async () => {
        getWindowSystem.mockRejectedValue(new Error("unknown command"));
        await loadWindowSystem();
        expect(isNativeWayland()).toBe(false);
    });
});

describe("hideDragImageOnNativeWayland", () => {
    it("gives the drag a transparent picture on native Wayland", async () => {
        getWindowSystem.mockResolvedValue("wayland");
        await loadWindowSystem();
        const setDragImage = vi.fn();
        expect(hideDragImageOnNativeWayland(setDragImage)).toBe(true);
        expect(setDragImage).toHaveBeenCalledOnce();
        const [image, x, y] = setDragImage.mock.calls[0];
        expect(image).toBeInstanceOf(HTMLCanvasElement);
        expect([image.width, image.height, x, y]).toEqual([1, 1, 0, 0]);
    });

    it("leaves the picture alone elsewhere", async () => {
        getWindowSystem.mockResolvedValue("x11");
        await loadWindowSystem();
        const setDragImage = vi.fn();
        expect(hideDragImageOnNativeWayland(setDragImage)).toBe(false);
        expect(setDragImage).not.toHaveBeenCalled();
    });

    it("does nothing without a way to set the picture", async () => {
        getWindowSystem.mockResolvedValue("wayland");
        await loadWindowSystem();
        expect(hideDragImageOnNativeWayland(null)).toBe(false);
    });
});
