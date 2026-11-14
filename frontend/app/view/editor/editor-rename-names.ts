// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The pure parts of renaming a file from its document tab: what the name box
 * selects, what it refuses by itself, where an Untitled document goes, and
 * how two spellings of one path compare. srv owns the name rules of each
 * platform (`fs_ops::validate_name_rules`); only what a text box can know is
 * decided here.
 * docs/specs/SPEC_EDITOR_TAB_RENAME_2026_11_14.md §3, §5, §11.5.
 */

import { canonicalizePath } from "@/app/store/editor-pane-state-store";

/** The last part of a path. */
export function baseName(path: string): string {
    const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return i >= 0 ? path.slice(i + 1) : path;
}

/** The folder a path is in ("" for a bare name). A root keeps its slash. */
export function dirName(path: string): string {
    const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    if (i < 0) return "";
    if (i === 0 || /^[A-Za-z]:$/.test(path.slice(0, i))) return path.slice(0, i + 1);
    return path.slice(0, i);
}

/**
 * Where the first selection ends: at the last dot, so typing replaces the
 * name and keeps the extension. A name with no dot, or whose only dot leads
 * it (`.env`), is selected whole.
 */
export function nameStemEnd(name: string): number {
    const dot = name.lastIndexOf(".");
    return dot > 0 ? dot : name.length;
}

/**
 * F2 inside the box moves the selection on: the name, the whole name, the
 * extension, and round again. A name with nothing to tell apart stays whole.
 */
export function nextNameSelection(name: string, start: number, end: number): [number, number] {
    const stem = nameStemEnd(name);
    if (stem === name.length) return [0, name.length];
    if (start === 0 && end === stem) return [0, name.length];
    if (start === 0 && end === name.length) return [stem + 1, name.length];
    return [0, stem];
}

// C0 controls and DEL; the direction marks, embeddings, overrides and isolates.
const CONTROL_CHARS = /[\u0000-\u001f\u007f]/;
const DIRECTION_CHARS = /[\u200e\u200f\u202a-\u202e\u2066-\u2069]/;

/** Why the box itself refuses `text`, as a sentence; null when srv may judge it. */
export function typedNameProblem(text: string): string | null {
    if (CONTROL_CHARS.test(text)) return "A name can't contain control characters.";
    if (DIRECTION_CHARS.test(text)) return "A name can't contain hidden direction characters.";
    return null;
}

const isWindowsPath = (p: string): boolean => /^[A-Za-z]:\//.test(p) || p.startsWith("//");

/** A path as two panes can compare it: canonical, and without case where the disk has none (a Windows path). */
function pathKey(path: string): string {
    const p = canonicalizePath(path);
    return isWindowsPath(p) ? p.toLowerCase() : p;
}

/** Whether two spellings name one path. */
export function samePath(a: string, b: string): boolean {
    return pathKey(a) === pathKey(b);
}

/**
 * `path` after the rename of `oldPath` to `newPath`: the new path when it is
 * that file, the same place under the new folder when it was inside it, and
 * null when the rename doesn't concern it.
 */
export function pathAfterRename(path: string, oldPath: string, newPath: string): string | null {
    const [p, from] = [canonicalizePath(path), canonicalizePath(oldPath)];
    const to = canonicalizePath(newPath);
    const [key, fromKey] = [pathKey(p), pathKey(from)];
    if (key === fromKey) return to;
    // Inside it: the folder's own path, then a separator.
    if (key.startsWith(fromKey + "/")) return to + p.slice(from.length);
    return null;
}

const isAbsolute = (p: string): boolean => p.startsWith("/") || p.startsWith("\\") || /^[A-Za-z]:[\\/]/.test(p);
const isHomePath = (p: string): boolean => p === "~" || p.startsWith("~/") || p.startsWith("~\\");

/**
 * Where an Untitled document is saved for `typed`: a name goes into `folder`,
 * a relative path under it, and a path from the home folder (`~/…`) or a
 * full one is used as given. `problem` is a sentence for the box.
 */
export function untitledDestination(typed: string, folder: string): { path: string } | { problem: string } {
    const text = typed.trim();
    const problem = typedNameProblem(text);
    if (problem) return { problem };
    const parts = text.split(/[\\/]/);
    if (parts.includes("..")) return { problem: "A path can't contain “..”." };
    if (!parts[parts.length - 1]) return { problem: "A name can't be empty." };
    if (isAbsolute(text) || isHomePath(text)) return { path: text };
    const base = folder || "~";
    return { path: (/[\\/]$/.test(base) ? base : base + "/") + text };
}

/** An RPC failure as srv's own sentence, without the wrappers around it. */
export function errorSentence(err: unknown): string {
    const msg = err instanceof Error ? err.message : String(err);
    return msg.replace(/^(Error:\s*)+/, "").replace(/^[a-z][a-z.]*:\s+/, "");
}
