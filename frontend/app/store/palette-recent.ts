// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The commands a person ran from the command palette, newest first (SPEC_COMMAND_PALETTE_TOP_CENTRE_2026_11_17.md §5,
// R29 to R33). The last 20, by command id, in the app's local storage: the same in every window and after a restart.
// A command run by a key, a menu or an agent is not here; the list is what the person picked in the palette.

import { createSignal } from "solid-js";
import type { CommandEntry } from "@/app/store/command-registry";

const STORAGE_KEY = "palette:recent";
/** How many are kept (R29). */
export const RECENT_KEPT = 20;

function read(): string[] {
    try {
        const parsed: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]");
        if (!Array.isArray(parsed)) return [];
        return parsed.filter((id): id is string => typeof id === "string").slice(0, RECENT_KEPT);
    } catch {
        return [];
    }
}

function write(ids: string[]): void {
    try {
        localStorage.setItem(STORAGE_KEY, JSON.stringify(ids));
    } catch {
        // Storage full or blocked: the list lasts as long as this window does.
    }
}

const [recent, setRecent] = createSignal<string[]>(read());

// Another window of the app ran something: this one lists it too.
if (typeof window !== "undefined") {
    window.addEventListener("storage", (e) => {
        if (e.key === STORAGE_KEY || e.key === null) setRecent(read());
    });
}

/** The remembered command ids, newest first. Reactive. */
export function recentCommandIds(): string[] {
    return recent();
}

/** `id` was run from the palette: it goes to the front, once, and the list keeps its 20 newest (R29). */
export function rememberCommand(id: string): void {
    const next = [id, ...recent().filter((r) => r !== id)].slice(0, RECENT_KEPT);
    setRecent(next);
    write(next);
}

/** Forget every remembered command (R32). */
export function clearRecentCommands(): void {
    setRecent([]);
    write([]);
}

/** The palette command that empties the list (R32). Shown only while there is something to forget. */
export const clearRecentEntry: CommandEntry = {
    id: "palette:clearRecent",
    label: "Clear Recently Used Commands",
    category: "Window",
    icon: "clock-rotate-left",
    keywords: "recent history forget reset commands palette",
    when: () => recent().length > 0,
    execute: clearRecentCommands,
};
