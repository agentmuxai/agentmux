// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A toast slides in when it appears. When it goes, the stack drops it at once and a copy fades out
// where it was; with motion off nothing is copied.

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const reduce = vi.hoisted(() => ({ categories: new Set<string>() }));
vi.mock("@/app/motion/motion", () => ({
    shouldReduceMotion: (c: string) => reduce.categories.has(c),
    scaledMs: (ms: number) => ms,
}));

import { installFakeAnimate, type FakeAnimate } from "@/app/motion/test-utils";
import { NotificationItem } from "./notificationitem";

const notification = { id: "n1", title: "Saved", message: "All done", type: "info", timestamp: "" } as NotificationType;

function mount(isBubble = true) {
    return render(() => (
        <NotificationItem
            notification={notification}
            onRemove={() => {}}
            onCopy={() => {}}
            onActionClick={() => {}}
            formatTimestamp={() => ""}
            isBubble={isBubble}
        />
    ));
}

let fake: FakeAnimate;
beforeEach(() => {
    fake = installFakeAnimate();
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
        left: 600,
        top: 300,
        right: 980,
        bottom: 380,
        width: 380,
        height: 80,
        x: 600,
        y: 300,
        toJSON: () => ({}),
    });
});
afterEach(() => {
    cleanup();
    fake.restore();
    reduce.categories.clear();
    vi.restoreAllMocks();
    document.body.innerHTML = "";
});

describe("a toast's motion", () => {
    it("slides in when it appears", () => {
        const { container } = mount();
        const toast = container.querySelector(".notification-bubble");
        expect(fake.calls).toHaveLength(1);
        expect(fake.calls[0].el).toBe(toast);
        expect(fake.calls[0].keyframes[0]).toMatchObject({ opacity: 0 });
        expect(fake.calls[0].keyframes[0].transform).toMatch(/^translateY\(/);
        expect(fake.calls[0].keyframes[1]).toMatchObject({ opacity: 1 });
    });

    it("leaves a copy that fades out in its place, and the copy removes itself", async () => {
        const { container } = mount();
        fake.finishAll();
        cleanup();
        expect(container.querySelector(".notification-bubble")).toBeNull();
        const copy = document.body.querySelector<HTMLElement>('body > .notification-bubble[aria-hidden="true"]');
        expect(copy).not.toBeNull();
        expect(copy!.style.position).toBe("fixed");
        expect(copy!.style.left).toBe("600px");
        expect(copy!.style.top).toBe("300px");
        expect(copy!.style.pointerEvents).toBe("none");
        expect(copy!.hasAttribute("inert")).toBe(true);
        const exiting = fake.calls[fake.calls.length - 1];
        expect(exiting.el).toBe(copy);
        expect(exiting.keyframes[1]).toMatchObject({ opacity: 0 });
        exiting.finish();
        await Promise.resolve();
        expect(copy!.isConnected).toBe(false);
    });

    it("plays nothing and copies nothing when motion is off", () => {
        reduce.categories.add("transition");
        mount();
        cleanup();
        expect(fake.calls).toHaveLength(0);
        expect(document.body.querySelector(".notification-bubble")).toBeNull();
    });

    it("leaves the notification list's rows alone", () => {
        mount(false);
        cleanup();
        expect(fake.calls).toHaveLength(0);
    });
});
