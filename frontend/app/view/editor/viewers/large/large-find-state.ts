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
import { largePatternFor } from "./large-find";

const searchOnSrv: SearchCall = (req) => RpcApi.FileSearchBytesCommand(TabRpcClient, req);

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
    /** Tests give their own search. */
    call?: SearchCall;
}

/** One search from `cursor`: where it ended up, "none" for no match, "changed" for a match past the file's end now, null for cancelled. */
async function searchFrom(
    o: LargeFindParts,
    p: { hex: string; align: 1 | 2 },
    cursor: number,
    backward: boolean,
    cancelled: () => boolean
): Promise<{ offset: number; line: number; wrapped: boolean } | "none" | "changed" | null> {
    const r = await findMatch({ call: o.call ?? searchOnSrv, path: o.path(), hex: p.hex, size: o.size(), cursor, backward, align: p.align, cancelled });
    if (!r) return null;
    if (!r.found) return "none";
    if (r.offset >= o.size()) return "changed";
    const line = await o.lineOf(r.offset);
    return cancelled() ? null : { offset: r.offset, line, wrapped: r.wrapped };
}

export function createLargeFind(o: LargeFindParts) {
    const [open, setOpen] = createSignal(false);
    const [text, setText] = createSignal("");
    const [status, setStatus] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    /** The text of the match on show, for marking it in its line; "" when none. */
    const [needle, setNeedle] = createSignal("");
    /** Where the match on show starts: the next search goes on from it. */
    let at: { offset: number; text: string } | null = null;
    let input: HTMLInputElement | undefined;
    let run = 0;
    const pattern = createMemo(() => largePatternFor(o.encoding(), text()));
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
            const kept = at && at.text === text() ? at : null;
            // From the match on show, else from the top of the screen.
            const top = kept ? kept.offset : (await o.topOffset()) - (backward ? 0 : 1);
            const r = await searchFrom(o, p, top, backward, () => mine !== run);
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
            at = { offset: r.offset, text: text() };
            setNeedle(text());
            o.show(r.line + 1, r.offset);
            setStatus(`Line ${(r.line + 1).toLocaleString("en-US")}${r.wrapped ? ", from the other end of the file" : ""}`);
        } catch (e) {
            if (mine === run) setStatus(`Couldn't search: ${e instanceof Error ? e.message : String(e)}`);
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
        next: () => void go(false),
        previous: () => void go(true),
        openStrip,
        close,
        bindInput: (el: HTMLInputElement) => (input = el),
    };
}

export type LargeFind = ReturnType<typeof createLargeFind>;
