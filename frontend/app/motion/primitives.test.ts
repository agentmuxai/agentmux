// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
//
// docs/specs/SPEC_MOTION_PRIMITIVES_2026_10_29.md §7.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const reduce = vi.hoisted(() => ({ categories: new Set<string>() }));
vi.mock("./motion", () => ({ shouldReduceMotion: (c: string) => reduce.categories.has(c) }));

import {
    DURATION,
    EASE,
    MOTION,
    capture,
    enter,
    exit,
    finishedHandle,
    glide,
    groupHandle,
    mergeInto,
    move,
    play,
    pulse,
    travel,
} from "./primitives";
import { installFakeAnimate, setVisibility, type FakeAnimate } from "./test-utils";

let fake: FakeAnimate;
beforeEach(() => {
    reduce.categories.clear();
    fake = installFakeAnimate();
});
afterEach(() => {
    fake.restore();
    document.body.innerHTML = "";
});

const opts = { category: "transition", duration: "base", easing: "out" } as const;
const div = () => document.body.appendChild(document.createElement("div"));

/** An element whose rectangle is whatever the test says, and can be changed. */
function at(left: number, top: number, width = 100, height = 20) {
    const el = div();
    const rect = { left, top, width, height, right: left + width, bottom: top + height };
    el.getBoundingClientRect = () => rect as DOMRect;
    return Object.assign(el, { place: (l: number, t: number) => Object.assign(rect, { left: l, top: t }) });
}
const rectOf = (left: number, top: number) => ({ left, top, width: 100, height: 20 }) as DOMRect;

describe("play", () => {
    it("animates with the token values and runs cleanup once when it finishes", () => {
        const cleanup = vi.fn();
        const el = div();
        const handle = play(el, [{ opacity: 0 }, { opacity: 1 }], {
            ...opts,
            easing: "emphasized",
            duration: "moderate",
            cleanup,
        });
        expect(fake.calls).toHaveLength(1);
        expect(fake.calls[0].options).toMatchObject({ duration: DURATION.moderate, easing: EASE.emphasized });
        expect(cleanup).not.toHaveBeenCalled();
        fake.finishAll();
        fake.calls[0].finish();
        expect(cleanup).toHaveBeenCalledTimes(1);
        expect(handle.cancelled).toBe(false);
    });

    it("runs cleanup once on cancel, resolves finished, and cancel after the end does nothing", async () => {
        const cleanup = vi.fn();
        const handle = play(div(), [], { ...opts, cleanup });
        handle.cancel();
        handle.cancel();
        await handle.finished;
        expect(cleanup).toHaveBeenCalledTimes(1);
        expect(handle.cancelled).toBe(true);
        expect(fake.calls[0].ended).toBe(true);

        const done = vi.fn();
        const second = play(div(), [], { ...opts, cleanup: done });
        fake.calls[1].finish();
        second.cancel();
        expect(done).toHaveBeenCalledTimes(1);
        expect(second.cancelled).toBe(false);
    });

    it("lets a second play with the same key replace the first, cleaning it up", () => {
        const el = div();
        const first = vi.fn();
        const a = play(el, [], { ...opts, key: "k", cleanup: first });
        const b = play(el, [], { ...opts, key: "k" });
        expect(a.cancelled).toBe(true);
        expect(first).toHaveBeenCalledTimes(1);
        expect(b.cancelled).toBe(false);
        // A different key, or another element, is left alone.
        const c = play(el, [], { ...opts, key: "other" });
        const d = play(div(), [], { ...opts, key: "k" });
        expect(b.cancelled || c.cancelled || d.cancelled).toBe(false);
        // Once the first has ended, its key is free again.
        fake.calls[1].finish();
        const e = play(el, [], { ...opts, key: "k" });
        expect(e.cancelled).toBe(false);
        expect(b.cancelled).toBe(false);
    });

    it("starts nothing, and still cleans up, when the policy asks for no motion in the category", async () => {
        reduce.categories.add("transition");
        const cleanup = vi.fn();
        const handle = play(div(), [], { ...opts, cleanup });
        await handle.finished;
        expect(fake.calls).toHaveLength(0);
        expect(cleanup).toHaveBeenCalledTimes(1);
        // Another category is unaffected.
        play(div(), [], { ...opts, category: "feedback" });
        expect(fake.calls).toHaveLength(1);
    });

    it("starts nothing on a hidden page", () => {
        const restore = setVisibility("hidden");
        const cleanup = vi.fn();
        play(div(), [], { ...opts, cleanup });
        restore();
        expect(fake.calls).toHaveLength(0);
        expect(cleanup).toHaveBeenCalledTimes(1);
    });

    it("hands back a finished handle, with cleanup run, where the engine has no animate", async () => {
        fake.restore();
        const cleanup = vi.fn();
        const handle = play(div(), [], { ...opts, cleanup });
        await handle.finished;
        expect(cleanup).toHaveBeenCalledTimes(1);
        expect(handle.cancelled).toBe(false);
        fake = installFakeAnimate();
    });

    it("never throws into the caller: not from animate, not from cleanup", async () => {
        fake.restore();
        fake = installFakeAnimate({ fail: true });
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const cleanup = vi.fn();
        const handle = play(div(), [], { ...opts, cleanup });
        await handle.finished;
        expect(cleanup).toHaveBeenCalledTimes(1);

        fake.restore();
        fake = installFakeAnimate();
        const throwing = play(div(), [], {
            ...opts,
            cleanup: () => {
                throw new Error("boom");
            },
        });
        expect(() => throwing.cancel()).not.toThrow();
        await throwing.finished;
        warn.mockRestore();
    });
});

