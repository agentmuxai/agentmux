// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Takes the capturing machine's own names out of the page just before each
// capture — see docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §9.
// An isolated data folder (AGENTMUX_HOME_OVERRIDE) keeps the machine's agents,
// accounts and memory out of a shot, but not the machine itself: the status bar
// shows its hostname, an empty tab shows user@host, and paths show the user
// name. Redaction rewrites those in what the page already shows. It's
// display-only: nothing is saved, and the next render puts the real text back.
//
// It can't reach text drawn into a canvas (a terminal's contents, rendered by
// xterm's WebGL renderer), so a shot of a terminal still needs a prompt that
// names neither (see demo-env.mjs).

import { hostname, networkInterfaces, userInfo } from "node:os";

/** What each kind of name is replaced with. An address becomes one from the
 *  range reserved for documentation (RFC 5737). */
export const REPLACEMENTS = { user: "demo", host: "demo-host", address: "192.0.2.10" };

/** This machine's own IPv4 addresses, not loopback (the host panel shows its
 *  LAN address). */
function localAddresses() {
    try {
        return Object.values(networkInterfaces())
            .flat()
            .filter((a) => a && a.family === "IPv4" && !a.internal)
            .map((a) => a.address);
    } catch {
        return [];
    }
}

/** Names shorter than this aren't replaced: too likely to match ordinary words. */
export const MIN_LENGTH = 3;

function currentUser() {
    try {
        return userInfo().username;
    } catch {
        return "";
    }
}

/** The `{from, to}` pairs to redact: any `extra` pairs first, then the user
 *  name, the hostname and the machine's IPv4 addresses. A name that's too
 *  short, or that an earlier pair already covers (case-insensitively; a
 *  machine named after its user, say), is skipped. */
export function machinePairs({ user = currentUser(), host = hostname(), addresses = localAddresses() } = {}, extra = []) {
    const pairs = [];
    const add = (from, to) => {
        if (!from || from.length < MIN_LENGTH) return;
        if (pairs.some((p) => p.from.toLowerCase() === from.toLowerCase())) return;
        pairs.push({ from, to });
    };
    for (const { from, to } of extra) add(from, to);
    add(user, REPLACEMENTS.user);
    add(host, REPLACEMENTS.host);
    for (const a of addresses) add(a, REPLACEMENTS.address);
    return pairs;
}

/** Parses a `--redact from=to` value. */
export function parseRedactArg(value) {
    const i = value.indexOf("=");
    if (i <= 0 || i === value.length - 1) throw new Error(`--redact "${value}": expected from=to`);
    return { from: value.slice(0, i), to: value.slice(i + 1) };
}

/** Runs in the page (serialized by `redactExpression`), or against any
 *  `document` in tests. Replaces each pair's `from`, as a whole word and
 *  ignoring case, in every text node under `body` and in the title,
 *  placeholder, aria-label and value of every element that has one. Returns
 *  the number of replacements, never the text replaced. */
export function redactDocument(pairs, doc = document) {
    const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
    const rules = pairs.map((p) => ({
        // Not inside a longer word, nor a longer address ("1.203.0.113.26"
        // doesn't contain 203.0.113.26; a full stop after it still ends it).
        re: new RegExp(`(?<![A-Za-z0-9]|\\d\\.)${escape(p.from)}(?![A-Za-z0-9]|\\.\\d)`, "gi"),
        to: p.to,
    }));
    let count = 0;
    const fix = (s) => {
        let out = s;
        for (const r of rules) {
            out = out.replace(r.re, () => {
                count++;
                return r.to;
            });
        }
        return out;
    };
    const walker = doc.createTreeWalker(doc.body, 4 /* NodeFilter.SHOW_TEXT */);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const v = fix(node.data);
        if (v !== node.data) node.data = v;
    }
    const attrs = ["title", "placeholder", "aria-label"];
    for (const el of doc.querySelectorAll("[title], [placeholder], [aria-label]")) {
        for (const a of attrs) {
            const v = el.getAttribute(a);
            if (!v) continue;
            const f = fix(v);
            if (f !== v) el.setAttribute(a, f);
        }
    }
    for (const el of doc.querySelectorAll("input, textarea")) {
        if (!el.value) continue;
        const f = fix(el.value);
        if (f !== el.value) el.value = f;
    }
    return count;
}

/** A `Runtime.evaluate` expression that runs `redactDocument(pairs)` in the page. */
export function redactExpression(pairs) {
    return `(${redactDocument.toString()})(${JSON.stringify(pairs)})`;
}
