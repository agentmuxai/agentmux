// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The chrome suite: the window's own chrome, its menus, the status bar and its
// panels, and every Settings tab — Part 2 of
// docs/reports/REPORT_DOCS_SCREENSHOT_COVERAGE_PLAN_2026_10_10.md, spec §9. Run
// with
//
//   node scripts/ui-screenshots/capture.mjs --suite chrome --port N
//
// Every shot lays the page out at the medium size (so images match from run to
// run whatever the window), starts with nothing open, checks with `verify` that
// what it opened is on screen, and closes it again afterwards. Menus are DOM
// overlays in the page (spec §9.3), found by these selectors:
//   - the hamburger menu and the top bar's submenus: `.menu`, and each open
//     submenu as a separate `.menu.sub-menu` at the end of the body;
//   - right-click and pane `+` menus: `.menu` inside `#cef-context-menu-overlay`,
//     submenus nested in their row.

import { SIZES } from "./sizes.mjs";
import { VISIBLE_BLOCK_IDS, closeTab, openNewTab, visibleCenter } from "./widget-shots.mjs";

const VIEWPORT = SIZES.medium;

const MENU = ".menu:not(.sub-menu)";
const SUBMENU = ".menu.sub-menu";
const CONTEXT_MENU = "#cef-context-menu-overlay > .menu";
const CONTEXT_SUBMENU = "#cef-context-menu-overlay .menu.sub-menu";
const MORE_DROPDOWN = ".action-widget-more-dropdown";
const PALETTE = ".command-palette-panel";
const POPOVER = ".anchored-popover";
const SETTINGS = ".settings-view-container";

/** Anything a shot can leave open. */
const TAB_PANEL = ".tab-context-panel";
const OPEN_OVERLAYS = [MENU, CONTEXT_MENU, MORE_DROPDOWN, ".modal-root", POPOVER, TAB_PANEL];

const anyOpen = `(() => {
    for (const sel of ${JSON.stringify(OPEN_OVERLAYS)}) {
        for (const el of document.querySelectorAll(sel)) {
            const r = el.getBoundingClientRect();
            if (r.width > 0 && r.height > 0) return sel;
        }
    }
    return null;
})()`;

/** Closes whatever is open (Escape, then a click on the empty middle of the
 *  status bar), and throws if something still is: it would be in every later
 *  shot. */
async function dismiss(session) {
    for (let i = 0; i < 3; i++) {
        const open = await session.evaluate(anyOpen);
        if (!open) return;
        if (i < 2) await session.pressKey("Escape");
        else {
            const pt = await session.centerOf(".status-bar-center");
            if (pt) await session.clickAt(pt);
        }
        await session.wait(250);
    }
    const open = await session.evaluate(anyOpen);
    if (open) throw new Error(`${open} is still open`);
}

/** The first pane on screen (hit test at its centre), as a selector by block id. */
async function firstVisiblePane(session) {
    const ids = await session.evaluate(VISIBLE_BLOCK_IDS);
    const id = ids ? ids.split(",")[0] : null;
    if (!id) throw new Error("no pane is on screen");
    return `.pane-stack:has([data-blockid="${id}"])`;
}

async function openHamburger(session) {
    await session.clickSelector(".hamburger-btn");
    await session.wait(300);
}

/** Opens the hamburger menu's `label` submenu, by hovering its row. */
async function openHamburgerSubmenu(session, label) {
    await openHamburger(session);
    await session.hoverText(MENU, label);
    await session.wait(500);
}

/** Opens the status bar item `trigger` and waits for its panel. */
async function openStatusPanel(session, trigger) {
    const pt = await session.evaluate(visibleCenter(trigger));
    if (!pt) throw new Error(`${trigger} isn't on the status bar`);
    await session.clickAt(pt);
    await session.wait(500);
}

/** A shot with the suite's shared setup and cleanup around its own. */
function shot(def) {
    return {
        containsWorkspaceData: "review",
        retries: 1,
        ...def,
        prep: async (session) => {
            await session.setViewport(VIEWPORT);
            await dismiss(session);
            if (def.prep) await def.prep(session);
        },
        cleanup: async (session) => {
            if (def.cleanup) await def.cleanup(session);
            await dismiss(session);
            await session.clearViewport();
        },
    };
}

/** A shot taken in a new window tab, closed again afterwards. `inTab(session)`
 *  runs once the tab is active and settled. */
