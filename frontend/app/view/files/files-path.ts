// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Paths as the Files pane shows and edits them: breadcrumb segments for
 * Windows drive paths, UNC shares and POSIX paths, the parent folder, joining
 * a name, and checking a new name before it goes to srv (which checks again).
 * docs/specs/SPEC_FILE_BROWSER_PANE_2026_10_01.md §5.1, §7.4, §9.
 */

/** One clickable breadcrumb segment: its label and the folder it opens. */
export interface Crumb {
    label: string;
    path: string;
}

const isWindowsLike = (path: string): boolean => /^[A-Za-z]:([\\/]|$)/.test(path) || /^(\\\\|\/\/)[^\\/]/.test(path);

/** The separator a path uses: `\` for a Windows drive or UNC path, else `/`. */
export function sepOf(path: string): string {
    return isWindowsLike(path) && !path.startsWith("//") ? "\\" : "/";
}

/**
 * The breadcrumb for a folder, root first. `C:\Users\a` gives `C:`, `Users`,
 * `a`; `\\server\share\x` gives `\\server\share`, `x` (a share is the root:
 * `\\server` alone can't be listed); `/home/a` gives `/`, `home`, `a`.
 */
export function crumbsOf(path: string): Crumb[] {
    if (!path) return [];
    const sep = sepOf(path);
    const parts = path.split(/[\\/]+/).filter((p) => p !== "");
    const out: Crumb[] = [];
    if (/^(\\\\|\/\/)/.test(path)) {
        if (parts.length < 2) return [{ label: path, path }];
        const root = `${sep}${sep}${parts[0]}${sep}${parts[1]}`;
        out.push({ label: root, path: root + sep });
        let cur = root;
        for (const p of parts.slice(2)) {
            cur = `${cur}${sep}${p}`;
            out.push({ label: p, path: cur });
        }
        return out;
    }
    if (/^[A-Za-z]:/.test(path)) {
        const drive = parts[0];
        out.push({ label: drive, path: drive + sep });
        let cur = drive;
        for (const p of parts.slice(1)) {
            cur = `${cur}${sep}${p}`;
            out.push({ label: p, path: cur });
        }
        return out;
    }
    out.push({ label: "/", path: "/" });
    let cur = "";
    for (const p of parts) {
        cur = `${cur}/${p}`;
        out.push({ label: p, path: cur });
    }
    return out;
}

/** The folder above `path`, or null at a root (`C:\`, `/`, a UNC share). */
export function parentOf(path: string): string | null {
    const crumbs = crumbsOf(path);
    return crumbs.length >= 2 ? crumbs[crumbs.length - 2].path : null;
}

/** `name` inside `dir`, with the folder's own separator. */
export function joinPath(dir: string, name: string): string {
    const sep = sepOf(dir);
    return dir.endsWith("/") || dir.endsWith("\\") ? `${dir}${name}` : `${dir}${sep}${name}`;
}

/** The last segment of a path. */
export function baseName(path: string): string {
    const parts = path.split(/[\\/]+/).filter((p) => p !== "");
    return parts[parts.length - 1] ?? path;
}

/** Two paths name the same folder: separators and a trailing one ignored; case
 *  ignored on Windows paths, where the filesystem ignores it. */
export function samePath(a: string, b: string): boolean {
    const norm = (p: string): string => {
        const s = p.replace(/[\\/]+/g, "/").replace(/(.)\/$/, "$1");
        return isWindowsLike(p) ? s.toLowerCase() : s;
    };
    return norm(a) === norm(b);
}

/** `inner` is `outer` or inside it. */
export function isWithin(inner: string, outer: string): boolean {
    if (samePath(inner, outer)) return true;
    const o = outer.replace(/[\\/]+$/, "");
    const lower = isWindowsLike(outer);
    const i = lower ? inner.toLowerCase() : inner;
    const p = lower ? o.toLowerCase() : o;
    return i.startsWith(p + "/") || i.startsWith(p + "\\");
}

const RESERVED_WINDOWS = /^(con|prn|aux|nul|com[1-9]|lpt[1-9])(\..*)?$/i;
// eslint-disable-next-line no-control-regex
const ILLEGAL_WINDOWS = /[<>:"|?*\u0000-\u001f]/;

/**
 * Why `name` can't be a file or folder name, as a sentence for the rename
 * box, or null when it can. Windows rules apply on Windows (srv enforces the
 * same set: this only saves a round trip).
 */
export function nameProblem(name: string, windows: boolean): string | null {
    if (name.trim() === "") return "A name can't be empty.";
    if (name === "." || name === "..") return `"${name}" isn't a valid name.`;
    if (/[\\/]/.test(name)) return "A name can't contain / or \\.";
    if (name.includes("\u0000")) return "A name can't contain a NUL character.";
    if (windows) {
        if (ILLEGAL_WINDOWS.test(name)) return 'A name can\'t contain < > : " | ? * or control characters.';
        if (/[. ]$/.test(name)) return "On Windows a name can't end with a dot or a space.";
        if (RESERVED_WINDOWS.test(name)) return `"${name}" is reserved by Windows.`;
    }
    return null;
}

/** The part of a name a rename selects first: everything before the last dot,
 *  unless the name starts with its only dot (`.gitignore`) or is a folder. */
export function stemLength(name: string, isDir: boolean): number {
    if (isDir) return name.length;
    const dot = name.lastIndexOf(".");
    return dot > 0 ? dot : name.length;
}

/**
 * `path` with a leading `~` replaced by `home` and `.` and `..` segments
 * resolved, lexically: links can't be followed here. For comparing where a
 * path points, not for display.
 */
export function normalizePath(path: string, home: string): string {
    let p = path.trim();
    if (home && (p === "~" || /^~[\\/]/.test(p))) p = home + p.slice(1);
    const crumbs = crumbsOf(p);
    if (crumbs.length === 0) return p;
    const sep = sepOf(p);
    const root = crumbs[0].path;
    const out: string[] = [];
    for (const c of crumbs.slice(1)) {
        if (c.label === ".") continue;
        if (c.label === "..") out.pop();
        else out.push(c.label);
    }
    if (out.length === 0) return root;
    return root.endsWith(sep) ? root + out.join(sep) : root + sep + out.join(sep);
}
