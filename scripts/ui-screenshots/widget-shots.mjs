// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The widget suite: one shot per widget, each in every size in sizes.mjs —
// see docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md §8. Run with
//
//   node scripts/ui-screenshots/capture.mjs --suite widgets --port N
//
// The list is built from the app's own widget config
// (crates/srv/src/config/widgets.json), so a new widget is captured without
// editing this file; EXCLUDE names the ones that make no sense as a shot.
//
// Each shot opens a new window tab, opens its widget there, maximizes the
// widget's pane so it fills the tab, and crops to that pane. The tab is closed
// afterwards, so shots don't affect each other.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const WIDGETS_JSON = join(
    dirname(fileURLToPath(import.meta.url)),
    "..",
    "..",
    "crates",
    "srv",
    "src",
    "config",
    "widgets.json"
);

/** Widgets not captured, and why. */
export const EXCLUDE = {
    // A group, not a pane: it opens a submenu of third-party web apps.
    messengers: "group of third-party web apps",
    // Its web page is drawn by a separate, native browser view laid over the
    // pane, which a screenshot of the app's page doesn't include. Capturing
    // it means compositing that view's own capture into the pane (spec §8).
    browser: "web content is a native view outside the page screenshot",
};

/** Per-widget overrides:
 *   - title, description, settleMs (for a widget that loads content after it opens);
 *   - marker: a selector inside the widget's pane, for a widget that focuses a
 *     pane already in the new tab's default layout instead of opening one;
 *   - afterOpen(session, paneSelector): extra setup once the pane is maximized;
 *   - containsWorkspaceData: true for a widget that always shows this
 *     machine's own data (its installs, paths), so it isn't published from an
 *     ordinary machine. The default, "review", means a human checks the image.
 *
 * Widgets that read the file system (Hangar, Terminal, Editor) are pointed at
 * a demo project by the capture instance's own widgets.json, not here — see
 * the spec's §8. */
const OVERRIDES = {
    agent: {
        marker: ".agent-view",
        description: "The agent picker: your agents, and the harnesses you can start a new one with.",
    },
    swarm: { marker: ".swarm-view" },
    help: { settleMs: 2500 },
    sysinfo: { marker: ".sysinfo-plot", settleMs: 2000, description: "Live CPU and memory graphs for this computer." },
    editor: {
        settleMs: 1500,
        afterOpen: async (session, pane) => {
            // A "language server not installed" notice isn't part of the editor.
            await session.clickSelector(`${pane} .editor-lsp-banner-dismiss`).catch(() => {});
        },
    },
    toolchain: {
        containsWorkspaceData: true,
        description: "The tools AgentMux uses on this computer, with their versions and where they're installed.",
    },
};

/** The widgets to capture, from the app's widget config: visible ones with a
 *  view, minus EXCLUDE. */
export function widgetEntries(config = JSON.parse(readFileSync(WIDGETS_JSON, "utf8"))) {
    return Object.entries(config)
        .map(([key, w]) => ({
            name: key.replace(/^defwidget@/, ""),
            label: w.label ?? key,
            icon: w.icon,
            view: w.blockdef?.meta?.view,
            hidden: !!w["display:hidden"],
        }))
        .filter((w) => !w.hidden && !EXCLUDE[w.name] && (w.view || w.name === "agent"));
}

/** Snapshot of every pane's block id, to find the pane a widget opens. */
const BLOCK_IDS = `[...document.querySelectorAll('.pane-stack [data-blockid]')].map(e => e.getAttribute('data-blockid'))`;

/** Centre of the first element matching `selector` that a click there would
 *  actually reach, or null. A hit test, not just "has a size": the top bar
 *  renders measuring copies of its icons that have a size but sit under, or
 *  away from, the real buttons. */
const visibleCenter = (selector) => `(() => {
    for (const el of document.querySelectorAll(${JSON.stringify(selector)})) {
        const r = el.getBoundingClientRect();
        if (r.width === 0 || r.height === 0) continue;
        const x = r.x + r.width / 2, y = r.y + r.height / 2;
        const hit = document.elementFromPoint(x, y);
        if (hit && (hit === el || el.contains(hit) || hit.contains(el))) return { x, y };
    }
    return null;
})()`;

/** Opens widget `w`, trying in turn: its pinned icon in the top bar (the icon
 *  class comes from widgets.json), its entry in the top bar's More list, the
 *  hamburger menu, and the command palette ("Open <label>"). Returns which
 *  route was used. */