function tabShot(def) {
    const tab = { tabId: null };
    return shot({
        ...def,
        prep: async (session) => {
            tab.tabId = null;
            await openNewTab(session, tab);
            await def.inTab(session);
        },
        cleanup: async (session) => {
            await session.pressKey("Escape");
            if (tab.tabId) await closeTab(session, tab.tabId);
            tab.tabId = null;
        },
    });
}

/** Maximizes `pane` and waits until it fills the tab. */
async function maximize(session, pane) {
    const fills = `(() => { const r = document.querySelector(${JSON.stringify(pane)}).getBoundingClientRect(); return r.width > window.innerWidth * 0.95; })()`;
    for (let attempt = 0; attempt < 2; attempt++) {
        await session.clickSelector(`${pane} .block-frame-magnify[title="Maximize"]`);
        for (let i = 0; i < 12; i++) {
            await session.wait(250);
            if (await session.evaluate(fills)) return;
        }
    }
    throw new Error("the pane didn't maximize");
}

/** Opens Settings (hamburger → Settings) in the current tab and maximizes its
 *  pane; returns the pane's selector. */
async function openSettings(session) {
    await openHamburger(session);
    await session.clickText(MENU, "Settings");
    let pane = null;
    for (let i = 0; i < 20 && !pane; i++) {
        await session.wait(250);
        pane = await session.evaluate(`(() => {
            const el = [...document.querySelectorAll(${JSON.stringify(SETTINGS)})].find((e) => {
                const r = e.getBoundingClientRect();
                if (r.width === 0 || r.height === 0) return false;
                const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
                return hit && e.contains(hit);
            });
            const id = el?.closest('.pane-stack')?.querySelector('[data-blockid]')?.getAttribute('data-blockid');
            return id ? '.pane-stack:has([data-blockid="' + id + '"])' : null;
        })()`);
    }
    if (!pane) throw new Error("Settings didn't open");
    await maximize(session, pane);
    return pane;
}

/** Selects the Settings tab `label` (by its aria-label, which stays when a
 *  narrow pane shows only icons). */
async function selectSettingsTab(session, label) {
    await session.clickSelector(`${SETTINGS} button.ui-tab[aria-label=${JSON.stringify(label)}]`);
    await session.wait(400);
}

const settingsTabSelected = (label) => async (session) =>
    (await session.evaluate(
        `document.querySelector(${JSON.stringify(`${SETTINGS} button.ui-tab[aria-label=${JSON.stringify(label)}]`)})?.getAttribute('aria-selected') === 'true'`
    )) || `the ${label} tab isn't selected`;

/** The Settings tabs, as labelled (settings-model.ts), and each shot's id. */
export const SETTINGS_TABS = [
    ["Appearance", "appearance"],
    ["Window & Panes", "window-panes"],
    ["Browser", "browser"],
    ["Terminal", "terminal"],
    ["Sounds", "sounds"],
    ["Notifications & Tray", "notifications-tray"],
    ["Recording", "recording"],
    ["Paired devices", "paired-devices"],
    ["Widgets", "widgets"],
    ["Advanced", "advanced"],
];

let paneSelector = null;

