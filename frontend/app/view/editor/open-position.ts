// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where in a file `pane.open` asks a viewer to open (`offset`: the hex viewer's byte; `page`: the PDF viewer's page;
 * the text viewer's `line` is open-at-line.ts). The Editor notes it when it opens the file; the viewer takes it once
 * when it has the file, whether it mounts afterwards or is already showing it. An entry nobody takes (the file opened
 * in a viewer that has no use for it) lapses, so a later open of the same file doesn't jump to an old request.
 */

import { canonicalizePath } from "@/app/store/editor-pane-state-store";
import { createSignal } from "solid-js";

export interface OpenPosition {
    /** The byte the hex viewer puts its cursor on. */
    offset?: number;
    /** The 1-based page the PDF viewer shows. */
    page?: number;
}

/** A new pane's meta: where its initial `file` opens (pane.open's `offset`, `page`), cleared once used. */
export const META_OFFSET = "editor:offset";
export const META_PAGE = "editor:page";
export const POSITION_META_CLEARED = { [META_OFFSET]: null, [META_PAGE]: null };

/** The offset and page a new pane's meta asks its initial file to open at. */
export function readPositionMeta(meta: Record<string, unknown> | undefined): OpenPosition {
    return { offset: whole(meta?.[META_OFFSET], 0), page: whole(meta?.[META_PAGE], 1) };
}

/** How long a request waits for its viewer. */
export const OPEN_POSITION_TTL_MS = 60_000;

interface Waiting extends OpenPosition {
    expires: number;
}

const waiting = new Map<string, Waiting>();
const [changed, setChanged] = createSignal(0);

const keyOf = (blockId: string, path: string) => `${blockId}\n${canonicalizePath(path)}`;

/** `n` when it is a whole number of at least `min`, else undefined (what a request from another process may carry). */
export function whole(n: unknown, min: number): number | undefined {
    return typeof n === "number" && Number.isInteger(n) && n >= min ? n : undefined;
}

/** Notes where `path` in `blockId`'s Editor should open; a request with neither field clears any waiting one. */
export function setOpenPosition(blockId: string, path: string, at: OpenPosition, now = Date.now()): void {
    const offset = whole(at.offset, 0);
    const page = whole(at.page, 1);
    const key = keyOf(blockId, path);
    if (offset === undefined && page === undefined) {
        if (waiting.delete(key)) setChanged((n) => n + 1);
        return;
    }
    waiting.set(key, { offset, page, expires: now + OPEN_POSITION_TTL_MS });
    setChanged((n) => n + 1);
}

/** Reads (and so tracks) the notes, so an effect that takes a position runs again when one is set. */
export function watchOpenPosition(): number {
    return changed();
}

/** The one field a viewer reads, once: it is gone after this. */
export function takeOpenPosition<K extends keyof OpenPosition>(blockId: string, path: string, field: K, now = Date.now()): OpenPosition[K] {
    const key = keyOf(blockId, path);
    const entry = waiting.get(key);
    if (!entry) return undefined;
    if (entry.expires < now) {
        waiting.delete(key);
        return undefined;
    }
    const value = entry[field];
    delete entry[field];
    if (entry.offset === undefined && entry.page === undefined) waiting.delete(key);
    return value;
}

/** Drops every waiting request (for tests). */
export function clearOpenPositions(): void {
    waiting.clear();
}
