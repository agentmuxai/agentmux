// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pinned folders and recent folders for the Files pane's Places sidebar
 * (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.5). Both are the person's, not a pane's: every Files pane
 * in the app shows the same lists, kept in the browser storage the window shares (the way the macOS "listed
 * safely" folders are), so a pin made in one pane shows in the others at once.
 *
 * A folder is a path and, for one on an SSH host, the connection's name. Nothing here lists a folder: a
 * recent is recorded only after its first page of rows came back, and the sidebar shows names only, so a
 * protected macOS folder is never listed by Places (§9.1.4).
 */

import { createSignal } from "solid-js";
import { baseName, samePath } from "./files-path";

/** A folder on this computer (no `connection`) or on an SSH host. */
export interface PlaceRef {
    path: string;
    connection?: string;
}

const PINNED_KEY = "files:places:pinned";
const RECENT_KEY = "files:places:recent";
/** How many recent folders are kept; the sidebar shows fewer. */
export const MAX_RECENTS = 8;
export const SHOWN_RECENTS = 5;

export function samePlace(a: PlaceRef, b: PlaceRef): boolean {
    return (a.connection ?? "") === (b.connection ?? "") && samePath(a.path, b.path);
}

/** The name a sidebar row shows: the folder's own name, or the path at a drive or share root. */
export function placeLabel(p: PlaceRef): string {
    return baseName(p.path) || p.path;
}

/** The tooltip: where it is, and on which host. */
export function placeTitle(p: PlaceRef): string {
    return p.connection ? `${p.path} on ${p.connection}` : p.path;
}

function read(key: string): PlaceRef[] {
    try {
        const raw = JSON.parse(localStorage.getItem(key) ?? "[]") as unknown;
        if (!Array.isArray(raw)) return [];
        const out: PlaceRef[] = [];
        for (const v of raw) {
            const r = v as Partial<PlaceRef> | null;
            if (!r || typeof r.path !== "string" || r.path === "") continue;
            out.push(typeof r.connection === "string" && r.connection !== "" ? { path: r.path, connection: r.connection } : { path: r.path });
        }
        return out;
    } catch {
        return [];
    }
}

function write(key: string, list: PlaceRef[]): void {
    try {
        localStorage.setItem(key, JSON.stringify(list));
    } catch {
        // Storage full or unavailable: the lists last until the window closes.
    }
}

const [pinnedSignal, setPinned] = createSignal<PlaceRef[]>(read(PINNED_KEY));
const [recentSignal, setRecent] = createSignal<PlaceRef[]>(read(RECENT_KEY));

// Another window of the app changed a list.
if (typeof window !== "undefined") {
    window.addEventListener("storage", (e) => {
        if (e.key === PINNED_KEY) setPinned(read(PINNED_KEY));
        else if (e.key === RECENT_KEY) setRecent(read(RECENT_KEY));
    });
}

/** The pinned folders, in the order they were pinned. */
export const pinnedPlaces = (): PlaceRef[] => pinnedSignal();

/** The recent folders, newest first, without the pinned ones (they show above). */
export const recentPlaces = (): PlaceRef[] => {
    const pinned = pinnedSignal();
    return recentSignal()
        .filter((r) => !pinned.some((p) => samePlace(p, r)))
        .slice(0, SHOWN_RECENTS);
};

export const isPinned = (p: PlaceRef): boolean => pinnedSignal().some((x) => samePlace(x, p));

export function pinPlace(p: PlaceRef): void {
    if (isPinned(p)) return;
    const next = [...pinnedSignal(), p];
    setPinned(next);
    write(PINNED_KEY, next);
}

export function unpinPlace(p: PlaceRef): void {
    const next = pinnedSignal().filter((x) => !samePlace(x, p));
    setPinned(next);
    write(PINNED_KEY, next);
}

export function togglePin(p: PlaceRef): void {
    if (isPinned(p)) unpinPlace(p);
    else pinPlace(p);
}

/** A folder the person opened: first in the recents, once. */
export function addRecent(p: PlaceRef): void {
    const next = [p, ...recentSignal().filter((x) => !samePlace(x, p))].slice(0, MAX_RECENTS);
    setRecent(next);
    write(RECENT_KEY, next);
}

export function clearRecents(): void {
    setRecent([]);
    write(RECENT_KEY, []);
}

/** For the tests: forget what was read, and what was kept. */
export function resetPlacesForTests(): void {
    try {
        localStorage.removeItem(PINNED_KEY);
        localStorage.removeItem(RECENT_KEY);
    } catch {
        // Nothing kept.
    }
    setPinned([]);
    setRecent([]);
}