export const shots = [
    // ── The window ────────────────────────────────────────────────────────
    shot({
        id: "chrome-window",
        title: "The AgentMux window",
        description: "The whole window: the top bar, a tab's panes, and the status bar.",
    }),
    shot({
        id: "chrome-top-bar",
        title: "Top bar",
        description: "The hamburger menu, the window's tabs, and the widget buttons.",
        selector: ".window-header",
    }),
    shot({
        id: "chrome-more-dropdown",
        title: "The top bar's More list",
        description: "Widgets that aren't pinned to the top bar, under More.",
        prep: async (session) => {
            // It opens on hover, and closes when the pointer leaves, so the
            // pointer stays on the button.
            const pt = await session.evaluate(visibleCenter(".action-widget-more-btn"));
            if (!pt) throw new Error("no visible More button");
            await session.hoverAt(pt);
            await session.wait(500);
        },
        keepPointer: true,
        verify: MORE_DROPDOWN,
        selector: [MORE_DROPDOWN, ".action-widgets:not(.action-widgets--measure) > .action-widget-more-btn"],
        padding: 4,
    }),

    shot({
        id: "chrome-tab-menu",
        title: "A window tab's menu",
        description: "Right-click a window tab to colour it or rename it.",
        prep: async (session) => {
            await session.rightClickSelector(".tab.active");
            await session.wait(400);
        },
        verify: TAB_PANEL,
        selector: [TAB_PANEL, ".tab.active"],
        padding: 4,
    }),
    (() => {
        // Closing a tab with panes in it asks first. The shot's own new tab is
        // the one closed, so Cancel leaves the instance as it was.
        const tab = { tabId: null };
        return shot({
            id: "chrome-close-tab-confirm",
            title: "Close tab?",
            description: "Closing a window tab asks first, since its panes close with it.",
            prep: async (session) => {
                tab.tabId = null;
                await openNewTab(session, tab);
                await session.clickSelector(`.tab[data-tab-id="${tab.tabId}"] [title="Close Tab"]`);
                await session.wait(500);
            },
            verify: ".modal-root .modal-panel",
            selector: ".modal-root .modal-panel",
            cleanup: async (session) => {
                const cancel = await session.centerOfText(".modal-root", "Cancel");
                if (cancel) await session.clickAt(cancel);
                await session.wait(300);
                if (tab.tabId) await closeTab(session, tab.tabId);
                tab.tabId = null;
            },
        });
    })(),

    // ── The hamburger menu ────────────────────────────────────────────────
    shot({
        id: "chrome-hamburger-menu",
        title: "Hamburger menu",
        description: "The main menu, from the button at the top left.",
        prep: openHamburger,
        verify: MENU,
        selector: MENU,
        padding: 4,
    }),
    ...[
        ["Theme", "theme"],
        ["Opacity", "opacity"],
        ["Layouts", "layouts"],
    ].map(([label, key]) =>
        shot({
            id: `chrome-hamburger-${key}`,
            title: `Hamburger menu — ${label}`,
            description: `The main menu with its ${label} submenu open.`,
            prep: (session) => openHamburgerSubmenu(session, label),
            // A submenu closes when the pointer leaves its row.
            keepPointer: true,
            verify: [MENU, SUBMENU],
            selector: [MENU, SUBMENU],
            padding: 4,
        })
    ),

    // ── The command palette ───────────────────────────────────────────────
    shot({
        id: "chrome-command-palette",
        title: "Command palette",
        description: "The command palette (Ctrl+Shift+P), before typing.",
        prep: async (session) => {
            await session.pressKey("P", { ctrl: true, shift: true });
            await session.wait(400);
        },
        verify: PALETTE,
        selector: PALETTE,
    }),
    shot({
        id: "chrome-command-palette-filtered",
        title: "Command palette, filtered",
        description: "The command palette narrowed to the commands that match what was typed.",
        prep: async (session) => {
            await session.pressKey("P", { ctrl: true, shift: true });
            await session.wait(400);
            await session.typeText("split");
            await session.wait(400);
        },
        verify: [PALETTE, ".command-palette-item"],
        selector: PALETTE,
    }),

    // ── Pane menus ────────────────────────────────────────────────────────
    shot({
        id: "chrome-pane-add-menu",
        title: "A pane's + menu",
        description: "What a pane's + button adds: a new tab of each pane type, in this pane.",
        prep: async (session) => {
            const pane = await firstVisiblePane(session);
            await session.clickSelector(`${pane} .pane-tab-strip-add`);
            await session.wait(400);
        },
        verify: CONTEXT_MENU,
        selector: CONTEXT_MENU,
        padding: 4,
    }),
    shot({
        id: "chrome-pane-context-menu",
        title: "A pane's right-click menu",
        description: "Right-click a pane's header: split, magnify, close, and the pane's colour.",
        prep: async (session) => {
            const pane = await firstVisiblePane(session);
            await session.rightClickSelector(`${pane} .block-frame-default-header`);
            await session.wait(400);
        },
        verify: CONTEXT_MENU,
        selector: CONTEXT_MENU,
        padding: 4,
    }),
    shot({
        id: "chrome-pane-color-submenu",
        title: "Pane Color",
        description: "A pane's right-click menu with its Pane Color submenu open.",
        prep: async (session) => {
            const pane = await firstVisiblePane(session);
            await session.rightClickSelector(`${pane} .block-frame-default-header`);
            await session.wait(400);
            await session.hoverText(CONTEXT_MENU, "Pane Color");
            await session.wait(500);
        },
        keepPointer: true,
        verify: [CONTEXT_MENU, CONTEXT_SUBMENU],
        selector: [CONTEXT_MENU, CONTEXT_SUBMENU],
        padding: 4,
    }),
    tabShot({
        id: "chrome-split-layout",
        title: "A split pane",
        description: "A pane split in two with Split Right, from its right-click menu: the new pane opens beside it.",
        inTab: async (session) => {
            const before = (await session.evaluate(VISIBLE_BLOCK_IDS)).split(",").filter(Boolean).length;
            const pane = await firstVisiblePane(session);
            await session.rightClickSelector(`${pane} .block-frame-default-header`);
            await session.wait(400);
            await session.clickText(CONTEXT_MENU, "Split Right");
            for (let i = 0; i < 12; i++) {
                await session.wait(250);
                const now = (await session.evaluate(VISIBLE_BLOCK_IDS)).split(",").filter(Boolean).length;
                if (now > before) return;
            }
            throw new Error("Split Right didn't add a pane");
        },
        verify: async (session) =>
            (await session.evaluate(VISIBLE_BLOCK_IDS)).split(",").filter(Boolean).length >= 2 || "the tab doesn't show two panes",
    }),
    // In the first tab, which has several panes: in a tab of one pane, a
    // maximized pane looks like any other.
    shot({
        id: "chrome-pane-maximized",
        title: "A maximized pane",
        description: "A pane maximized to fill its tab, the others behind it; Restore puts it back.",
        prep: async (session) => {
            // The agent pane: an empty Swarm maximized shows little.
            const agent = await session.evaluate(`(() => {
                const el = [...document.querySelectorAll('.pane-stack:has(.agent-view)')].find((p) => {
                    const r = p.getBoundingClientRect();
                    if (r.width === 0 || r.height === 0) return false;
                    const hit = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
                    return hit && p.contains(hit);
                });
                const id = el?.querySelector('[data-blockid]')?.getAttribute('data-blockid');
                return id ? '.pane-stack:has([data-blockid="' + id + '"])' : null;
            })()`);
            await maximize(session, agent ?? (await firstVisiblePane(session)));
        },
        verify: ".block.magnified",
        cleanup: async (session) => {
            // The Restore button on screen (it isn't inside `.block.magnified`,
            // and a hidden tab can hold another magnified block).
            const RESTORE = '.block-frame-magnify[title="Restore"]';
            const restore = await session.evaluate(visibleCenter(RESTORE));
            if (restore) await session.clickAt(restore);
            await session.wait(500);
            if (await session.evaluate(visibleCenter(RESTORE))) throw new Error("the maximized pane didn't restore");
        },
    }),

    // ── The status bar and its panels ─────────────────────────────────────
    shot({
        id: "chrome-status-bar",
        title: "Status bar",
        description: "The status bar along the bottom of the window.",
        selector: ".status-bar",
    }),
    ...[
        ["backend", "Backend status", "The backend's state and how long it has been up.", ".status-bar-item.clickable:has(.stat-uptime)", '.status-bar-popover[aria-label="Backend status"]'],
        ["cpu", "CPU cores", "The load on each CPU core.", "button.stat-cpu-button", ".cpu-cores-popover"],
        ["disk", "Disk volumes", "Each disk volume's size and how full it is.", "button.stat-disk-button", ".disk-volumes-popover"],
        ["host", "Host", "This computer: its OS, address, data folder and connections.", '.status-bar-item[aria-label="Host info"]', ".host-popover"],
        ["tokens", "Token usage", "Tokens your agents have sent and received.", "button.token-usage-indicator", ".token-usage-breakdown"],
        ["instance", "Instance panel", "This AgentMux instance, and the others running on this computer.", "button.status-version", ".instance-panel"],
    ].map(([key, title, description, trigger, panel]) =>
        shot({
            id: `chrome-status-${key}`,
            title: `Status bar — ${title}`,
            description,
            prep: (session) => openStatusPanel(session, trigger),
            verify: panel,
            // The panel with the item that opened it, so it's clear where it opens from.
            selector: [panel, trigger],
            padding: 4,
        })
    ),

    // ── Settings ──────────────────────────────────────────────────────────
    ...SETTINGS_TABS.map(([label, key]) =>
        tabShot({
            id: `settings-${key}`,
            title: `Settings — ${label}`,
            description: `The Settings pane's ${label} tab.`,
            inTab: async (session) => {
                paneSelector = await openSettings(session);
                await selectSettingsTab(session, label);
            },
            verify: settingsTabSelected(label),
            selector: () => paneSelector,
        })
    ),
    tabShot({
        id: "settings-search",
        title: "Settings — search",
        description: "Searching Settings: every setting that matches, from any tab.",
        inTab: async (session) => {
            paneSelector = await openSettings(session);
            await session.clickSelector(`${SETTINGS} input.settings-search-input`);
            await session.typeText("font");
            await session.wait(500);
        },
        verify: ".settings-search-result",
        selector: () => paneSelector,
    }),
];