async function openWidget(session, w) {
    if (w.icon && !w.icon.includes("@")) {
        const pt = await session.evaluate(visibleCenter(`.action-widgets .action-widget-slot .fa-${w.icon}`));
        if (pt) {
            await session.clickAt(pt);
            return "pinned icon";
        }
    }
    await session.clickSelector(".action-widget-more-btn");
    await session.wait(300);
    const more = await session.evaluate(
        `[...document.querySelectorAll('.action-widget-more-item-label')].some(e => e.offsetParent && e.textContent.trim() === ${JSON.stringify(w.label)})`
    );
    if (more) {
        await session.clickText(null, w.label, ".action-widget-more-item-label");
        return "More list";
    }
    await session.pressKey("Escape");
    await session.clickSelector(".hamburger-btn");
    await session.wait(300);
    const inMenu = await session.evaluate(
        `[...document.querySelectorAll('.menu .menu-item')].some(e => e.innerText.trim().split('\\n')[0].trim() === ${JSON.stringify(w.label)})`
    );
    if (inMenu) {
        await session.clickText(".menu", w.label);
        return "hamburger menu";
    }
    await session.pressKey("Escape");
    await session.pressKey("P", { ctrl: true, shift: true });
    await session.wait(300);
    await session.typeText(`Open ${w.label}`);
    await session.wait(300);
    await session.pressKey("Enter");
    return "command palette";
}

/** Turns "<selector> >> <n>" (the n-th match) into a selector for that one
 *  pane, by its block id. */
async function nthAsId(session, spec) {
    const [sel, n] = spec.split(" >> ");
    const id = await session.evaluate(
        `document.querySelectorAll(${JSON.stringify(sel)})[${Number(n)}]?.querySelector('[data-blockid]')?.getAttribute('data-blockid') ?? null`
    );
    return id ? `.pane-stack:has([data-blockid="${id}"])` : null;
}

/** True when the first element matching `paneSelector` is on screen: a hit
 *  test at its centre lands in it. Every window tab's panes stay laid out in
 *  the page, the hidden ones under the visible tab, so being in the DOM with a
 *  size doesn't mean being seen. (Park the pointer first: the tooltip of the
 *  widget icon just clicked can cover part of the pane.) */
const paneVisible = (paneSelector) => `(() => {
    const p = document.querySelector(${JSON.stringify(paneSelector)});
    if (!p) return false;
    const r = p.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    return !!hit && p.contains(hit);
})()`;

const TAB_IDS = `[...document.querySelectorAll('.tab[data-tab-id]')].map(t => t.getAttribute('data-tab-id'))`;

/** Closes the window tab `tabId`, confirming the "Close tab?" dialog if the
 *  app asks, and checks the tab is gone. Only ever a tab this suite created:
 *  closing "the active tab" could close the instance's own tab if the new one
 *  never became active (Codex on #4471). */
async function closeTab(session, tabId) {
    const tab = `.tab[data-tab-id="${tabId}"]`;
    if (!(await session.evaluate(`!!document.querySelector(${JSON.stringify(tab)})`))) return;
    // A wide viewport keeps the tab's close button clear of the window buttons.
    await session.setViewport({ width: 1920, height: 800, scale: 1 });
    await session.clickSelector(`${tab} [title="Close Tab"]`);
    await session.wait(400);
    const confirm = await session.evaluate(visibleCenter(".modal-backdrop ~ * button, [role=dialog] button"));
    if (confirm) {
        const pt = await session.evaluate(`(() => {
            const b = [...document.querySelectorAll('button')].find((e) => e.offsetParent && e.innerText.trim() === 'Close tab');
            if (!b) return null;
            const r = b.getBoundingClientRect();
            return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
        })()`);
        if (pt) await session.clickAt(pt);
        await session.wait(400);
    }
    if (await session.evaluate(`!!document.querySelector(${JSON.stringify(tab)})`)) {
        throw new Error(`the shot's tab (${tabId}) didn't close`);
    }
}

/** The block ids of the panes on screen (centre hit test, as paneVisible). */
const VISIBLE_BLOCK_IDS = `[...document.querySelectorAll('.pane-stack')].filter((p) => {
    const r = p.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) return false;
    const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    return !!hit && p.contains(hit);
}).map((p) => p.querySelector('[data-blockid]')?.getAttribute('data-blockid')).filter(Boolean).sort().join(',')`;

