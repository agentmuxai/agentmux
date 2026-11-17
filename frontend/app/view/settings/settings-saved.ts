// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a setting the person just changed was written. `set` (settings-controls.tsx) reports each write here; a
 * row that names its setting (`SettingRow`'s `settingKey`) shows the result next to its label: a "Saved" flag that
 * fades after `DURATION_LINGER_MS`, or an error that stays until the next write.
 *
 * The flag shows once the write has succeeded, so a number field (which commits after its debounce, element/ui's
 * NumberInput) shows it once, not on each keystroke. Writes to one key in a burst keep the flag up and fade it once,
 * after the last.
 */

import { createSignal } from "solid-js";
import { DURATION_LINGER_MS } from "@/app/motion/tokens";

export interface SaveResult {
    kind: "saved" | "error";
    /** A new object for each report, so a row can restart the fade for a write that follows another. */
    seq: number;
}

const [results, setResults] = createSignal<Readonly<Record<string, SaveResult>>>({});
const timers = new Map<string, ReturnType<typeof setTimeout>>();
let seq = 0;

/** The latest result for a setting while it is showing, `undefined` when it has faded or was never written. */
export function saveResult(key: string): SaveResult | undefined {
    return results()[key];
}

function clear(key: string): void {
    const timer = timers.get(key);
    if (timer != null) clearTimeout(timer);
    timers.delete(key);
    setResults((all) => {
        if (!(key in all)) return all;
        const { [key]: _gone, ...rest } = all;
        return rest;
    });
}

/** `set` calls this when the write for `key` has finished. */
export function reportSave(key: string, ok: boolean): void {
    const timer = timers.get(key);
    if (timer != null) clearTimeout(timer);
    timers.delete(key);
    setResults((all) => ({ ...all, [key]: { kind: ok ? "saved" : "error", seq: ++seq } }));
    // A failure stays until the next write (it names a value that did not take); a success goes after its linger.
    if (ok) timers.set(key, setTimeout(() => clear(key), DURATION_LINGER_MS));
}

/** For tests: forget every result and timer. */
export function resetSaveResults(): void {
    for (const timer of timers.values()) clearTimeout(timer);
    timers.clear();
    setResults({});
}
