// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Layer L1b of verify-shortcuts.mjs: after a command runs, did the window show anything different?
// L1 says the command ran (RunCommand returned `ran`), L2 that its keys resolve to it. Neither says the
// user sees something happen. This takes a snapshot of what a person can see (the tabs, the panes and
// where they sit, which pane has focus, what is open on top, the zoom) before and after, and names what
// changed. A row where nothing changed is a failure, unless its command is one that has nothing to show
// here (NO_EFFECT_OK says why).

/** The commands that can run and leave the window as it was, with the reason. The row is then judged on L1
 *  and L2 only. Keep the reason a fact about the command, not "it failed once". */
export const NO_EFFECT_OK = {
    "app:escape": "with nothing open there is nothing to close",
    "view:zoom:reset": "a pane already at its default zoom has nothing to reset",
    "view:zoom:resetAll": "every pane already at its default zoom has nothing to reset",
    "agent:focusComposer": "focusing the agent pane already puts the caret in its composer",
    "term:clear": "a terminal with nothing in it has nothing to clear",
    "term:multiInput": "multi-input stays off with fewer than two terminals",
    "files:copy": "copying shows nothing",
    "term:copy": "copying shows nothing",
    "files:selectAll": "nothing to select in an empty folder",
};

/** Runs in the page: what a person can see. Cheap, and the same shape every time.
 *  `html` is a hash of the whole body, to catch what the named parts miss; the caller drops it when the page
 *  changes by itself (see `noisy`). */
export const SNAPSHOT = `(() => {
    const visible = (e) => {
        // Panes of other tabs stay mounted: count only what is on screen.
        const r = e.getBoundingClientRect();
        return r.width > 0 && r.height > 0 && r.right > 0 && r.bottom > 0 && r.left < innerWidth && r.top < innerHeight && getComputedStyle(e).visibility !== "hidden";
    };
    const hash = (s) => { let h = 5381; for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0; return h; };
    const panes = [...document.querySelectorAll(".block")].filter((b) => b.getAttribute("data-blockid") || /block-[0-9a-f]{8}/.test(b.className));
    const rect = (e) => { const r = e.getBoundingClientRect(); return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)].join(","); };
    const tiles = [...document.querySelectorAll(".tile-node, .tile-minimized")].filter(visible).map((t) => (t.querySelector("[data-blockid]")?.getAttribute("data-blockid") ?? "") + "@" + rect(t));
    const focusedIds = [...new Set([...document.querySelectorAll(".block-focused, .pane-stack-focused")].flatMap((e) => (e.className.match(/block-[0-9a-f-]{36}/) ?? [e.querySelector("[data-blockid]")?.getAttribute("data-blockid") ?? ""])))];
    const active = document.activeElement;
    return JSON.stringify({
        title: document.title,
        tabs: [...document.querySelectorAll(".tab")].map((t) => (t.className.includes("active") ? "*" : "") + (t.textContent ?? "").trim()).join("|"),
        panes: panes.length,
        tiles: tiles.sort().join(";"),
        focus: focusedIds.sort().join(",") + "/" + (active ? active.tagName + "." + String(active.className).slice(0, 40) : ""),
        paneZoom: [...document.querySelectorAll(".block-content")].map((e) => e.style.getPropertyValue("--pane-zoom")).join(","),
        dialogs: document.querySelectorAll("[role=dialog]").length,
        menus: document.querySelectorAll("[role=menu], [role=listbox], [role=tooltip]").length,
        zoom: window.devicePixelRatio + "/" + window.innerWidth + "/" + getComputedStyle(document.documentElement).getPropertyValue("--zoomfactor"),
        html: hash(document.body.outerHTML),
    });
})()`;

/** The snapshot fields a person would name, in the order a change is reported. */
const FIELDS = ["windows", "title", "tabs", "panes", "tiles", "focus", "dialogs", "menus", "zoom", "paneZoom", "html"];

/** Names the fields that differ between two snapshots (each a parsed SNAPSHOT). `html` is left out when the
 *  caller knows the page changes by itself. */
export function changed(before, after, { noisy = false } = {}) {
    return FIELDS.filter((f) => !(noisy && f === "html") && before[f] !== after[f]);
}

/** Two snapshots a moment apart with nothing run between them: the fields that change on their own. */
export function idleNoise(a, b) {
    return FIELDS.filter((f) => a[f] !== b[f]);
}

/** One row's verdict from what changed. `null` is a pass; otherwise the failure text. */
export function judge(command, fields, ran, ctx = {}) {
    if (!ran || fields.length) return null;
    return noEffectReason(command, ctx) ? null : "nothing visible changed";
}

/** Why a command may leave the window as it was: its table entry, or the target it names not existing
 *  (tab:goto:N with fewer than N tabs, pane:focus:N with fewer than N panes). `ctx` is `{ tabs, panes }`. */
export function noEffectReason(command, ctx = {}) {
    if (NO_EFFECT_OK[command]) return NO_EFFECT_OK[command];
    const m = /^(tab:goto|pane:focus):(\d+|last)$/.exec(command);
    if (!m) return null;
    const count = m[1] === "tab:goto" ? ctx.tabs : ctx.panes;
    if (m[2] === "last" || m[2] === "1") return count <= 1 ? `only ${count} of them is open, and it is already the one` : null;
    return count !== undefined && Number(m[2]) > count ? `there are ${count}, so there is no number ${m[2]}` : null;
}

/** Commands that move one way and have an opposite: at the edge (the first tab, the left-most pane) one of
 *  the pair does nothing, so the verifier runs the opposite to leave the edge and then the row again. */
export const OPPOSITE = {
    "tab:moveLeft": "tab:moveRight",
    "tab:moveRight": "tab:moveLeft",
    "tab:next": "tab:prev",
    "tab:prev": "tab:next",
    "pane:focus:up": "pane:focus:down",
    "pane:focus:down": "pane:focus:up",
    "pane:focus:left": "pane:focus:right",
    "pane:focus:right": "pane:focus:left",
    "pane:focus:next": "pane:focus:prev",
    "pane:focus:prev": "pane:focus:next",
    "pane:swap:up": "pane:swap:down",
    "pane:swap:down": "pane:swap:up",
    "pane:swap:left": "pane:swap:right",
    "pane:swap:right": "pane:swap:left",
    "pane:resize:up": "pane:resize:down",
    "pane:resize:down": "pane:resize:up",
    "pane:resize:left": "pane:resize:right",
    "pane:resize:right": "pane:resize:left",
};

/** The table cell for the effect column. */
export function describe(command, fields, ran, ctx = {}) {
    if (!ran) return "";
    if (fields.length) return `changed: ${fields.join(", ")}`;
    const why = noEffectReason(command, ctx);
    return why ? `none (${why})` : "nothing visible changed";
}
