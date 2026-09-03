// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * resize-contract.ts — step 2 of SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md.
 *
 * No component under test here — this file has no call sites yet (that's
 * step 3). These tests exercise the two entry points directly against a
 * plain DOM element, mirroring the FLIP-mechanics assertions
 * ToolOverlayLog.test.tsx already makes for the (soon to be migrated)
 * `flipHeight()` this replaces, plus the policy layer that function didn't
 * have to reason about in isolation: reduced motion, unmeasurable
 * (content-visibility/zero-size) elements, the magnitude cap, and
 * per-element cancellation.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let reducedMotion = false;
vi.mock("@/app/store/global", () => ({
    atoms: {
        prefersReducedMotionAtom: () => reducedMotion,
    },
}));

import { beginHeightContinuity, withHeightContinuity } from "./resize-contract";

/** Elements whose computed content-visibility should read "hidden". The
 *  module under test only ever reads THIS one property off
 *  getComputedStyle's return value, so replacing the global wholesale for
 *  this file is safe and far simpler than partially mocking jsdom's CSSOM
 *  (which doesn't recognize content-visibility as a settable inline style
 *  property at all). */
const hiddenEls = new Set<Element>();
function setHidden(el: Element, hidden: boolean): void {
    if (hidden) hiddenEls.add(el);
    else hiddenEls.delete(el);
}

function setOffset(el: HTMLElement, height: number, width = 100): void {
    Object.defineProperty(el, "offsetHeight", { configurable: true, value: height });
    Object.defineProperty(el, "offsetWidth", { configurable: true, value: width });
}

// Real ids + real removal-on-cancel — NOT a no-op cancelAnimationFrame. A
// no-op cancel would let this suite's cancellation test pass even with
// cancellation itself deleted from the source: both the stale and the
// fresh callback would still fire (in queue order), and since the fresh
// one is queued second, it "wins" the final `el.style.height` value either
// way — the bug is only observable by checking how many frames were
// actually PENDING before the flush, which needs real bookkeeping here.
let rafQueue: Map<number, FrameRequestCallback> = new Map();
let nextRafId = 0;
function flushRaf(): void {
    const pending = [...rafQueue.values()];
    rafQueue.clear();
    for (const cb of pending) cb(0);
}

function endTransition(el: HTMLElement): void {
    el.dispatchEvent(new (globalThis as any).TransitionEvent("transitionend", { propertyName: "height" }));
}

beforeEach(() => {
    hiddenEls.clear();
    rafQueue.clear();
    nextRafId = 0;
    reducedMotion = false;
    vi.stubGlobal("getComputedStyle", (el: Element) => ({
        contentVisibility: hiddenEls.has(el) ? "hidden" : "visible",
    }) as CSSStyleDeclaration);
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
        const id = ++nextRafId;
        rafQueue.set(id, cb);
        return id;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => {
        rafQueue.delete(id);
    });
});

afterEach(() => {
    vi.unstubAllGlobals();
});

