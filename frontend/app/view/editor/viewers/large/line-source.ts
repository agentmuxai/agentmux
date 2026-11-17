// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Large text viewer's lines: a file read a page of lines at a time
 * (srv's `fileindexlines` says which bytes hold them, a range read fetches
 * them), the pages kept least recently used first out, so a file of any size
 * is shown without being loaded. A line over about 10,000 characters is cut
 * short, with a note of what is hidden.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import type { FileIndexLinesResult } from "@/types/rpc/FileIndexLinesResult";
import { formatBytes } from "@/util/format-bytes";
import type { ReadRange } from "../hex/hex-chunks";

/** Lines per page. */
export const PAGE_LINES = 200;

/** Pages kept: 8,000 lines. */
export const PAGES_KEPT = 40;

/** Characters of a line shown before it is cut. */
export const LINE_SHOWN_MAX = 10_000;

/** A page of more bytes than this is read a line at a time, each line's start only. */
export const PAGE_BYTES_MAX = 4 * 1024 * 1024;

/** Bytes read of a line in a page that is read a line at a time: room for LINE_SHOWN_MAX characters of any width. */
export const LINE_READ_BYTES = 40_000;

/** Which lines hold which bytes: `fileindexlines`. */
export type IndexLines = (from: number, count: number) => Promise<FileIndexLinesResult>;

/** A line as shown. */
export interface LineText {
    text: string;
    /** What the cut-off part is, when the line was cut ("+12,345 characters"). */
    hidden?: string;
}

/** The label a TextDecoder knows for the probe's encoding. */
export function decoderLabel(encoding: string | undefined): string {
    if (encoding === "utf-16le" || encoding === "utf-16be") return encoding;
    // Not UTF-8: the read path's detector decides; the common case is a Windows code page.
    return encoding === "legacy" ? "windows-1252" : "utf-8";
}

/** `text` cut to what is shown. */
function cut(text: string): LineText {
    if (text.length <= LINE_SHOWN_MAX) return { text };
    return { text: text.slice(0, LINE_SHOWN_MAX), hidden: `+${(text.length - LINE_SHOWN_MAX).toLocaleString("en-US")} characters` };
}

/** The lines in `bytes`, which run from a line's start to just after a line break (or the file's end). */
export function linesOfBytes(bytes: Uint8Array, encoding: string | undefined): LineText[] {
    const text = new TextDecoder(decoderLabel(encoding)).decode(bytes);
    const lines = text.split("\n");
    // The break after the last line leaves nothing after it.
    if (lines[lines.length - 1] === "") lines.pop();
    return lines.map((l) => cut(l.endsWith("\r") ? l.slice(0, -1) : l));
}

export class LineSource {
    /** The file's lines, once srv has said; -1 before. */
    total = -1;
    private stamp = "";
    // Insertion order is use order: the first key is the least recently used.
    private readonly pages = new Map<number, LineText[]>();
    private readonly loading = new Map<number, Promise<void>>();

    constructor(
        private readonly index: IndexLines,
        private readonly read: ReadRange,
        private readonly encoding: string | undefined,
        private readonly keep = PAGES_KEPT
    ) {}

    /** Line `n` (0-based), when its page is here. */
    lineAt(n: number): LineText | undefined {
        return this.pages.get(Math.floor(n / PAGE_LINES))?.[n % PAGE_LINES];
    }

    /** Whether every line of `first..=last` that the file has is here. */
    has(first: number, last: number): boolean {
        for (let p = Math.floor(first / PAGE_LINES); p <= Math.floor(last / PAGE_LINES); p++) {
            if (this.total >= 0 && p * PAGE_LINES >= this.total) break;
            if (!this.pages.has(p)) return false;
        }
        return true;
    }

    /** Lines `first..=last` are in use: their pages are the last to go. */
    touch(first: number, last: number): void {
        for (let p = Math.floor(first / PAGE_LINES); p <= Math.floor(last / PAGE_LINES); p++) {
            const page = this.pages.get(p);
            if (!page) continue;
            this.pages.delete(p);
            this.pages.set(p, page);
        }
    }

    /** Read the pages holding `first..=last` that aren't here yet. */
    async ensure(first: number, last: number): Promise<void> {
        const waits: Promise<void>[] = [];
        for (let p = Math.floor(first / PAGE_LINES); p <= Math.floor(last / PAGE_LINES); p++) {
            if (this.total >= 0 && p * PAGE_LINES >= this.total) break;
            if (!this.pages.has(p)) waits.push(this.load(p));
        }
        await Promise.all(waits);
    }

    private load(page: number): Promise<void> {
        const running = this.loading.get(page);
        if (running) return running;
        const p = this.fetchPage(page)
            .then((lines) => {
                this.pages.set(page, lines);
                while (this.pages.size > this.keep) this.pages.delete(this.pages.keys().next().value as number);
            })
            .finally(() => this.loading.delete(page));
        this.loading.set(page, p);
        return p;
    }

    /** A line index that is another file's (it changed while it was shown): what was read of the old one goes. */
    private note(r: FileIndexLinesResult): void {
        const stamp = `${r.size}:${r.mtime}`;
        if (this.stamp && this.stamp !== stamp) this.pages.clear();
        this.stamp = stamp;
        this.total = r.total_lines;
    }

    private async fetchPage(page: number): Promise<LineText[]> {
        const from = page * PAGE_LINES;
        const r = await this.index(from, PAGE_LINES);
        this.note(r);
        if (r.end <= r.start) return [];
        if (r.end - r.start <= PAGE_BYTES_MAX) return linesOfBytes(await this.read(r.start, r.end - 1), this.encoding);
        // A page with a line this long: each line's start, and how much more there is.
        const lines: LineText[] = [];
        for (let i = 0; i < Math.min(PAGE_LINES, this.total - from); i++) lines.push(await this.longLine(from + i));
        return lines;
    }

    private async longLine(n: number): Promise<LineText> {
        const r = await this.index(n, 1);
        const length = r.end - r.start;
        if (length <= 0) return { text: "" };
        const got = Math.min(length, LINE_READ_BYTES);
        const bytes = await this.read(r.start, r.start + got - 1);
        const text = new TextDecoder(decoderLabel(this.encoding)).decode(bytes).replace(/\r?\n$/, "");
        // What isn't read is known in bytes only; what was read and not shown, in characters (about as many bytes).
        const hidden = length - got + Math.max(0, text.length - LINE_SHOWN_MAX);
        return hidden > 0 ? { text: text.slice(0, LINE_SHOWN_MAX), hidden: `+${formatBytes(hidden)}` } : { text };
    }
}
