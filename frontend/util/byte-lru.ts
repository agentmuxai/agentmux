// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A Map-backed LRU bounded by an estimate of the bytes it holds, not by its
 * entry count: a cache keyed by a whole file's text can hold 200 entries of
 * 200 KB each when it is bounded by count alone.
 * SPEC_AGENT_PANE_TOOL_OUTPUT_RETENTION_2026_10_13.md §2.8.
 */
export class ByteLru<V> {
    private readonly entries = new Map<string, { value: V; bytes: number }>();
    private total = 0;

    constructor(
        private readonly maxBytes: number,
        private readonly sizeOf: (key: string, value: V) => number
    ) {}

    get size(): number {
        return this.entries.size;
    }

    get bytes(): number {
        return this.total;
    }

    get(key: string): V | undefined {
        const hit = this.entries.get(key);
        if (!hit) return undefined;
        // Most recently used goes last.
        this.entries.delete(key);
        this.entries.set(key, hit);
        return hit.value;
    }

    has(key: string): boolean {
        return this.entries.has(key);
    }

    set(key: string, value: V): void {
        const old = this.entries.get(key);
        if (old) {
            this.total -= old.bytes;
            this.entries.delete(key);
        }
        const bytes = this.sizeOf(key, value);
        // An entry bigger than the whole budget isn't kept.
        if (bytes > this.maxBytes) return;
        this.entries.set(key, { value, bytes });
        this.total += bytes;
        while (this.total > this.maxBytes) {
            const oldest = this.entries.keys().next().value as string;
            this.total -= this.entries.get(oldest)!.bytes;
            this.entries.delete(oldest);
        }
    }

    clear(): void {
        this.entries.clear();
        this.total = 0;
    }
}