describe("move", () => {
    const keyOf = (el: Element) => el.getAttribute("data-k");
    const row = (k: string, left: number, top: number) => {
        const el = at(left, top);
        el.setAttribute("data-k", k);
        return el;
    };

    it("plays from the old place for an element that moved, and not for one that moved under a pixel", () => {
        const a = row("a", 0, 10);
        const b = row("b", 0, 60);
        const before = capture([a, b], keyOf);
        a.place(0, 70);
        b.place(0, 60.5);
        const handle = move(before, [a, b], keyOf);
        expect(fake.calls).toHaveLength(1);
        expect(fake.calls[0].el).toBe(a);
        expect(fake.calls[0].keyframes).toEqual([{ transform: "translate(0px, -60px)" }, { transform: "none" }]);
        expect(fake.calls[0].options).toMatchObject({ duration: DURATION.base, easing: EASE.out });
        expect(handle.cancelled).toBe(false);
    });

    it("moves on one axis only when asked", () => {
        const a = row("a", 5, 10);
        const before = capture([a], keyOf);
        a.place(25, 40);
        move(before, [a], keyOf, { axis: "y" });
        expect(fake.calls[0].keyframes[0]).toEqual({ transform: "translateY(-30px)" });
        move(before, [a], keyOf, { axis: "x" });
        expect(fake.calls[1].keyframes[0]).toEqual({ transform: "translateX(-20px)" });
    });

    it("divides the distance by the zoom of the box the elements sit in", () => {
        const a = row("a", 0, 100);
        const before = capture([a], keyOf);
        a.place(0, 0);
        move(before, [a], keyOf, { axis: "y", zoom: 2 });
        expect(fake.calls[0].keyframes[0]).toEqual({ transform: "translateY(50px)" });
    });

    it("skips a key that vanished, and treats an element without an entry as new", () => {
        const a = row("a", 0, 10);
        const gone = row("gone", 0, 50);
        const before = capture([a, gone], keyOf);
        gone.remove();
        const fresh = row("fresh", 0, 90);
        a.place(0, 30);
        move(before, [a, fresh], keyOf, { axis: "y" });
        expect(fake.calls).toHaveLength(1);

        move(before, [a, fresh], keyOf, { axis: "y", onNew: "fade" });
        const entered = fake.calls.filter((c) => c.el === fresh);
        expect(entered).toHaveLength(1);
        expect(entered[0].keyframes[0]).toEqual({ opacity: 0 });
    });

    it("animates nothing when the policy asks for no motion for a transition, and when asked, uses its category", () => {
        const a = row("a", 0, 10);
        const before = capture([a], keyOf);
        a.place(0, 70);
        reduce.categories.add("transition");
        move(before, [a], keyOf);
        expect(fake.calls).toHaveLength(0);
        move(before, [a], keyOf, { category: "feedback" });
        expect(fake.calls).toHaveLength(1);
    });

    it("cancels a slide still running before it measures, so the new one starts from the captured place", () => {
        const a = row("a", 0, 0);
        const before = capture([a], keyOf);
        a.place(0, 100);
        move(before, [a], keyOf, { axis: "y" });
        // Mid-flight the element reports a rectangle that includes the slide's transform.
        const measured = a.getBoundingClientRect;
        a.getBoundingClientRect = () => {
            const rect = measured.call(a);
            return fake.calls[0].ended ? rect : ({ ...rect, top: rect.top - 50 } as DOMRect);
        };
        const midFlight = capture([a], keyOf);
        a.place(0, 200);
        move(midFlight, [a], keyOf, { axis: "y" });
        expect(fake.calls[0].ended).toBe(true);
        // From the visual place (50) to the layout place (200): 50 - 200.
        expect(fake.calls[1].keyframes[0]).toEqual({ transform: "translateY(-150px)" });
    });

    it("reads every rectangle before it starts any animation, so a long list forces one layout, not one per row", () => {
        const rows = Array.from({ length: 5 }, (_, i) => row(`r${i}`, 0, i * 20));
        const before = capture(rows, keyOf);
        const log: string[] = [];
        rows.forEach((r, i) => {
            r.place(0, i * 20 + 40);
            const read = r.getBoundingClientRect;
            r.getBoundingClientRect = () => (log.push("read"), read.call(r));
        });
        const animate = Element.prototype.animate;
        Element.prototype.animate = function (this: Element, ...args: Parameters<Element["animate"]>) {
            log.push("write");
            return animate.apply(this, args);
        };
        try {
            move(before, rows, keyOf, { axis: "y" });
            // Slides still running are cancelled before the measuring too.
            move(before, rows, keyOf, { axis: "y" });
        } finally {
            Element.prototype.animate = animate;
        }
        expect(log.filter((e) => e === "write")).toHaveLength(10);
        const firstWrite = log.indexOf("write");
        expect(log.slice(0, firstWrite)).toEqual(Array(5).fill("read"));
        expect(log.slice(firstWrite).filter((e) => e === "read")).toHaveLength(5);
        expect(log.slice(firstWrite, firstWrite + 5)).toEqual(Array(5).fill("write"));
    });

    it("measures nothing when the policy asks for no motion", () => {
        const a = row("a", 0, 0);
        const before = capture([a], keyOf);
        const read = vi.fn(a.getBoundingClientRect);
        a.getBoundingClientRect = read;
        reduce.categories.add("transition");
        move(before, [a], keyOf);
        expect(read).not.toHaveBeenCalled();
    });

    it("replaces a slide still running on the element", () => {
        const a = row("a", 0, 10);
        const before = capture([a], keyOf);
        a.place(0, 70);
        move(before, [a], keyOf);
        move(before, [a], keyOf);
        expect(fake.calls.map((c) => c.ended)).toEqual([true, false]);
    });
});

