// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Large text viewer's state: the file's lines (read a page at a time),
 * the lines on screen, go to a line, and what the keys do. Read-only: nothing
 * here writes the file.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { formatBytes } from "@/util/format-bytes";
import { createComputed, createEffect, createMemo, createSignal, untrack } from "solid-js";
import type { ViewerBarProps } from "../viewer-bar";
import type { FileProbe } from "../viewer-choice";
import { rangeReaderOf } from "../hex/hex-chunks";
import { LineSource, type LineText } from "./line-source";
import { createLargeFind } from "./large-find-state";
import { createLineScroll } from "./line-scroll";

export interface LargeTextProps {
    /** The file on disk. */
    path: string;
    /** Bumped when the file changed in place: read it again. */
    revision: number;
    /** srv's probe of the file: its size and text encoding ("utf-8", "utf-16le", "utf-16be", "legacy"). */
    probe?: FileProbe | null;
    blockId?: string;
    bar: Omit<ViewerBarProps, "start" | "facts">;
}

/** A cold index of a file of many gigabytes reads all of it: more than the default five seconds. */
const INDEX_TIMEOUT_MS = 600_000;

const REMEMBERED_MAX = 50;
const remembered = new Map<string, number>();

function remember(key: string, line: number): void {
    remembered.delete(key);
    remembered.set(key, line);
    while (remembered.size > REMEMBERED_MAX) remembered.delete(remembered.keys().next().value as string);
}

/** For tests: forget where every tab was. */
export function forgetLargeTextMemory(): void {
    remembered.clear();
}

/** What was typed in the line box: a line number from 1 to `total`; null when it isn't one. */
export function parseLine(text: string, total: number): number | null {
    const t = text.trim().replace(/[\s_,]/g, "");
    if (!/^[0-9]+$/.test(t)) return null;
    const n = Number.parseInt(t, 10);
    return Number.isSafeInteger(n) && n >= 1 && n <= total ? n : null;
}

/** The file's lines, a page at a time; a page that arrives redraws the lines. */
function createLineFeed(props: LargeTextProps) {
    const [source, setSource] = createSignal<LineSource | null>(null);
    const [total, setTotal] = createSignal(-1);
    const [loaded, setLoaded] = createSignal(0);
    const [error, setError] = createSignal("");

    let was = "";
    createComputed(() => {
        void props.revision;
        const { path, blockId } = props;
        const encoding = props.probe?.text?.encoding;
        const index = (from: number, count: number) =>
            RpcApi.FileIndexLinesCommand(TabRpcClient, { path, encoding, from_line: from, count, line_starts: true }, { timeout: INDEX_TIMEOUT_MS });
        // The same file changed in place: the lines it had stay until the new index says, so the reader keeps its place.
        if (path !== was) setTotal(-1);
        was = path;
        setError("");
        setSource(path ? new LineSource(index, rangeReaderOf(path, blockId), encoding) : null);
        setLoaded((v) => v + 1);
    });

    /** Read what `first..=last` needs, unless it is here. */
    const want = (first: number, last: number): void => {
        const s = untrack(source);
        if (!s) return;
        if (s.has(first, last)) return s.touch(first, last);
        void s.ensure(first, last).then(
            () => {
                if (untrack(source) !== s) return;
                setError("");
                setTotal(s.total);
                setLoaded((v) => v + 1);
            },
            (e) => {
                if (untrack(source) === s) setError(`Couldn't read the file: ${e instanceof Error ? e.message : String(e)}`);
            }
        );
    };
    const lineAt = (n: number): LineText | undefined => {
        void loaded();
        return source()?.lineAt(n);
    };
    const retry = () => {
        setError("");
        setLoaded((v) => v + 1);
    };
    /** The 0-based line that holds a byte of the file. */
    const lineOf = async (offset: number): Promise<number> => {
        const { path } = props;
        const encoding = props.probe?.text?.encoding;
        const r = await RpcApi.FileIndexLinesCommand(
            TabRpcClient,
            { path, encoding, from_line: 0, count: 0, at_offset: offset },
            { timeout: INDEX_TIMEOUT_MS }
        );
        return r.line_at ?? 0;
    };
    return { source, total, loaded, error, want, lineAt, retry, lineOf };
}