/** Opens a new window tab and waits until it is the active tab and its
 *  default panes have stopped changing. The app activates a new tab only once
 *  its layout is built, which can take seconds on a slow instance; acting
 *  before then puts the widget in the old tab, or takes the new tab's late
 *  panes for the widget's (Codex on #4471). Records the created tab's id in
 *  `state.tabId` as soon as it appears, even if this then times out, so the
 *  shot's cleanup closes that tab and no other. */
async function openNewTab(session, state) {
    const tabsBefore = new Set(await session.evaluate(TAB_IDS));
    await session.clickSelector(".hamburger-btn");
    await session.wait(300);
    await session.clickText(".menu", "New Tab");
    const deadline = Date.now() + 10000;
    let last = null;
    let stable = 0;
    while (Date.now() < deadline) {
        await session.wait(250);
        if (!state.tabId) state.tabId = (await session.evaluate(TAB_IDS)).find((id) => !tabsBefore.has(id)) ?? null;
        if (!state.tabId) continue;
        const active = await session.evaluate(
            `document.querySelector('.tab.active')?.getAttribute('data-tab-id') ?? null`
        );
        if (active !== state.tabId) continue;
        const panes = await session.evaluate(VISIBLE_BLOCK_IDS);
        stable = panes && panes === last ? stable + 1 : 0;
        last = panes;
        if (stable >= 3) return;
    }
    throw new Error("the new tab didn't become active with a settled layout within 10 s");
}

function widgetShot(w) {
    const o = OVERRIDES[w.name] ?? {};
    let paneSelector = null;
    // The tab this shot created, once it exists; cleanup closes only that tab.
    const tab = { tabId: null };
    return {
        id: `widget-${w.name}`,
        title: o.title ?? w.label,
        description: o.description ?? `The ${w.label} widget, in a pane of its own.`,
        sizes: true,
        // The UI's timing (a tab still settling, a click on a pane that's still
        // mounting) makes the odd attempt fail; one retry, after cleanup.
        retries: 1,
        settleMs: o.settleMs ?? 800,
        containsWorkspaceData: o.containsWorkspaceData ?? "review",
        prep: async (session) => {
            await session.setViewport({ width: 1280, height: 800, scale: 1 });
            await session.pressKey("Escape");
            tab.tabId = null;
            await openNewTab(session, tab);
            const before = new Set(await session.evaluate(BLOCK_IDS));
            const route = await openWidget(session, w);
            await session.wait(1200);
            // The widget's pane: a block that is new since the tab opened and is
            // on screen, or (for a widget already in the new tab's default
            // layout) the visible pane holding its marker.
            const fresh = (await session.evaluate(BLOCK_IDS)).filter((id) => !before.has(id));
            const candidates = fresh.map((id) => `.pane-stack:has([data-blockid="${id}"])`);
            if (o.marker) {
                const n = await session.evaluate(`document.querySelectorAll('.pane-stack:has(${o.marker})').length`);
                for (let i = 0; i < n; i++) candidates.push(`.pane-stack:has(${o.marker}) >> ${i}`);
            }
            paneSelector = null;
            await session.parkMouse();
            for (const c of candidates) {
                const sel = c.includes(" >> ") ? await nthAsId(session, c) : c;
                if (sel && (await session.evaluate(paneVisible(sel)))) {
                    paneSelector = sel;
                    break;
                }
            }
            if (!paneSelector) throw new Error(`opening ${w.label} (via ${route}) didn't open or focus a visible pane`);
            // Maximized means it fills the tab's width (less the pane gaps).
            // Poll rather than check once: a click on a pane still settling
            // after it opened can be missed, so it's clicked once more.
            const fillsTab = `(() => { const r = document.querySelector(${JSON.stringify(paneSelector)}).getBoundingClientRect(); return r.width > window.innerWidth * 0.95; })()`;
            let fills = false;
            for (let attempt = 0; attempt < 2 && !fills; attempt++) {
                await session.clickSelector(`${paneSelector} .block-frame-default-header [title="Maximize"]`);
                for (let i = 0; i < 12 && !fills; i++) {
                    await session.wait(250);
                    fills = await session.evaluate(fillsTab);
                }
            }
            if (!fills) throw new Error(`${w.label}'s pane didn't maximize`);
            if (o.afterOpen) await o.afterOpen(session, paneSelector);
        },
        selector: () => paneSelector,
        cleanup: async (session) => {
            await session.pressKey("Escape");
            if (tab.tabId) await closeTab(session, tab.tabId);
            tab.tabId = null;
            await session.clearViewport();
        },
    };
}

export const shots = widgetEntries().map(widgetShot);
