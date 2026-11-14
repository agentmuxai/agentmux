// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The landing place of a tab pill that leaves other tabs behind (PaneLanding.tsx): the tab's own picture at
 * the pane's size, the empty pane where there is none, and one asking per drag.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { peeks, picture, ended } = vi.hoisted(() => ({
    peeks: [] as string[],
    picture: { current: undefined as unknown },
    ended: [] as Array<() => void>,
}));

vi.mock("@/app/drag/tearoff-snapshot", () => ({
    peekTearOffSnapshot: (blockId: string) => {
        peeks.push(blockId);
        return picture.current instanceof Error ? Promise.reject(picture.current) : Promise.resolve(picture.current);
    },
}));
vi.mock("@/app/drag/drag-session", () => ({
    onSessionEnded: (listener: () => void) => {
        ended.push(listener);
        return () => {};
    },
}));
vi.mock("@/app/element/EmptyPaneLogo", () => ({
    EmptyPaneLogo: () => <div class="empty-pane-logo" />,
}));

import { PaneLanding, resetPaneLandingForTests } from "./PaneLanding";

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

beforeEach(() => {
    peeks.length = 0;
    picture.current = undefined;
    resetPaneLandingForTests();
});
afterEach(cleanup);

describe("PaneLanding", () => {
    it("shows the tab's picture at the pane's own size", async () => {
        picture.current = { picture: "QUJD", grab: { x: 4, y: 5 }, size: { width: 640, height: 480 } };
        const { container } = render(() => <PaneLanding blockId="b1" />);
        await settle();
        const img = container.querySelector(".pane-landing img") as HTMLImageElement;
        expect(img.getAttribute("src")).toBe("data:image/jpeg;base64,QUJD");
        expect(img.style.width).toBe("640px");
        expect(img.style.height).toBe("480px");
        expect(container.querySelector(".empty-pane-logo")).toBeNull();
        expect(container.querySelector(".pane-landing")?.getAttribute("aria-hidden")).toBe("true");
    });

    it("is an empty pane until the picture comes, and when there is none", async () => {
        const { container } = render(() => <PaneLanding blockId="b1" />);
        expect(container.querySelector(".pane-landing .empty-pane-logo")).not.toBeNull();
        await settle();
        expect(container.querySelector(".pane-landing img")).toBeNull();
        expect(container.querySelector(".pane-landing .empty-pane-logo")).not.toBeNull();
    });

    it("is an empty pane for a picture of nothing, and for one that could not be read", async () => {
        picture.current = { picture: "QUJD", grab: { x: 0, y: 0 }, size: { width: 0, height: 0 } };
        const first = render(() => <PaneLanding blockId="b1" />);
        await settle();
        expect(first.container.querySelector("img")).toBeNull();
        first.unmount();

        resetPaneLandingForTests();
        picture.current = new Error("no host");
        const second = render(() => <PaneLanding blockId="b2" />);
        await settle();
        expect(second.container.querySelector("img")).toBeNull();
        expect(second.container.querySelector(".empty-pane-logo")).not.toBeNull();
    });

    it("asks for the picture once in a drag, however often the place comes back, and again in the next drag", async () => {
        picture.current = { picture: "QUJD", grab: { x: 0, y: 0 }, size: { width: 10, height: 10 } };
        const first = render(() => <PaneLanding blockId="b1" />);
        await settle();
        first.unmount();
        const again = render(() => <PaneLanding blockId="b1" />);
        await settle();
        expect(again.container.querySelector("img")).not.toBeNull();
        expect(peeks).toEqual(["b1"]);
        again.unmount();

        for (const end of ended) end();
        render(() => <PaneLanding blockId="b1" />);
        await settle();
        expect(peeks).toEqual(["b1", "b1"]);
    });
});
