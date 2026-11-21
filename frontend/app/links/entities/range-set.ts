// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sorted, non-overlapping ranges with a value each, searched by bisection, so protecting a text and choosing among
 * candidates cost O(n log n) however many names or code spans a long message holds.
 */

import type { Range } from "./types";

type Item<T> = Range & { value: T };

export class RangeSet<T> {
    private items: Item<T>[] = [];

    /** The union of `ranges`: overlapping and touching ranges merge into one. */
    static union(ranges: Range[]): RangeSet<true> {
        const set = new RangeSet<true>();
        const sorted = [...ranges].sort((a, b) => a.start - b.start || a.end - b.end);
        for (const r of sorted) {
            const last = set.items[set.items.length - 1];
            if (last && r.start <= last.end) last.end = Math.max(last.end, r.end);
            else set.items.push({ start: r.start, end: r.end, value: true });
        }
        return set;
    }

    /** The first item index whose end is after `at`. */
    private firstEndingAfter(at: number): number {
        let lo = 0;
        let hi = this.items.length;
        while (lo < hi) {
            const mid = (lo + hi) >>> 1;
            if (this.items[mid].end > at) hi = mid;
            else lo = mid + 1;
        }
        return lo;
    }

    /** The value of an item that overlaps `range`, if any. */
    find(range: Range): T | undefined {
        const item = this.items[this.firstEndingAfter(range.start)];
        return item && item.start < range.end ? item.value : undefined;
    }

    /** Adds a range that overlaps no item (the caller has checked with `find`). */
    add(range: Range, value: T): void {
        this.items.splice(this.firstEndingAfter(range.start), 0, { start: range.start, end: range.end, value });
    }
}