/** What a key does in the lines: the box or the find strip to go to, the first line to scroll to, or null for nothing. */
export function keyTarget(key: string, mod: boolean, at: { first: number; fits: number; total: number }): "box" | "find" | number | null {
    if (mod && key === "g") return "box";
    if (mod && key === "f") return "find";
    if (mod && key !== "Home" && key !== "End") return null;
    const page = Math.max(1, at.fits - 1);
    switch (key) {
        case "ArrowUp":
            return at.first - 1;
        case "ArrowDown":
            return at.first + 1;
        case "PageUp":
            return at.first - page;
        case "PageDown":
            return at.first + page;
        case "Home":
            return 0;
        case "End":
            return at.total;
        default:
            return null;
    }
}

export function createLargeText(props: LargeTextProps) {
    const feed = createLineFeed(props);
    const lines = createMemo(() => Math.max(0, feed.total()));
    const scroll = createLineScroll(lines, remembered.get(props.path) ?? 0);
    const [mark, setMark] = createSignal<number | null>(null);
    let lineBox: HTMLInputElement | undefined;
    const [badLine, setBadLine] = createSignal(false);

    // The lines on screen are read (and the page after them, so a slow scroll doesn't wait), and kept in mind.
    createEffect(() => {
        void feed.source();
        void feed.loaded();
        const first = scroll.first();
        feed.want(first, first + scroll.fits() * 2);
    });
    createEffect(() => remember(props.path, scroll.first()));

    const goTo = (line: number): void => {
        setMark(line);
        if (line - 1 < scroll.first() || line - 1 > scroll.first() + scroll.fits() - 1) scroll.showLine(line - 3);
    };
    const goToTyped = (text: string): void => {
        const n = parseLine(text, lines());
        setBadLine(n == null);
        if (n != null) {
            goTo(n);
            scroll.focus();
        }
    };
    const find = createLargeFind({
        path: () => props.path,
        size: () => props.probe?.size ?? 0,
        encoding: () => props.probe?.text?.encoding,
        revision: () => props.revision,
        topOffset: async () => (await untrack(feed.source)?.startOf(scroll.first())) ?? 0,
        lineOf: feed.lineOf,
        show: goTo,
        focusGrid: scroll.focus,
    });
    const onKeyDown = (e: KeyboardEvent): void => {
        if (e.altKey) return;
        const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
        const to = keyTarget(key, e.ctrlKey || e.metaKey, { first: scroll.first(), fits: scroll.fits(), total: lines() });
        if (to == null || !lines()) return;
        e.preventDefault();
        e.stopPropagation();
        if (to === "box") lineBox?.select();
        else if (to === "find") find.openStrip();
        else scroll.showLine(to);
    };
    /** Keys in the viewer bar: the find strip opens from the line box too. */
    const onBarKeyDown = (e: KeyboardEvent): void => {
        if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "f") {
            e.preventDefault();
            e.stopPropagation();
            find.openStrip();
        }
    };

    const facts = () => {
        const out = lines() ? [`Line ${(scroll.first() + 1).toLocaleString("en-US")} of ${lines().toLocaleString("en-US")}`] : [];
        if (props.probe) out.push(formatBytes(props.probe.size));
        return out;
    };
    return {
        ...feed,
        ...scroll,
        lines,
        mark,
        badLine,
        goTo,
        goToTyped,
        clearBadLine: () => setBadLine(false),
        bindLineBox: (el: HTMLInputElement) => (lineBox = el),
        find,
        onKeyDown,
        onBarKeyDown,
        facts,
    };
}

export type LargeTextState = ReturnType<typeof createLargeText>;
