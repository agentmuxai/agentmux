// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a streamed Browser pane and srv say to each other, and how a mouse, wheel or key event of the page becomes a
// message. The same protocol as crates/srv/src/server/streamed_browser/protocol.rs;
// docs/specs/SPEC_STREAMED_BROWSER_PANE_DEMO_2026_11_15.md §4.

/** First byte of a binary message: a JPEG frame follows. */
export const FRAME_KIND = 1;

/** One window of the pane: the page it started with, or a window a page opened (a sign-in pop-up, a link to a new tab). */
export type WindowInfo = { id: number; url: string; title: string; opener: number | null; closable: boolean };

export type ServerMsg =
    | { t: "nav"; url: string; title: string; canGoBack: boolean; canGoForward: boolean; loading: boolean }
    | { t: "windows"; active: number; windows: WindowInfo[] }
    | { t: "notice"; message: string }
    | { t: "size"; w: number; h: number }
    | { t: "error"; message: string }
    | { t: "clipboard"; text: string }
    | { t: "closed" };

/** The JPEG in a binary message, or null for any other kind (a client ignores kinds it does not know). */
export function frameBytes(data: ArrayBuffer): Uint8Array | null {
    const bytes = new Uint8Array(data);
    return bytes.length > 1 && bytes[0] === FRAME_KIND ? bytes.subarray(1) : null;
}

/** A text message from srv, or null when it is not one this pane knows. */
export function parseServerMessage(text: string): ServerMsg | null {
    let value: unknown;
    try {
        value = JSON.parse(text);
    } catch {
        return null;
    }
    if (typeof value !== "object" || value === null) return null;
    const m = value as Record<string, unknown>;
    switch (m.t) {
        case "nav":
            return {
                t: "nav",
                url: String(m.url ?? ""),
                title: String(m.title ?? ""),
                canGoBack: m.canGoBack === true,
                canGoForward: m.canGoForward === true,
                loading: m.loading === true,
            };
        case "windows":
            return parseWindows(m);
        case "notice":
            // The server already cleaned what a page wrote; this is the second guard, so a hidden or reordering character never shows.
            return { t: "notice", message: plainText(String(m.message ?? "")) };
        case "size":
            return typeof m.w === "number" && typeof m.h === "number" ? { t: "size", w: m.w, h: m.h } : null;
        case "error":
            return { t: "error", message: String(m.message ?? "") };
        case "clipboard":
            // What the person's remote browser copied, brought over because they asked (Ctrl+C or Ctrl+X in the pane).
            return typeof m.text === "string" ? { t: "clipboard", text: m.text.slice(0, CLIPBOARD_MAX) } : null;
        case "closed":
            return { t: "closed" };
        default:
            return null;
    }
}

function parseWindows(m: Record<string, unknown>): ServerMsg | null {
    if (typeof m.active !== "number" || !Array.isArray(m.windows)) return null;
    const windows: WindowInfo[] = [];
    for (const w of m.windows) {
        if (typeof w !== "object" || w === null) return null;
        const e = w as Record<string, unknown>;
        if (typeof e.id !== "number") return null;
        windows.push({ id: e.id, url: String(e.url ?? ""), title: String(e.title ?? ""), opener: typeof e.opener === "number" ? e.opener : null, closable: e.closable === true });
    }
    return { t: "windows", active: m.active, windows };
}

/** Characters a person cannot see, or that change how the text around them reads: controls, bidi marks, zero-width ones. */
// eslint-disable-next-line no-control-regex
const HIDDEN = /[\u0000-\u001F\u007F-\u009F\u00AD\u061C\u180E\u200B-\u200F\u202A-\u202E\u2060-\u206F\uFEFF\u{E0000}-\u{E007F}]/gu;

/** Words a page wrote, as one line with nothing hidden in it. */
export function plainText(text: string): string {
    // Line breaks and tabs become spaces first (they are controls, but they only separate words); then what is hidden goes.
    // (JavaScript's \s also matches the byte-order mark, so it must not run before the hidden characters are removed.)
    return text.replace(/[\t\n\r\v\f]+/g, " ").replace(HIDDEN, "").replace(/\s+/g, " ").trim();
}

/** The most characters of a site in a tab. */
export const LABEL_MAX = 24;

/** The site a window is on, as the tab names it: the host, else the title, else a placeholder. A long host is cut from the left, so
 * the end of it (the part that says whose site it is) stays visible; a title is cut at its end. */
export function windowLabel(w: Pick<WindowInfo, "url" | "title">): string {
    try {
        const host = plainText(new URL(w.url).host);
        if (host) return host.length > LABEL_MAX ? `…${host.slice(-(LABEL_MAX - 1))}` : host;
    } catch {
        // not an address: fall through to the title
    }
    const title = plainText(w.title);
    if (!title) return "…";
    return title.length > LABEL_MAX ? `${title.slice(0, LABEL_MAX - 1)}…` : title;
}

