// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Declarative manifest of screenshots to capture for the UI manual — see
// docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md. Edit this file
// to add/remove/change shots; capture.mjs shouldn't need to change.
//
// Each shot: { id, title, description, selector?, padding?, prep?, settleMs? }
//   - id: stable, filesystem-safe, used in the output filename and manifest.json.
//   - selector: CSS selector to crop to; omitted = whole window.
//   - padding: extra px around the selector's bounding box (default 0).
//   - prep(session): optional async setup (clicks/waits) before capture.
//     `session` is the CdpSession from capture.mjs (evaluate/clickSelector/
//     clickText/wait/send all available).
//   - settleMs: wait after `prep` before measuring/capturing (default 400).
//
// Selectors confirmed live against a running instance's DOM (2026-09-19) —
// see the spec's §2 for why these are resolved fresh at capture time
// instead of hand-maintained pixel coordinates.

const FOCUSED_PANE = ".pane-stack.pane-stack-focused, .pane-stack.pane-stack-focused-alone";

async function openWidget(session, label) {
    await session.clickText(".action-widgets", label);
}

export const shots = [
    {
        id: "main-window-overview",
        title: "Main window — default layout",
        description: "The whole AgentMux window as it looks on first launch, before opening anything extra.",
    },
    {
        id: "top-tab-bar",
        title: "Top tab bar",
        description: "Hamburger menu, window tab strip, and the pinned-widget button row.",
        selector: ".window-header",
    },
    {
        id: "hamburger-menu-open",
        title: "Hamburger menu, open",
        description: "The dropdown menu reached from the hamburger button.",
        selector: ".menu",
        prep: async (session) => {
            await session.clickSelector(".hamburger-btn");
        },
    },
    {
        id: "agent-picker",
        title: "Agent picker",
        description: "The \"pick an agent to launch\" screen shown by an agent pane before anything is running.",
        selector: ".agent-picker",
    },
    {
        id: "pane-header-tabstrip",
        title: "Pane header / tab strip",
        description: "Close-up of a single pane's header row, showing the universal pane-tab strip.",
        selector: ".block-frame-default-header",
    },
    {
        id: "swarm-widget",
        title: "Swarm widget",
        description: "The Swarm widget's pane, opened from the top bar.",
        selector: `.pane-stack:has(.swarm-view)`,
        prep: async (session) => {
            await openWidget(session, "Swarm");
        },
    },
    {
        id: "sysinfo-widget",
        title: "Sysinfo widget",
        description: "The Sysinfo widget's pane (CPU/memory graphs), opened from the top bar.",
        selector: FOCUSED_PANE,
        prep: async (session) => {
            await openWidget(session, "Sysinfo");
        },
    },
    {
        id: "armory-bundles",
        title: "Armory — Bundles",
        description: "The Armory widget's Bundles tab.",
        selector: `.pane-stack:has(.armory-view)`,
        prep: async (session) => {
            await openWidget(session, "Armory");
            await session.wait(300);
            await session.clickText(".bundle-manager-tab-bar", "Bundles");
        },
    },
    {
        id: "settings-appearance",
        title: "Settings — Appearance",
        description: "The Settings pane, Appearance tab, reached from the hamburger menu.",
        selector: ".settings-view-container",
        prep: async (session) => {
            await session.clickSelector(".hamburger-btn");
            await session.wait(300);
            await session.clickText(".menu", "Settings");
            await session.wait(500);
            await session.clickText(null, "Appearance");
        },
    },
    {
        id: "terminal-pane-fresh",
        title: "Fresh terminal pane",
        description: "A newly-opened, empty terminal pane.",
        selector: ".view-term",
        prep: async (session) => {
            // Terminal isn't pinned by default — reachable via the "More"
            // dropdown in the top bar (action-widgets-config.ts), not
            // directly inside .action-widgets like the four pinned widgets.
            await session.clickText(".window-header", "more");
            await session.wait(300);
            await session.clickText(null, "Terminal");
        },
    },
];
