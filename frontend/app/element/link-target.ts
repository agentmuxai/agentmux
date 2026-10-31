// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a link's `href` points at, decided by its text alone, so the click
 * handler (link-open.ts) and the Markdown pipeline agree on it:
 *
 *  - `web`: an http(s) page (the system browser; a browser pane on middle-click).
 *  - `file`: a local path, written as `C:/…`, `C:\…`, `/…`, `~/…`, `\\host\share\…` or a `file:` URL
 *    (opened in the Editor, or in Files when it is a folder).
 *  - `task`: `agentmux://task/<n>`, the Kanban card for task #n.
 *  - `blocked`: any other `agentmux:` URL. A link may only navigate, so it is never handed to the OS.
 *  - `other`: everything else (mailto:, ssh:, a relative link), left to the system handler as before.
 */
export type LinkTarget =
    | { kind: "web"; url: string }
    | { kind: "file"; path: string }
    | { kind: "task"; num: number }
    | { kind: "blocked" }
    | { kind: "other" };

const TASK_LINK = /^agentmux:\/\/task\/(\d{1,9})\/?$/i;
const DRIVE_PATH = /^[A-Za-z]:(?:[\\/]|%5C)/i;
const UNC_PATH = /^\\\\[^\\/]/;

function decode(text: string): string {
    try {
        return decodeURIComponent(text);
    } catch {
        return text;
    }
}

/** The path a `file:` URL names: `file:///C:/a%20b` is `C:/a b`, `file:///home/u` is `/home/u`,
 *  `file://server/share/x` is the UNC path `//server/share/x`. */
function pathOfFileUrl(href: string): string | null {
    const rest = href.replace(/^file:/i, "");
    const m = /^\/\/([^/]*)(\/.*)?$/.exec(rest);
    if (!m) return null;
    const host = m[1];
    const body = decode((m[2] ?? "").replace(/[?#].*$/, ""));
    if (!body) return null;
    if (host && host.toLowerCase() !== "localhost") return `//${host}${body}`;
    // `/C:/x` is a drive path whose leading slash is only the URL's.
    return /^\/[A-Za-z]:[\\/]/.test(body) ? body.slice(1) : body;
}

export function classifyHref(href: string | null | undefined): LinkTarget {
    const text = (href ?? "").trim();
    if (!text) return { kind: "other" };
    if (/^https?:\/\//i.test(text)) return { kind: "web", url: text };
    if (/^agentmux:/i.test(text)) {
        const task = TASK_LINK.exec(text);
        return task ? { kind: "task", num: Number(task[1]) } : { kind: "blocked" };
    }
    if (/^file:/i.test(text)) {
        const path = pathOfFileUrl(text);
        return path ? { kind: "file", path } : { kind: "blocked" };
    }
    if (DRIVE_PATH.test(text)) return { kind: "file", path: decode(text) };
    if (UNC_PATH.test(text)) return { kind: "file", path: text };
    // `//host/x` is a protocol-relative URL, not a path.
    if (/^\/(?!\/)/.test(text)) return { kind: "file", path: decode(text) };
    if (/^~(?:[\\/]|$)/.test(text)) return { kind: "file", path: decode(text) };
    return { kind: "other" };
}