export type Modifiers = { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean };

/** CDP's modifier bits: Alt 1, Ctrl 2, Meta 4, Shift 8. */
export function modifiers(e: Modifiers): number {
    return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
}

const BUTTONS = ["left", "middle", "right"] as const;

/** The button a `MouseEvent.button` names; `none` for a move or a button the page has no use for (back, forward). */
export function buttonName(button: number): "left" | "middle" | "right" | "none" {
    return BUTTONS[button] ?? "none";
}

type Point = { clientX: number; clientY: number };
type Box = { left: number; top: number };

export function mouseMessage(type: "down" | "up" | "move", e: Point & Modifiers & { button: number; buttons: number; detail: number }, box: Box) {
    return {
        t: "mouse",
        type,
        x: e.clientX - box.left,
        y: e.clientY - box.top,
        button: type === "move" ? "none" : buttonName(e.button),
        buttons: e.buttons,
        clicks: type === "move" ? 0 : Math.max(1, e.detail),
        mods: modifiers(e),
    };
}

/** Pixels per line and per page when a wheel reports lines or pages. */
const LINE_PX = 16;

export function wheelMessage(e: Point & Modifiers & { deltaX: number; deltaY: number; deltaMode: number }, box: Box, pageHeight: number) {
    const scale = e.deltaMode === 1 ? LINE_PX : e.deltaMode === 2 ? pageHeight : 1;
    return { t: "wheel", x: e.clientX - box.left, y: e.clientY - box.top, dx: e.deltaX * scale, dy: e.deltaY * scale, mods: modifiers(e) };
}

/** What the key types: one character, or Enter as a carriage return; nothing for a chord with Ctrl or Meta. */
export function typedText(e: { key: string; ctrlKey: boolean; metaKey: boolean }): string {
    if (e.ctrlKey || e.metaKey) return "";
    if (e.key === "Enter") return "\r";
    return [...e.key].length === 1 ? e.key : "";
}

/** Cmd letters a Mac person presses for what the remote Linux browser knows as Ctrl (copy, cut, select all, undo, find, address, reload). */
const CMD_AS_CTRL = new Set(["c", "x", "a", "z", "f", "l", "r"]);

/** The modifiers the remote browser is sent for a key: on a Mac, Cmd with one of those letters (and no Ctrl or Alt) is the Ctrl chord. */
export function remoteModifiers(e: Modifiers & { key: string }, mac: boolean): Modifiers {
    // Read the four flags out one by one: a keyboard event's own properties are not its modifiers, so it cannot be spread.
    const own = { altKey: e.altKey, ctrlKey: e.ctrlKey, metaKey: e.metaKey, shiftKey: e.shiftKey };
    if (!mac || !e.metaKey || e.ctrlKey || e.altKey || !CMD_AS_CTRL.has(e.key.toLowerCase())) return own;
    return { ...own, ctrlKey: true, metaKey: false };
}

/** The screen backend presses the key the event's `code` names and ignores its modifier bits, so on a Mac the Cmd key itself is sent as the Control key. */
const CMD_KEY_AS_CTRL_KEY: Record<string, string> = { MetaLeft: "ControlLeft", MetaRight: "ControlRight" };

export function keyMessage(type: "down" | "up", e: Modifiers & { key: string; code: string; keyCode: number }, mac = false) {
    const asCtrl = mac ? CMD_KEY_AS_CTRL_KEY[e.code] : undefined;
    if (asCtrl) {
        const mods = { altKey: e.altKey, ctrlKey: true, metaKey: false, shiftKey: e.shiftKey };
        return { t: "key", type, key: "Control", code: asCtrl, text: "", vk: 17, mods: modifiers(mods) };
    }
    const mods = remoteModifiers(e, mac);
    return { t: "key", type, key: e.key, code: e.code, text: typedText({ key: e.key, ...mods }), vk: e.keyCode, mods: modifiers(mods) };
}

/** The most characters of a copy that are brought to the person's clipboard (the server's limit is in bytes). */
export const CLIPBOARD_MAX = 1024 * 1024;

/** Ctrl with C or X: the person copying or cutting in the remote browser. */
export function isCopyChord(e: { key: string; ctrlKey: boolean; altKey: boolean; metaKey: boolean }): boolean {
    return e.ctrlKey && !e.altKey && !e.metaKey && (e.key.toLowerCase() === "c" || e.key.toLowerCase() === "x");
}

/** Ctrl or Cmd with V: the browser's own paste event carries the text, so the key itself is not forwarded. */
export function isPasteChord(e: { key: string; ctrlKey: boolean; metaKey: boolean }): boolean {
    return (e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v";
}