describe("travel", () => {
    it("flies a copy from the old rectangle to the element, hiding the element meanwhile", async () => {
        const el = at(200, 300);
        el.style.visibility = "visible";
        const handle = travel(el, rectOf(20, 30));
        const copy = fake.calls[0].el as HTMLElement;
        expect(copy).not.toBe(el);
        expect(copy.parentElement).toBe(document.body);
        expect(copy.style).toMatchObject({
            position: "fixed",
            left: "20px",
            top: "30px",
            width: "100px",
            height: "20px",
        });
        expect(el.style.visibility).toBe("hidden");
        expect(fake.calls[0].keyframes[1]).toEqual({ transform: "translate(180px, 270px)" });
        expect(fake.calls[0].options).toMatchObject({ duration: DURATION.moderate, easing: EASE.emphasized });

        fake.calls[0].finish();
        await handle.finished;
        expect(copy.isConnected).toBe(false);
        expect(el.style.visibility).toBe("visible");
    });

    it("removes the copy and shows the element again after cancel() mid-flight", () => {
        const el = at(200, 300);
        const handle = travel(el, rectOf(20, 30));
        const copy = fake.calls[0].el;
        handle.cancel();
        expect(copy.isConnected).toBe(false);
        expect(el.style.visibility).toBe("");
        expect(handle.cancelled).toBe(true);
    });

    it("keeps one flight per element: a second travel replaces the first and the element ends shown", () => {
        const el = at(200, 300);
        const first = travel(el, rectOf(20, 30));
        const second = travel(el, rectOf(40, 50));
        expect(first.cancelled).toBe(true);
        expect(el.style.visibility).toBe("hidden");
        fake.calls[1].finish();
        expect(el.style.visibility).toBe("");
        expect(second.cancelled).toBe(false);
        expect(document.body.querySelectorAll("div")).toHaveLength(1);
    });

    it("calls onArrive when the element is shown again, also with motion off, and not after a cancel", () => {
        const arrived = vi.fn();
        const el = at(200, 300);
        travel(el, rectOf(20, 30), { onArrive: arrived });
        expect(arrived).not.toHaveBeenCalled();
        fake.calls[0].finish();
        expect(arrived).toHaveBeenCalledTimes(1);
        expect(el.style.visibility).toBe("");

        const cancelled = vi.fn();
        travel(el, rectOf(20, 30), { onArrive: cancelled }).cancel();
        expect(cancelled).not.toHaveBeenCalled();

        reduce.categories.add("transition");
        const off = vi.fn();
        travel(el, rectOf(20, 30), { onArrive: off });
        expect(off).toHaveBeenCalledTimes(1);
    });

    it("lets the caller adjust the copy", () => {
        const el = at(0, 0);
        el.className = "pill pill--landing";
        travel(el, rectOf(5, 5), { onCopy: (c) => c.classList.remove("pill--landing") });
        expect((fake.calls[0].el as HTMLElement).className).toBe("pill");
    });

    it("draws no copy and leaves the element shown when motion is off", async () => {
        reduce.categories.add("transition");
        const el = at(0, 0);
        const handle = travel(el, rectOf(5, 5));
        await handle.finished;
        expect(document.body.children).toHaveLength(1);
        expect(el.style.visibility).toBe("");
    });
});

