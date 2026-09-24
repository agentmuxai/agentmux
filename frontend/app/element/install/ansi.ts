// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Minimal ANSI SGR parser for the install log (spec §4.2 "ANSI colour
 * support"): bold and the 16 basic foreground colours, mapped to CSS
 * classes. Every other escape sequence is dropped, so the log never shows
 * raw `ESC[…` noise.
 */

export interface AnsiSegment {
    text: string;
    /** Space-separated classes, e.g. "log-fg-red log-bold"; empty when plain. */
    cls: string;
}

const ESC = String.fromCharCode(27);
// CSI sequences (`ESC [ params final`) and two-byte escapes (`ESC x`).
const ESCAPE_RE = new RegExp(`${ESC}(?:\\[([0-9;?]*)([@-~])|[@-Z\\\\-_])`, "g");

const FG = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];

export function parseAnsi(input: string): AnsiSegment[] {
    if (!input.includes(ESC)) return [{ text: input, cls: "" }];
    const out: AnsiSegment[] = [];
    let fg: string | null = null;
    let bold = false;
    let last = 0;
    const push = (text: string) => {
        if (!text) return;
        const cls = [fg ? `log-fg-${fg}` : "", bold ? "log-bold" : ""].filter(Boolean).join(" ");
        const prev = out[out.length - 1];
        if (prev && prev.cls === cls) prev.text += text;
        else out.push({ text, cls });
    };
    for (const m of input.matchAll(ESCAPE_RE)) {
        push(input.slice(last, m.index));
        last = m.index! + m[0].length;
        if (m[2] !== "m") continue; // only SGR changes style
        const codes = (m[1] || "0").split(";").map((c) => Number(c || 0));
        for (const c of codes) {
            if (c === 0) {
                fg = null;
                bold = false;
            } else if (c === 1) bold = true;
            else if (c === 22) bold = false;
            else if (c === 39) fg = null;
            else if (c >= 30 && c <= 37) fg = FG[c - 30];
            else if (c >= 90 && c <= 97) fg = `bright-${FG[c - 90]}`;
        }
    }
    push(input.slice(last));
    return out.length ? out : [{ text: "", cls: "" }];
}
