// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Files agents changed recently, by path: what the Files pane's "touched by"
 * badges show (docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §8.4).
 *
 * The source is the agents' own tool calls as their panes receive them live
 * (Write, Edit, MultiEdit, NotebookEdit), not filesystem watching: a watcher
 * can't say who changed a file, and a user's save in the Editor must not
 * show a badge. A call counts only once its result says it succeeded.
 */

import { createSignal } from "solid-js";

export interface Touch {
    /** The agent pane's block, and its name and colour when it had them. */
    blockId: string;
    agentName: string;
    color?: string;
    tool: string;
    /** When the change landed (ms). */
    at: number;
}

/** A badge shows for this long after the change. */
export const TOUCH_TTL_MS = 30 * 60 * 1000;
/** Calls waiting for their result; more than this and the oldest go. */
const MAX_PENDING = 500;
/** Paths remembered; more than this and the oldest go. */
const MAX_TOUCHED = 2000;

/** Tools that write a file, and the parameter naming it. */
const FILE_TOOLS: Record<string, string> = {
    Write: "file_path",
    Edit: "file_path",
    MultiEdit: "file_path",
    NotebookEdit: "notebook_path",
};

/** A path as a comparable key: one separator, and no case on Windows-style
 *  paths, whose filesystems ignore it. */
export function touchKey(path: string): string {
    // Decided on the path as given: normalising first would turn `C:\` into
    // `C:` and a UNC `\\srv\share` into `/srv/share` (muxreview on #4224).
    const windows = isWindowsStyle(path);
    const unc = /^[\\/]{2}[^\\/]/.test(path);
    let p = path.replace(/[\\/]+/g, "/").replace(/(.)\/$/, "$1");
    if (unc) p = "/" + p;
    return windows ? p.toLowerCase() : p;
}

/** A drive (`C:`, `C:\…`) or UNC (`\\srv\share`) path. */
function isWindowsStyle(path: string): boolean {
    return /^[A-Za-z]:([\\/]|$)/.test(path) || /^[\\/]{2}[^\\/]/.test(path);
}

const isAbsolute = (p: string): boolean => /^[A-Za-z]:[\\/]/.test(p) || p.startsWith("/") || p.startsWith("\\\\");

const pending = new Map<string, { path: string; touch: Omit<Touch, "at"> }>();
const [touched, setTouched] = createSignal<ReadonlyMap<string, Touch>>(new Map());

/** The live map of touched files, by `touchKey`. */
export { touched };

/** An agent pane saw a tool call start: remember it if it writes a file. */
export function noteToolCall(
    info: { blockId: string; agentName?: string; color?: string },
    call: { id: string; tool: string; params?: Record<string, unknown> }
): void {
    const param = FILE_TOOLS[call.tool];
    const path = param ? call.params?.[param] : undefined;
    if (typeof path !== "string" || !isAbsolute(path)) return;
    pending.set(call.id, {
        path,
        touch: { blockId: info.blockId, agentName: info.agentName || "agent", color: info.color, tool: call.tool },
    });
    while (pending.size > MAX_PENDING) pending.delete(pending.keys().next().value!);
}

/** Its result arrived: a success records the file as touched now. */
export function noteToolResult(id: string, status: string | undefined, now: number = Date.now()): void {
    const p = pending.get(id);
    if (!p) return;
    pending.delete(id);
    if (status !== "success") return;
    const next = new Map(touched());
    const key = touchKey(p.path);
    // Re-insert so the map's order is oldest first.
    next.delete(key);
    next.set(key, { ...p.touch, at: now });
    for (const [k, t] of next) {
        if (next.size <= MAX_TOUCHED && now - t.at <= TOUCH_TTL_MS) break;
        next.delete(k);
    }
    setTouched(next);
}

/**
 * The most recent touch, within the TTL, of each direct child of `dir` that
 * is (or contains) a touched file: a folder shows the latest change inside it.
 */
export function touchesUnder(dir: string, now: number = Date.now()): Map<string, Touch> {
    const dirKey = touchKey(dir);
    const base = dirKey.endsWith("/") ? dirKey : dirKey + "/";
    const out = new Map<string, Touch>();
    for (const [key, t] of touched()) {
        if (now - t.at > TOUCH_TTL_MS || !key.startsWith(base)) continue;
        const rest = key.slice(base.length);
        const child = rest.split("/", 1)[0];
        if (!child) continue;
        const prev = out.get(child);
        if (!prev || prev.at < t.at) out.set(child, t);
    }
    return out;
}

/** How `touchesUnder(dir)` names the child `name`: lower case under a
 *  Windows-style folder, as its keys are. */
export function childKey(dir: string, name: string): string {
    return isWindowsStyle(dir) ? name.toLowerCase() : name;
}

/** Test hook. */
export function resetTouchedForTests(): void {
    pending.clear();
    setTouched(new Map());
}
