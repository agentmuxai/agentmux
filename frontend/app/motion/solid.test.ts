// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./motion", () => ({ shouldReduceMotion: () => false, scaledMs: (ms: number) => ms }));

import { useMotion, usePulseWhen } from "./solid";
import { installFakeAnimate, type FakeAnimate } from "./test-utils";

let fake: FakeAnimate;
beforeEach(() => {
    fake = installFakeAnimate();
});
afterEach(() => {
    fake.restore();
    document.body.innerHTML = "";
});

describe("useMotion", () => {
    it("cancels a running motion, and removes its copy, when the owner is disposed", () => {
        const el = document.body.appendChild(document.createElement("div"));
        el.getBoundingClientRect = () => ({ left: 100, top: 100, width: 10, height: 10 }) as DOMRect;
        let travelling!: { cancelled: boolean };
        const dispose = createRoot((dispose) => {
            const motion = useMotion();
            travelling = motion.travel(el, { left: 0, top: 0 } as DOMRect);
            return dispose;
        });
        const copy = fake.calls[0].el;
        expect(copy.isConnected).toBe(true);
        expect(el.style.visibility).toBe("hidden");
        dispose();
        expect(copy.isConnected).toBe(false);
        expect(el.style.visibility).toBe("");
        expect(travelling.cancelled).toBe(true);
    });

    it("leaves a motion that already finished alone", async () => {
        const el = document.body.appendChild(document.createElement("div"));
        const cleanup = vi.fn();
        const dispose = createRoot((dispose) => {
            useMotion().play(el, [], { category: "feedback", duration: "fast", easing: "out", cleanup });
            return dispose;
        });
        fake.calls[0].finish();
        await Promise.resolve();
        dispose();
        expect(cleanup).toHaveBeenCalledTimes(1);
    });
});

describe("usePulseWhen", () => {
    it("pulses the element each time the flag turns true, and not while it stays true", async () => {
        const { createSignal } = await import("solid-js");
        const el = document.body.appendChild(document.createElement("div"));
        const [flag, setFlag] = createSignal(false);
        const dispose = createRoot((dispose) => {
            usePulseWhen(() => el, flag);
            return dispose;
        });
        expect(fake.calls).toHaveLength(0);
        setFlag(true);
        expect(fake.calls).toHaveLength(1);
        expect(fake.calls[0].el).toBe(el);
        expect(fake.calls[0].keyframes[0]).toMatchObject({ transform: "scaleX(0.88)" });
        setFlag(false);
        setFlag(true);
        expect(fake.calls).toHaveLength(2);
        dispose();
    });

    it("pulses at once when the flag starts true, and cancels the pulse with its owner", () => {
        const el = document.body.appendChild(document.createElement("div"));
        const dispose = createRoot((dispose) => {
            usePulseWhen(
                () => el,
                () => true
            );
            return dispose;
        });
        expect(fake.calls).toHaveLength(1);
        dispose();
        expect(fake.calls[0].ended).toBe(true);
    });
});