describe("withHeightContinuity", () => {
    it("freezes at the old height synchronously, then eases to the new height next frame", () => {
        const el = document.createElement("div");
        setOffset(el, 40);
        withHeightContinuity(el, () => setOffset(el, 120));

        // Synchronous: frozen at the FROM height, transition already armed —
        // must happen before the new height is assigned, or there is nothing
        // for the browser to ease.
        expect(el.style.height).toBe("40px");
        expect(el.style.transition).toContain("height");

        flushRaf();
        expect(el.style.height).toBe("120px");

        endTransition(el);
        expect(el.style.height).toBe("");
        expect(el.style.transition).toBe("");
        expect(el.style.overflowY).toBe("");
    });

    it("shrinking works the same way as growing", () => {
        const el = document.createElement("div");
        setOffset(el, 300);
        withHeightContinuity(el, () => setOffset(el, 80));
        expect(el.style.height).toBe("300px");
        flushRaf();
        expect(el.style.height).toBe("80px");
    });

    it("does not animate when the height doesn't change", () => {
        const el = document.createElement("div");
        setOffset(el, 60);
        withHeightContinuity(el, () => setOffset(el, 60));
        expect(el.style.height).toBe("");
    });

    it("does not animate under reduced motion, but the mutation still runs", () => {
        reducedMotion = true;
        const el = document.createElement("div");
        setOffset(el, 40);
        let mutated = false;
        withHeightContinuity(el, () => {
            mutated = true;
            setOffset(el, 120);
        });
        expect(mutated).toBe(true);
        expect(el.style.height).toBe("");
    });

    it("skips measurement entirely for a content-visibility:hidden element, even though its height genuinely changes", () => {
        // mutate() must actually change the height here — a no-op mutate
        // can't distinguish "correctly skipped" from "measured, but nothing
        // changed so there was nothing to animate anyway." A prior version
        // of this test used a no-op mutate and stayed green even with the
        // content-visibility check deleted from the source.
        const el = document.createElement("div");
        setHidden(el, true);
        setOffset(el, 999); // would be a huge FLIP if this were read as "from"
        let mutated = false;
        withHeightContinuity(el, () => {
            mutated = true;
            setOffset(el, 40); // real change — proves isMeasurable(), not shouldAnimate(), is what gated this
        });
        expect(mutated).toBe(true);
        expect(el.style.height).toBe(""); // never touched
    });

    it("does not FLIP from a zero (display:none-equivalent) starting height", () => {
        // Covered by shouldAnimate's own `fromPx <= 0` guard, not a
        // dedicated isMeasurable branch — see that function's doc comment
        // for why a separate zero-size check turned out to be redundant.
        const el = document.createElement("div");
        setOffset(el, 0, 0);
        withHeightContinuity(el, () => setOffset(el, 200));
        expect(el.style.height).toBe(""); // 0 -> 200 is a first appearance, not a shrink to ease
    });

    it("does not animate a delta past the magnitude cap — jumps straight to the mutation", () => {
        const el = document.createElement("div");
        setOffset(el, 40000);
        withHeightContinuity(el, () => setOffset(el, 100));
        expect(el.style.height).toBe(""); // no freeze attempted
    });

    it("cancels a still-in-flight transition's PENDING FRAME on the same element when a second mutation lands first", () => {
        // The decisive check is the queue size, not the final height value:
        // with the stale first flip's rAF callback still pending, it would
        // ALSO fire on the next flush (in queue order, before the second
        // flip's callback) and happen to still leave the same final value
        // (the second one, queued later, overwrites it) — final state alone
        // can't tell a genuinely cancelled first flip apart from one that
        // just happened to lose a race. Only checking how many frames were
        // actually pending distinguishes them.
        const el = document.createElement("div");
        setOffset(el, 100);
        withHeightContinuity(el, () => setOffset(el, 300)); // first FLIP starts, one frame queued
        expect(rafQueue.size).toBe(1);

        withHeightContinuity(el, () => setOffset(el, 50)); // must cancel the first flip's frame
        expect(rafQueue.size).toBe(1); // NOT 2 — the stale one was removed, not left stacked

        flushRaf();
        expect(el.style.height).toBe("50px");
        endTransition(el);
        expect(el.style.height).toBe("");
    });

    it("two independent elements animate without interfering with each other", () => {
        const a = document.createElement("div");
        const b = document.createElement("div");
        setOffset(a, 40);
        setOffset(b, 200);
        withHeightContinuity(a, () => setOffset(a, 80));
        withHeightContinuity(b, () => setOffset(b, 20));
        expect(a.style.height).toBe("40px");
        expect(b.style.height).toBe("200px");
        flushRaf();
        expect(a.style.height).toBe("80px");
        expect(b.style.height).toBe("20px");
    });
});

describe("beginHeightContinuity", () => {
    it("captures the height now and eases on commit, once the mutation has landed", () => {
        const el = document.createElement("div");
        setOffset(el, 40);
        const commit = beginHeightContinuity(el);

        setOffset(el, 120); // the deferred mutation "lands" independently
        expect(el.style.height).toBe(""); // not yet — commit() hasn't run

        commit();
        expect(el.style.height).toBe("40px"); // frozen at the height captured at begin() time
        flushRaf();
        expect(el.style.height).toBe("120px");
    });

    it("commit is a no-op if the element is unmeasurable by the time it's called", () => {
        // offsetHeight is deliberately left non-zero (200) at commit time —
        // if the commit-time isMeasurable() check were missing, that alone
        // would be enough to make shouldAnimate's own fromPx<=0 guard NOT
        // catch this case, so the height-still-empty assertion below only
        // holds if the commit-time check is actually doing its job.
        const el = document.createElement("div");
        setOffset(el, 40);
        const commit = beginHeightContinuity(el);
        setHidden(el, true);
        setOffset(el, 200);
        commit();
        expect(el.style.height).toBe("");
    });

    it("returns an inert no-op immediately when the element is already unmeasurable at capture time", () => {
        // el has a genuine non-zero height (300) while hidden — if
        // beginHeightContinuity captured it anyway (missing the
        // capture-time check), that non-zero fromPx would survive into
        // commit() and produce a real FLIP once made visible, rather than
        // being silently blocked by shouldAnimate's fromPx<=0 guard the way
        // an accidentally-still-zero offset would.
        const el = document.createElement("div");
        setHidden(el, true);
        setOffset(el, 300);
        const commit = beginHeightContinuity(el);
        setHidden(el, false);
        setOffset(el, 500);
        commit(); // no "from" was ever captured — must do nothing, not FLIP 300 -> 500
        expect(el.style.height).toBe("");
    });

    it("does not animate if the height at commit time is unchanged", () => {
        const el = document.createElement("div");
        setOffset(el, 60);
        const commit = beginHeightContinuity(el);
        commit();
        expect(el.style.height).toBe("");
    });
});
