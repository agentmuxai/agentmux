// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The find strip's state: the pattern typed, its mode, the search in flight
 * and what it found. One search at a time: a newer one, an edit and closing
 * the strip all end the one running. docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.5.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { createEffect, createMemo, createSignal, on, onCleanup, type Accessor } from "solid-js";
import { findMatch, patternFor, type FindMode, type SearchCall } from "./hex-find";
import { offsetText } from "./hex-layout";

const searchOnSrv: SearchCall = (req) => RpcApi.FileSearchBytesCommand(TabRpcClient, req);

export interface HexFindParts {
    /** The file on disk; "" for dropped bytes, which srv cannot search. */
    path: Accessor<string>;
    size: Accessor<number>;
    /** Bumps when the file on disk changed and the viewer reloaded: a search begun before it read the old bytes. */
    revision?: Accessor<number>;
    /** The selection: a search starts past its end (before its start, going backward). */
    lo: Accessor<number>;
    hi: Accessor<number>;
    /** Select `length` bytes from `offset`, scrolled into view. */
    select: (offset: number, length: number) => void;
    /** Back to the grid. */
    focusGrid: () => void;
    /** Tests give their own search. */
    call?: SearchCall;
}

export function createHexFind(o: HexFindParts) {
    const [open, setOpen] = createSignal(false);
    const [mode, setModeSignal] = createSignal<FindMode>("hex");
    const [text, setText] = createSignal("");
    const [status, setStatus] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    let input: HTMLInputElement | undefined;
    let run = 0;
    const pattern = createMemo(() => patternFor(mode(), text()));
    const error = (): string => {
        const p = pattern();
        return p && "error" in p ? p.error : "";
    };
    const stop = () => {
        run++;
        setBusy(false);
    };
    // The file was rewritten or cut short: what a search in flight finds is an offset in the old bytes.
    if (o.revision) {
        createEffect(
            on(
                o.revision,
                () => {
                    if (!busy()) return;
                    stop();
                    setStatus("");
                },
                { defer: true }
            )
        );
    }
    const edit = (next: string) => {
        stop();
        setText(next);
        setStatus("");
    };
    const setMode = (next: FindMode) => {
        stop();
        setModeSignal(next);
        setStatus("");
    };
    const go = async (backward: boolean): Promise<void> => {
        const p = pattern();
        if (!p || "error" in p || !o.size() || !o.path()) return;
        const mine = ++run;
        setBusy(true);
        setStatus("Searching…");
        try {
            const r = await findMatch({
                call: o.call ?? searchOnSrv,
                path: o.path(),
                hex: p.hex,
                size: o.size(),
                cursor: backward ? o.lo() : o.hi(),
                backward,
                cancelled: () => mine !== run,
            });
            if (mine !== run || !r) return;
            if (r.found && r.offset >= o.size()) {
                setStatus("The file changed; search again");
            } else if (r.found) {
                o.select(r.offset, r.length);
                setStatus(
                    `Found at 0x${offsetText(r.offset, 1)}${r.wrapped ? ", from the other end of the file" : ""}`
                );
            } else {
                setStatus("No match");
            }
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
        stop();
        setOpen(false);
        o.focusGrid();
    };
    onCleanup(stop);
    return {
        open,
        mode,
        text,
        status,
        busy,
        error,
        canSearch: () => !!o.path() && o.size() > 0,
        setMode,
        edit,
        next: () => void go(false),
        previous: () => void go(true),
        openStrip,
        close,
        bindInput: (el: HTMLInputElement) => (input = el),
    };
}

export type HexFind = ReturnType<typeof createHexFind>;
