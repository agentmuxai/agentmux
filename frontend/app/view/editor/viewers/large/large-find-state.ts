// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Large text viewer's find strip: the text typed, the search in flight and
 * what it found. One search at a time: a newer one, an edit and closing the
 * strip all end the one running. Like the hex viewer's, and over the same srv
 * search. docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.6.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { findMatch, type SearchCall } from "../hex/hex-find";
import { largeSearchFor, matcherFor, type LargeSearch, type LineMatcher } from "./large-find";

const searchOnSrv: SearchCall = (req) => RpcApi.FileSearchBytesCommand(TabRpcClient, req);

/** The search of text (case ignored, a regular expression): the sweep's pattern is the typed text. */
const textSearchOnSrv =
    (regex: boolean, ignoreCase: boolean): SearchCall =>
    (req) =>
        RpcApi.FileSearchTextCommand(TabRpcClient, { path: req.path, query: req.pattern, regex, ignore_case: ignoreCase, from: req.from, backward: req.backward });

export interface LargeFindParts {
    path: Accessor<string>;
    size: Accessor<number>;
    /** The file's text encoding, as srv or the probe names it. */
    encoding: Accessor<string | undefined>;
    /** Bumps when the file on disk changed and the viewer reloaded: a search begun before it read the old bytes. */
    revision: Accessor<number>;
    /** Where the first line on screen starts, in bytes: a search begins there. */
    topOffset: () => Promise<number>;
    /** The 0-based line that holds a byte. */
    lineOf: (offset: number) => Promise<number>;
    /** Show a line (1-based) and mark it, with the match at byte `offset` of the file as the current one in it. */
    show: (line: number, offset: number) => void;
    /** Take the mark off the line: nothing is found. */
    unmark: () => void;
    /** Back to the lines. */
    focusGrid: () => void;
    /** Tests give their own search of bytes, and of text. */
    call?: SearchCall;
    textCall?: (regex: boolean, ignoreCase: boolean) => SearchCall;
}

/** One search from `cursor`: where it ended up, "none" for no match, "changed" for a match past the file's end now, null for cancelled. */
async function searchFrom(
    o: LargeFindParts,
    p: { pattern: string; align: 1 | 2; call: SearchCall },
    cursor: number,
    backward: boolean,
    cancelled: () => boolean
): Promise<{ offset: number; length: number; line: number; wrapped: boolean } | "none" | "changed" | null> {
    // `hex` is the pattern the call is given: hex bytes for a search of bytes, the typed text for a search of text.
    const r = await findMatch({ call: p.call, path: o.path(), hex: p.pattern, size: o.size(), cursor, backward, align: p.align, cancelled });
    if (!r) return null;
    if (!r.found) return "none";
    if (r.offset >= o.size()) return "changed";
    const line = await o.lineOf(r.offset);
    return cancelled() ? null : { offset: r.offset, length: r.length, line, wrapped: r.wrapped };
}

type Ask = Exclude<LargeSearch, { error: string } | null>;

/** What srv is asked for `p`: the call, and the pattern and alignment it is given (hex bytes for bytes, the typed text for text). */
function askFor(o: LargeFindParts, p: Ask): { pattern: string; align: 1 | 2; call: SearchCall } {
    if (p.kind === "bytes") return { pattern: p.hex, align: p.align, call: o.call ?? searchOnSrv };
    return { pattern: p.query, align: 1, call: (o.textCall ?? textSearchOnSrv)(p.regex, p.ignoreCase) };
}

/**
 * The cursor a search goes on from: the match on show. A search of text goes on past the whole match, not one byte in
 * (`\d+` in "123" would find "23" next); going back, or for bytes, it is the match's start.
 */
function cursorFrom(p: Ask, kept: { offset: number; end: number }, backward: boolean): number {
    return p.kind === "text" && !backward ? kept.end - 1 : kept.offset;
}

/** srv's answer names its command ("filesearchtext: not a valid expression: …"): the reason is for the reader. */
function reasonOf(e: unknown): string {
    return (e instanceof Error ? e.message : String(e)).replace(/^filesearch(?:text|bytes):\s*/, "");
}

