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
import { type Accessor, createComputed, createEffect, createMemo, createSignal, untrack } from "solid-js";
import type { ViewerBarProps } from "../viewer-bar";
import type { FileProbe } from "../viewer-choice";
import { rangeReaderOf } from "../hex/hex-chunks";
import { takeOpenPosition, watchOpenPosition } from "../../open-position";
import { LineSource, type LineText } from "./line-source";
import { createFollow } from "./large-follow";
import { createLargeFind } from "./large-find-state";
import { hitOrdinal, type LineMatcher } from "./large-find";
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

/**
 * The match to show in the drawn line `line` (1-based): the current one, else the first marked; null while that line
 * is not drawn (not another line's match).
 */
export function hitInLine(grid: HTMLElement | undefined, line: number): Element | null {
    const row = grid?.querySelector(`.large-text-line[data-line="${line}"]`);
    return row?.querySelector(".large-text-hit-current") ?? row?.querySelector(".large-text-hit") ?? null;
}

/** Scroll `grid` sideways, as little as the find needs, so that `hit` (a match in a line) shows in the middle. */
export function revealSideways(grid: HTMLElement | undefined, hit: Element | null): void {
    if (!grid || !hit) return;
    const g = grid.getBoundingClientRect();
    const h = hit.getBoundingClientRect();
    if (h.left >= g.left && h.right <= g.right) return;
    grid.scrollLeft += h.left - g.left - (g.width - h.width) / 2;
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

/**
 * A match far along a line, scrolled into view sideways once its line is drawn. A line read before is not drawn until
 * the scroll to it has happened, so this waits for the match itself in the marked line: whenever what `watch` reads
 * changes, and a frame at a time for a short while after `want`.
 */
function createSidewaysReveal(grid: () => HTMLElement | undefined, mark: Accessor<number | null>, watch: () => unknown) {
    // `watch` includes which match is current, so a Next within one line, which changes nothing else, reveals it.
    let wanted = 0; // the line (1-based) to show the match of; 0 for none
    const tryReveal = (): boolean => {
        const hit = wanted && wanted === untrack(mark) ? hitInLine(grid(), wanted) : null;
        if (!hit) return false;
        wanted = 0;
        revealSideways(grid(), hit);
        return true;
    };
    const poll = (frames = 40): void => {
        if (wanted && !tryReveal() && frames > 0) requestAnimationFrame(() => poll(frames - 1));
    };
    createEffect(() => {
        void watch();
        void mark();
        tryReveal();
    });
    return {
        want: (line: number) => {
            wanted = line;
        },
        poll,
    };
}

/**
 * The match on show among those a line holds. A find hands over where in the file the match starts; this works out how
 * far into its line that is, so the line's own matches can tell which one it is (`at`) and the reveal go to that one.
 */
function createCurrentHit(feed: ReturnType<typeof createLineFeed>, needle: () => LineMatcher, encoding: () => string | undefined) {
    /** The match on show: its line (1-based) and how many bytes into that line it starts. */
    const [current, setCurrent] = createSignal<{ line: number; byte: number } | null>(null);
    let newest = 0; // the newest find; an older one still working out its place gives up
    return {
        current,
        /** Make the match at byte `offset` of the file, in `line` (1-based), the current one, then run `then`. */
        place: async (line: number, offset: number, then: () => void): Promise<void> => {
            const mine = ++newest;
            let byte = -1;
            try {
                byte = offset - ((await untrack(feed.source)?.startOf(line - 1)) ?? offset);
            } catch {
                // No place in the line: its first match is shown.
            }
            if (mine !== newest) return;
            setCurrent(byte >= 0 ? { line, byte } : null);
            then();
        },
        /** Nothing is on show. */
        clear: () => {
            newest++;
            setCurrent(null);
        },
        /** Which match of drawn line `n` (0-based) is the current one (0 for the first); -1 when it is another line's or not drawn. */
        at: (n: number): number => {
            const c = current();
            const text = c && c.line === n + 1 ? feed.lineAt(n)?.text : undefined;
            return c && text != null ? hitOrdinal(text, needle(), c.byte, encoding()) : -1;
        },
    };
}

/** Opened at a line (pane.open's `line`): once the line count is known, go there. Runs again for a request that arrives while this viewer already shows the file. */
function useOpenLine(props: LargeTextProps, lines: Accessor<number>, goTo: (line: number) => void): void {
    createEffect(() => {
        watchOpenPosition();
        const total = lines();
        if (!total || !props.blockId || !props.path) return;
        const at = takeOpenPosition(props.blockId, props.path, "line");
        if (at !== undefined) untrack(() => goTo(Math.min(at, total)));
    });
}

/** Keys in the viewer bar: the find strip opens from the line box too. */
function openFindFromBar(e: KeyboardEvent, find: { openStrip: () => void }): void {
    if ((e.ctrlKey || e.metaKey) && !e.altKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        e.stopPropagation();
        find.openStrip();
    }
}

export function createLargeText(props: LargeTextProps) {
    const feed = createLineFeed(props);
    const lines = createMemo(() => Math.max(0, feed.total()));
    const scroll = createLineScroll(lines, remembered.get(props.path) ?? 0);
    const [mark, setMark] = createSignal<number | null>(null);
    let lineBox: HTMLInputElement | undefined;
    let gridEl: HTMLElement | undefined;
    let reveal!: ReturnType<typeof createSidewaysReveal>;
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
    useOpenLine(props, lines, goTo);
    const hit = createCurrentHit(feed, () => find.needle(), () => props.probe?.text?.encoding);
    const follow = createFollow(lines, scroll);
    const find = createLargeFind({
        path: () => props.path,
        size: () => props.probe?.size ?? 0,
        encoding: () => (void feed.loaded(), feed.source()?.encodingName() ?? props.probe?.text?.encoding),
        revision: () => props.revision,
        topOffset: async () => (await untrack(feed.source)?.startOf(scroll.first())) ?? 0,
        lineOf: feed.lineOf,
        // Mark the line and bring its current match, not only its first, into view.
        show: (line, offset) =>
            void hit.place(line, offset, () => {
                reveal.want(line);
                goTo(line);
                reveal.poll();
            }),
        unmark: () => {
            hit.clear();
            reveal.want(0);
            setMark(null);
        },
        focusGrid: scroll.focus,
    });
    reveal = createSidewaysReveal(() => gridEl, mark, () => [feed.loaded(), find.needle(), scroll.first(), hit.current()]);
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
    const onBarKeyDown = (e: KeyboardEvent): void => openFindFromBar(e, find);

    const facts = () => {
        const out = lines() ? [`Line ${(scroll.first() + 1).toLocaleString("en-US")} of ${lines().toLocaleString("en-US")}`] : [];
        if (props.probe) out.push(formatBytes(props.probe.size));
        return out;
    };
    return {
        ...feed,
        ...scroll,
        bind: (node: HTMLDivElement) => {
            gridEl = node;
            scroll.bind(node);
        },
        lines,
        mark,
        currentHit: hit.at,
        badLine,
        goTo,
        goToTyped,
        clearBadLine: () => setBadLine(false),
        bindLineBox: (el: HTMLInputElement) => (lineBox = el),
        find,
        follow,
        onKeyDown,
        onBarKeyDown,
        facts,
    };
}

export type LargeTextState = ReturnType<typeof createLargeText>;
