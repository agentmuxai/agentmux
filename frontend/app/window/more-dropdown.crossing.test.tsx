// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pointer crossing the More dropdown toward a parent row's flyout (task #27346): scripted at the pace and step of a
 * real device, with the enter and leave events in the order a browser sends them, over many start points, speeds and
 * both sides the flyout can open on. The dropdown and its flyout must stay open all the way in.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// jsdom can't lay out a floating panel; only the placing is stubbed.
vi.mock("@floating-ui/dom", () => ({ autoUpdate: vi.fn(() => vi.fn()) }));
vi.mock("@/app/platform/pane-overlay", () => ({ useHiddenTabMarker: () => () => undefined, usePaneOverlay: vi.fn() }));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({ style: { position: "fixed", left: "0px", top: "0px" } })),
    assertMenuInPaintableArea: vi.fn(),
}));

import { createPeerRegistry, createSubmenuHover } from "@/app/util/submenu-hover";
import { MoreDropdown } from "./more-dropdown";

const widget = (label: string, extra: object = {}) =>
    ({ label, icon: "window-maximize", blockdef: { meta: { view: "term" } }, ...extra }) as unknown as WidgetConfigType;

const ROW_H = 36;
type Box = { left: number; top: number; right: number; bottom: number };
const box = (left: number, top: number, right: number, bottom: number): Box => ({ left, top, right, bottom });
const asRect = (b: Box) =>
    ({ ...b, x: b.left, y: b.top, width: b.right - b.left, height: b.bottom - b.top, toJSON: () => b }) as DOMRect;
const inside = (b: Box, x: number, y: number) => x >= b.left && x < b.right && y >= b.top && y < b.bottom;

/** The dropdown and its flyout side by side; `side` is where the flyout opens (it flips left near the window edge). */
function layout(side: "right" | "left") {
    const dropdown = box(600, 40, 780, 40 + 4 * ROW_H);
    const flyout =
        side === "right" ? box(788, 40, 900, 40 + 5 * ROW_H) : box(600 - 8 - 112, 40, 600 - 8, 40 + 5 * ROW_H);
    return { dropdown, flyout };
}

function mountBar(side: "right" | "left") {
    const closed = vi.fn();
    const peers = createPeerRegistry();
    const moreHover = createSubmenuHover({ onOpen: vi.fn(), onClose: closed });
    peers.register("more", moreHover);
    const g = layout(side);
    render(() => (
        <MoreDropdown
            widgets={() => [
                { key: "msgr", widget: widget("Messengers", { children: ["discord", "slack", "teams"] }) },
                { key: "sys", widget: widget("Sysinfo") },
                { key: "drone", widget: widget("Drone") },
                { key: "vault", widget: widget("Vault") },
            ]}
            onClose={vi.fn()}
            onItemContextMenu={vi.fn()}
            anchor={() => document.createElement("div")}
            settings={() => ({})}
            wmap={() => ({ discord: widget("Discord"), slack: widget("Slack"), teams: widget("Teams") })}
            onSubmenuEnter={() => moreHover.onSubmenuEnter()}
            onSubmenuLeave={(e) => moreHover.onSubmenuLeave(e)}
            setSubmenuEl={(el) => moreHover.setSubmenuEl(el)}
        />
    ));
    moreHover.openNow();
    const dropdown = document.querySelector(
        ".action-widget-more-dropdown:not(.action-widget-pinned-flyout)"
    ) as HTMLElement;
    dropdown.getBoundingClientRect = () => asRect(g.dropdown);
    const rows = [...dropdown.querySelectorAll<HTMLElement>(":scope > .action-widget-more-item")];
    rows.forEach((row, i) => {
        row.getBoundingClientRect = () => asRect(box(600, 40 + i * ROW_H, 780, 40 + (i + 1) * ROW_H));
    });
    return { dropdown, rows, closed, moreHover, g };
}

/** What a browser does as the pointer moves: leave events, then enter events, then the move, each at the new point. */
function pointerDriver(hit: (x: number, y: number) => HTMLElement | null) {
    let chain: HTMLElement[] = [];
    const chainOf = (el: HTMLElement | null) => {
        const out: HTMLElement[] = [];
        for (let n: HTMLElement | null = el; n; n = n.parentElement) out.unshift(n);
        return out;
    };
    return (x: number, y: number) => {
        const next = chainOf(hit(x, y));
        const init = { clientX: x, clientY: y };
        for (const el of [...chain].reverse()) {
            if (!next.includes(el)) el.dispatchEvent(new MouseEvent("mouseleave", init));
        }
        for (const el of next) {
            if (!chain.includes(el)) el.dispatchEvent(new MouseEvent("mouseenter", init));
        }
        chain = next;
        document.dispatchEvent(new MouseEvent("mousemove", init));
    };
}

/** A tiny seeded generator so a failing path can be replayed. */
function rng(seed: number) {
    let s = seed >>> 0;
    return () => (s = (Math.imul(s, 1664525) + 1013904223) >>> 0) / 2 ** 32;
}

function crossing(side: "right" | "left", durationMs: number, seed: number): string {
    const rand = rng(seed);
    const bar = mountBar(side);
    const flyoutEl = () => document.querySelector<HTMLElement>(".action-widget-pinned-flyout");
    const move = pointerDriver((x, y) => {
        const fly = flyoutEl();
        if (fly && inside(bar.g.flyout, x, y)) return fly;
        if (!inside(bar.g.dropdown, x, y)) return null;
        const i = Math.floor((y - 40) / ROW_H);
        return bar.rows[i] ?? bar.dropdown;
    });
    // Settle on the Messengers row until its flyout is open, then turn the flyout's rect on.
    const startX = 620 + Math.floor(rand() * 140);
    const startY = 40 + 8 + Math.floor(rand() * 20);
    move(startX, startY);
    vi.advanceTimersByTime(200);
    const fly = flyoutEl();
    expect(fly).not.toBeNull();
    fly!.getBoundingClientRect = () => asRect(bar.g.flyout);
    // Diagonal down across Sysinfo, Drone and Vault into the flyout, 10 ms steps, integer positions.
    const endX = side === "right" ? 800 + Math.floor(rand() * 80) : 500 + Math.floor(rand() * 80);
    const endY = startY + 110;
    const steps = Math.round(durationMs / 10);
    let failure = "";
    for (let i = 1; i <= steps; i++) {
        vi.advanceTimersByTime(10);
        const x = Math.round(startX + ((endX - startX) * i) / steps);
        const y = Math.round(startY + ((endY - startY) * i) / steps);
        move(x, y);
        if (failure) continue;
        if (bar.closed.mock.calls.length > 0) failure = `dropdown closed at ${i * 10} ms, pointer ${x},${y}`;
        else if (!flyoutEl()) failure = `flyout closed at ${i * 10} ms, pointer ${x},${y}`;
    }
    vi.advanceTimersByTime(1000);
    if (!failure && (bar.closed.mock.calls.length > 0 || !flyoutEl())) failure = "closed after arriving";
    cleanup();
    return failure;
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("crossing the More dropdown toward a row's flyout", () => {
    for (const side of ["right", "left"] as const) {
        for (const ms of [300, 700, 1000, 1500, 2000]) {
            it(`${side}: a ${ms} ms diagonal keeps the dropdown and the flyout open (100 start points)`, () => {
                const failures: string[] = [];
                for (let seed = 1; seed <= 100; seed++) {
                    const failure = crossing(side, ms, seed);
                    if (failure) failures.push(`seed ${seed}: ${failure}`);
                }
                expect(failures).toEqual([]);
            });
        }
    }
});
