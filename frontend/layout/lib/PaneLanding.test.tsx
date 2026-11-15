// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The landing place of a tab pill that leaves other tabs behind (PaneLanding.tsx, paneLandingPicture.ts): the
 * tab's picture as it stands when its placing starts, the press's picture where that cannot be taken, the
 * empty pane where there is none, and one picture per drag.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const { asked, now, press, ended } = vi.hoisted(() => ({
    asked: [] as string[],
    now: { current: undefined as unknown },
    press: { current: undefined as unknown },
    ended: [] as Array<() => void>,
}));

vi.mock("@/app/drag/tearoff-snapshot", () => ({
    picturePaneNow: (blockId: string) => {
        asked.push(`now:${blockId}`);
        return now.current instanceof Error ? Promise.reject(now.current) : Promise.resolve(now.current);
    },
    peekTearOffSnapshot: (blockId: string) => {
        asked.push(`press:${blockId}`);
        return Promise.resolve(press.current);
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

import { PaneLanding } from "./PaneLanding";
import { landingPictureOf, resetLandingPictureForTests } from "./paneLandingPicture";

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
const size = { width: 640, height: 480 };

beforeEach(() => {
    asked.length = 0;
    now.current = undefined;
    press.current = undefined;
    resetLandingPictureForTests();
});
afterEach(cleanup);

describe("PaneLanding", () => {
    it("shows the tab as it stands when its placing starts", async () => {
        now.current = { picture: "Tk9X", size };
        press.current = { picture: "UFJFU1M=", grab: { x: 1, y: 2 }, size };
        const { container } = render(() => <PaneLanding blockId="b1" />);
        await settle();
        const img = container.querySelector(".pane-landing img") as HTMLImageElement;
        expect(img.getAttribute("src")).toBe("data:image/jpeg;base64,Tk9X");
        expect(container.querySelector(".empty-pane-logo")).toBeNull();
        expect(container.querySelector(".pane-landing")?.getAttribute("aria-hidden")).toBe("true");
        expect(asked).toEqual(["now:b1"]);
    });

    it("shows the press's picture where the pane cannot be pictured now", async () => {
        press.current = { picture: "UFJFU1M=", grab: { x: 1, y: 2 }, size };
        const { container } = render(() => <PaneLanding blockId="b1" />);
        await settle();
        expect(container.querySelector(".pane-landing img")?.getAttribute("src")).toBe("data:image/jpeg;base64,UFJFU1M=");
        expect(asked).toEqual(["now:b1", "press:b1"]);
    });

    it("is an empty pane until a picture comes, when there is none, and when it is of nothing or cannot be read", async () => {
        const none = render(() => <PaneLanding blockId="b1" />);
        expect(none.container.querySelector(".pane-landing .empty-pane-logo")).not.toBeNull();
        await settle();
        expect(none.container.querySelector("img")).toBeNull();
        expect(none.container.querySelector(".empty-pane-logo")).not.toBeNull();
        none.unmount();

        resetLandingPictureForTests();
        now.current = { picture: "Tk9X", size: { width: 0, height: 0 } };
        const nothing = render(() => <PaneLanding blockId="b2" />);
        await settle();
        expect(nothing.container.querySelector("img")).toBeNull();
        nothing.unmount();

        resetLandingPictureForTests();
        now.current = new Error("no host");
        const failed = render(() => <PaneLanding blockId="b3" />);
        await settle();
        expect(failed.container.querySelector("img")).toBeNull();
        expect(failed.container.querySelector(".empty-pane-logo")).not.toBeNull();
    });

    it("takes one picture in a drag, however often the place comes back, and a new one in the next drag", async () => {
        now.current = { picture: "Tk9X", size };
        // The placing takes it first (pillPlacement.ts); the place then shows that one.
        void landingPictureOf("b1");
        const first = render(() => <PaneLanding blockId="b1" />);
        await settle();
        first.unmount();
        const again = render(() => <PaneLanding blockId="b1" />);
        await settle();
        expect(again.container.querySelector("img")).not.toBeNull();
        expect(asked).toEqual(["now:b1"]);
        again.unmount();

        for (const end of ended) end();
        render(() => <PaneLanding blockId="b1" />);
        await settle();
        expect(asked).toEqual(["now:b1", "now:b1"]);
    });
});