describe("mergeInto", () => {
    const flush = () => new Promise<void>((r) => setTimeout(r, 0));

    beforeEach(() => {
        vi.stubGlobal("requestAnimationFrame", (fn: () => void) => setTimeout(fn, 0) as unknown as number);
        vi.stubGlobal("cancelAnimationFrame", (id: number) => clearTimeout(id));
    });
    afterEach(() => vi.unstubAllGlobals());

    it("resolves a destination function after one frame and pairs by index; a missing destination is skipped", async () => {
        const dest = vi.fn(() => [at(200, 10), at(300, 10)]);
        const land = vi.fn();
        mergeInto([rectOf(0, 0), rectOf(0, 30), rectOf(0, 60)], dest, { land });
        expect(dest).not.toHaveBeenCalled();
        await flush();
        expect(dest).toHaveBeenCalledTimes(1);
        expect(fake.calls).toHaveLength(2);
        expect(fake.calls[0].keyframes[1]).toEqual({ transform: "translate(200px, 10px)" });
        expect(fake.calls[1].keyframes[1]).toEqual({ transform: "translate(300px, -20px)" });
    });

    it("hides each real element until its copy lands, then calls land with it and its index", async () => {
        const els = [at(200, 10), at(300, 10)];
        const land = vi.fn();
        const handle = mergeInto([rectOf(0, 0), rectOf(0, 30)], els, { land });
        expect(els.map((e) => e.style.visibility)).toEqual(["hidden", "hidden"]);
        expect(land).not.toHaveBeenCalled();
        fake.calls[1].finish();
        await flush();
        expect(land).toHaveBeenCalledTimes(1);
        expect(land).toHaveBeenCalledWith(els[1], 1);
        expect(els[1].style.visibility).toBe("");
        expect(els[0].style.visibility).toBe("hidden");
        fake.calls[0].finish();
        await handle.finished;
        expect(land.mock.calls).toEqual([
            [els[1], 1],
            [els[0], 0],
        ]);
    });

    it("lets the caller adjust each copy, and skips a hole in the destinations", () => {
        const els = [undefined, at(300, 10)];
        els[1]!.className = "pill pill--landing";
        mergeInto([rectOf(0, 0), rectOf(0, 30)], els, {
            onCopy: (copy) => copy.classList.remove("pill--landing"),
        });
        expect(fake.calls).toHaveLength(1);
        expect((fake.calls[0].el as HTMLElement).className).toBe("pill");
    });

    it("staggers the copies by index", () => {
        mergeInto([rectOf(0, 0), rectOf(0, 30), rectOf(0, 60)], [at(1, 1), at(1, 1), at(1, 1)], { stagger: 50 });
        expect(fake.calls.map((c) => c.options.delay)).toEqual([0, 50, 100]);
    });

    it("pulses each arrived piece when no land hook is given, and not when one is", async () => {
        const els = [at(200, 10)];
        mergeInto([rectOf(0, 0)], els);
        fake.calls[0].finish();
        await flush();
        expect(fake.calls).toHaveLength(2);
        expect(fake.calls[1].el).toBe(els[0]);
        expect(fake.calls[1].options.duration).toBe(DURATION.slow);

        fake.calls.length = 0;
        mergeInto([rectOf(0, 0)], [at(200, 10)], { land: () => {} });
        fake.calls[0].finish();
        await flush();
        expect(fake.calls).toHaveLength(1);
    });

    it("cancel() mid-flight removes every copy, shows every element, and lands nothing", async () => {
        const els = [at(200, 10), at(300, 10)];
        const land = vi.fn();
        const handle = mergeInto([rectOf(0, 0), rectOf(0, 30)], els, { land });
        const copies = fake.calls.map((c) => c.el);
        handle.cancel();
        await handle.finished;
        await flush();
        expect(copies.every((c) => !c.isConnected)).toBe(true);
        expect(els.map((e) => e.style.visibility)).toEqual(["", ""]);
        expect(land).not.toHaveBeenCalled();
        expect(handle.cancelled).toBe(true);
    });

    it("is not finished until the landing pulses end, and cancel() stops a pulse that has started", async () => {
        const el = at(200, 10);
        const handle = mergeInto([rectOf(0, 0)], [el]);
        let done = false;
        void handle.finished.then(() => (done = true));
        fake.calls[0].finish();
        await flush();
        const beat = fake.calls[1];
        expect(beat.el).toBe(el);
        expect(done).toBe(false);
        handle.cancel();
        expect(beat.ended).toBe(true);
        await handle.finished;

        const second = mergeInto([rectOf(0, 0)], [at(200, 10)]);
        fake.calls[2].finish();
        await flush();
        fake.calls[3].finish();
        await second.finished;
    });

    it("cancel() before the frame never resolves the destination", async () => {
        const dest = vi.fn(() => [at(1, 1)]);
        const handle = mergeInto([rectOf(0, 0)], dest);
        handle.cancel();
        await flush();
        expect(dest).not.toHaveBeenCalled();
        expect(fake.calls).toHaveLength(0);
    });

    it("flies nothing but still lands every piece when motion is off", async () => {
        reduce.categories.add("transition");
        const els = [at(200, 10), at(300, 10)];
        const land = vi.fn();
        const handle = mergeInto([rectOf(0, 0), rectOf(0, 30)], els, { land });
        await handle.finished;
        expect(fake.calls).toHaveLength(0);
        expect(document.body.children).toHaveLength(2);
        expect(land.mock.calls).toEqual([
            [els[0], 0],
            [els[1], 1],
        ]);
    });
});

