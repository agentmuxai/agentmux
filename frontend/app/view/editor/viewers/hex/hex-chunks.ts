// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The hex viewer's bytes: a file read 64 KB at a time, the parts kept least
 * recently used first out, so a file of any size is shown without being
 * loaded. A part is asked for once however many rows wait for it.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

import { fetchMediaRange } from "@/app/element/local-media";

/** Bytes per part. */
export const CHUNK = 65536;

/** Parts kept: 4 MB of a file at most. */
export const CHUNKS_KEPT = 64;

/** Reads bytes `start..=end` of the file. */
export type ReadRange = (start: number, end: number) => Promise<Uint8Array>;

/** Reads a file on disk through srv, a part at a time. */
export function rangeReaderOf(path: string, blockId?: string): ReadRange {
    return async (start, end) => {
        const r = await fetchMediaRange(path, start, end, { blockId });
        // No blob: srv sent the whole file instead of the part asked for.
        if (!r.blob) throw new Error("the file can't be read in parts");
        return new Uint8Array(await r.blob.arrayBuffer());
    };
}

/** Reads dropped bytes that have no file on disk. */
export function fileReaderOf(file: File): ReadRange {
    return async (start, end) => new Uint8Array(await file.slice(start, end + 1).arrayBuffer());
}

export class ChunkCache {
    // Insertion order is use order: the first key is the least recently used.
    private readonly kept = new Map<number, Uint8Array>();
    private readonly loading = new Map<number, Promise<Uint8Array>>();

    constructor(
        private readonly read: ReadRange,
        private readonly size: number,
        private readonly keep = CHUNKS_KEPT
    ) {}

    /** The byte at `offset`, when its part is here. */
    byteAt(offset: number): number | undefined {
        const index = Math.floor(offset / CHUNK);
        return this.kept.get(index)?.[offset - index * CHUNK];
    }

    /** Whether every byte of `start..=end` is here. */
    has(start: number, end: number): boolean {
        for (let i = Math.floor(start / CHUNK); i <= Math.floor(end / CHUNK); i++) if (!this.kept.has(i)) return false;
        return true;
    }

    /** `start..=end` is in use: its parts are the last to go. */
    touch(start: number, end: number): void {
        for (let i = Math.floor(start / CHUNK); i <= Math.floor(end / CHUNK); i++) {
            const chunk = this.kept.get(i);
            if (!chunk) continue;
            this.kept.delete(i);
            this.kept.set(i, chunk);
        }
    }

    /** Read the parts holding `start..=end` that aren't here yet. */
    async ensure(start: number, end: number): Promise<void> {
        const last = Math.min(end, this.size - 1);
        if (this.size <= 0 || start > last) return;
        const waits: Promise<unknown>[] = [];
        for (let i = Math.floor(start / CHUNK); i <= Math.floor(last / CHUNK); i++) if (!this.kept.has(i)) waits.push(this.load(i));
        await Promise.all(waits);
    }

    /** Bytes `start..=end`, read as needed. Not kept: a copy can be larger than the cache. */
    async bytes(start: number, end: number): Promise<Uint8Array> {
        const last = Math.min(end, this.size - 1);
        if (this.size <= 0 || start > last) return new Uint8Array(0);
        if (this.has(start, last)) {
            const out = new Uint8Array(last - start + 1);
            for (let i = Math.floor(start / CHUNK); i <= Math.floor(last / CHUNK); i++) {
                const chunk = this.kept.get(i)!;
                const base = i * CHUNK;
                const from = Math.max(start, base);
                const to = Math.min(last, base + chunk.length - 1);
                out.set(chunk.subarray(from - base, to - base + 1), from - start);
            }
            return out;
        }
        return this.read(start, last);
    }

    private load(index: number): Promise<Uint8Array> {
        const running = this.loading.get(index);
        if (running) return running;
        const start = index * CHUNK;
        const p = this.read(start, Math.min(start + CHUNK, this.size) - 1)
            .then((chunk) => {
                this.kept.set(index, chunk);
                while (this.kept.size > this.keep) this.kept.delete(this.kept.keys().next().value as number);
                return chunk;
            })
            .finally(() => this.loading.delete(index));
        this.loading.set(index, p);
        return p;
    }
}