/** "Aa" (exact case, on) and ".*" (the text is a regular expression, off). `onChange` runs before a change of either, `onChanged` after it. */
function createFindOptions(onChange: () => void, onChanged: () => void) {
    const [matchCase, setMatchCase] = createSignal(true);
    const [regex, setRegex] = createSignal(false);
    return {
        matchCase,
        regex,
        get: () => ({ matchCase: matchCase(), regex: regex() }),
        toggleCase: () => {
            onChange();
            setMatchCase((v) => !v);
            onChanged();
        },
        toggleRegex: () => {
            onChange();
            setRegex((v) => !v);
            onChanged();
        },
    };
}

export function createLargeFind(o: LargeFindParts) {
    const [open, setOpen] = createSignal(false);
    const [text, setText] = createSignal("");
    const [status, setStatus] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    // A change of how the text is read ends the search on show: its match was of another question. With a text typed,
    // the search is asked again at once, with the new reading, rather than on the next Enter.
    const opts = createFindOptions(
        () => reset(),
        () => {
            if (text()) void go(false);
        }
    );
    /** What marks the match on show in its line (its text, or an expression); "" when none. */
    const [needle, setNeedle] = createSignal<LineMatcher>("");
    /** Where the match on show starts: the next search goes on from it. `key` is the text and options it was found with. */
    let at: { offset: number; end: number; key: string } | null = null;
    let input: HTMLInputElement | undefined;
    let run = 0;
    const options = opts.get;
    const key = () => `${opts.matchCase() ? "A" : "a"}${opts.regex() ? "r" : "t"}:${text()}`;
    const pattern = createMemo(() => largeSearchFor(o.encoding(), text(), options()));
    const error = (): string => {
        const p = pattern();
        return p && "error" in p ? p.error : "";
    };
    const reset = () => {
        run++;
        setBusy(false);
        setStatus("");
        setNeedle("");
        at = null;
    };
    // The file was rewritten or cut short: the offsets of the old bytes mean nothing now.
    createEffect(on(o.revision, reset, { defer: true }));
    const edit = (next: string) => {
        reset();
        setText(next);
    };
    const go = async (backward: boolean): Promise<void> => {
        const p = pattern();
        if (!p || "error" in p || !o.size() || !o.path()) return;
        const mine = ++run;
        setBusy(true);
        setStatus("Searching…");
        try {
            const kept = at && at.key === key() ? at : null;
            // From the match on show, else from the top of the screen.
            const top = kept ? cursorFrom(p, kept, backward) : (await o.topOffset()) - (backward ? 0 : 1);
            const r = await searchFrom(o, askFor(o, p), top, backward, () => mine !== run);
            if (mine !== run || !r) return;
            if (r === "none") {
                // The line marked from an earlier match would look like this search's answer.
                setNeedle("");
                at = null;
                o.unmark();
                setStatus("No match");
                return;
            }
            if (r === "changed") {
                setStatus("The file changed; search again");
                return;
            }
            at = { offset: r.offset, end: r.offset + r.length, key: key() };
            setNeedle(matcherFor(text(), options()));
            o.show(r.line + 1, r.offset);
            setStatus(`Line ${(r.line + 1).toLocaleString("en-US")}${r.wrapped ? ", from the other end of the file" : ""}`);
        } catch (e) {
            if (mine === run) setStatus(`Couldn't search: ${reasonOf(e)}`);
        } finally {
            if (mine === run) setBusy(false);
        }
    };
    const openStrip = () => {
        setOpen(true);
        queueMicrotask(() => {
            input?.focus();
            input?.select();
        });
    };
    const close = () => {
        reset();
        setOpen(false);
        o.focusGrid();
    };
    onCleanup(() => run++);
    return {
        open,
        text,
        status,
        busy,
        needle,
        error,
        canSearch: () => !!o.path() && o.size() > 0,
        edit,
        matchCase: opts.matchCase,
        regex: opts.regex,
        toggleCase: opts.toggleCase,
        toggleRegex: opts.toggleRegex,
        next: () => void go(false),
        previous: () => void go(true),
        openStrip,
        close,
        bindInput: (el: HTMLInputElement) => (input = el),
    };
}

export type LargeFind = ReturnType<typeof createLargeFind>;
