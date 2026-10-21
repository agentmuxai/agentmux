// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { Emitter } from "./emitter";

afterEach(() => vi.useRealTimers());

describe("Emitter (the part of rxjs's Subject the app used)", () => {
    it("delivers each value to every subscriber, until it unsubscribes", () => {
        const e = new Emitter<number>();
        const a: number[] = [];
        const b: number[] = [];
        const subA = e.subscribe((v) => a.push(v));
        e.subscribe((v) => b.push(v));
        e.next(1);
        subA.unsubscribe();
        e.next(2);
        expect(a).toEqual([1]);
        expect(b).toEqual([1, 2]);
    });

    it("counts the same function subscribed twice as two subscriptions", () => {
        const e = new Emitter<number>();
        const fn = vi.fn();
        const first = e.subscribe(fn);
        e.subscribe(fn);
        e.next(1);
        first.unsubscribe();
        e.next(2);
        expect(fn).toHaveBeenCalledTimes(3);
    });

    it("drops everyone on complete and ignores later values and subscribers", () => {
        const e = new Emitter<number>();
        const fn = vi.fn();
        e.subscribe(fn);
        e.complete();
        e.next(1);
        e.subscribe(fn).unsubscribe();
        e.next(2);
        expect(fn).not.toHaveBeenCalled();
        expect(e.observed).toBe(false);
    });

    it("keeps delivering when one listener throws, and rethrows that error on its own task", () => {
        vi.useFakeTimers();
        const e = new Emitter<number>();
        const after = vi.fn();
        e.subscribe(() => {
            throw new Error("boom");
        });
        e.subscribe(after);
        e.next(1);
        expect(after).toHaveBeenCalledWith(1);
        expect(() => vi.runAllTimers()).toThrow("boom");
    });
});