describe("enter and exit", () => {
    it("exit resolves finished at once with motion off, at the end of the animation with motion on", async () => {
        const el = div();
        let removed = false;
        const handle = exit(el, "fadeScale");
        void handle.finished.then(() => (removed = true));
        await Promise.resolve();
        expect(removed).toBe(false);
        expect(fake.calls[0].keyframes[1]).toEqual({ opacity: 0, transform: "scale(0.96)" });
        expect(fake.calls[0].options.fill).toBe("forwards");
        fake.calls[0].finish();
        await handle.finished;
        expect(removed).toBe(true);

        reduce.categories.add("transition");
        const off = exit(div());
        await off.finished;
        expect(fake.calls).toHaveLength(1);
    });

    it("enter plays from the kind's start state to shown", () => {
        enter(div(), "slideY");
        expect(fake.calls[0].keyframes).toEqual([
            { opacity: 0, transform: "translateY(8px)" },
            { opacity: 1, transform: "none" },
        ]);
        expect(fake.calls[0].options).toMatchObject({ duration: DURATION.fast, easing: EASE.out });
    });
});

describe("pulse and glide", () => {
    it("pulse is one beat in the feedback category, and a new pulse replaces a running one", () => {
        const el = div();
        const a = pulse(el, "bounce");
        pulse(el, "flash");
        expect(a.cancelled).toBe(true);
        expect(fake.calls[0].options.duration).toBe(DURATION.slow);
        expect(fake.calls[1].options.duration).toBe(DURATION.moderate);
        reduce.categories.add("feedback");
        pulse(el, "glow");
        expect(fake.calls).toHaveLength(2);
    });

    it("glide adds to other motion, and overlapping glides sum instead of replacing", () => {
        const el = div();
        const a = glide(el, 30);
        const b = glide(el, -10, { cleanup: () => {} });
        expect(a.cancelled || b.cancelled).toBe(false);
        expect(fake.calls[0].options.composite).toBe("add");
        expect(fake.calls[0].keyframes).toEqual([{ transform: "translateY(30px)" }, { transform: "translateY(0)" }]);
        expect(fake.calls[0].options).toMatchObject({ duration: DURATION.moderate, easing: EASE.decelerate });
        groupHandle([a, b]).cancel();
        expect(fake.calls.every((c) => c.ended)).toBe(true);
    });
});

describe("groupHandle and the presets", () => {
    it("a group is finished when every member is, and cancelled when one is", async () => {
        const a = play(div(), [], opts);
        const b = play(div(), [], opts);
        const group = groupHandle([a, b, finishedHandle()]);
        fake.calls[0].finish();
        let done = false;
        void group.finished.then(() => (done = true));
        await Promise.resolve();
        expect(done).toBe(false);
        b.cancel();
        await group.finished;
        expect(group.cancelled).toBe(true);
    });

    it("every preset names a duration and an easing that exist", () => {
        for (const feel of Object.values(MOTION)) {
            expect(DURATION).toHaveProperty(feel.duration);
            expect(EASE).toHaveProperty(feel.easing);
        }
    });
});
