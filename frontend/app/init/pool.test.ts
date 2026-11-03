// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A pre-warmed window says so while it waits to be claimed, and stops saying
// so once promoted (docs/specs/PLAN_SHORTCUT_KINKS_2026_10_10.md, A5).

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ handlers: new Map<string, (payload: unknown) => void>() }));
vi.mock("@/store/global", () => ({
    getApi: () => ({
        listen: async (event: string, cb: (payload: unknown) => void) => {
            h.handlers.set(event, cb);
            return () => {};
        },
        getWindowLabel: async () => "window-pool-1",
        windows: { poolWindowReady: async () => {}, panePoolWindowReady: async () => {} },
        setWindowInitStatus: () => {},
    }),
}));
vi.mock("./startup-splash", () => ({ markPoolPromoted: vi.fn(), showTearOffSnapshot: vi.fn() }));

import { awaitPanePoolPromote, awaitPoolPromote, markPoolWaiting } from "./pool";

beforeEach(() => {
    h.handlers.clear();
    document.title = "";
    delete document.documentElement.dataset.poolWindow;
});

describe("pool window marker", () => {
    it("names a waiting pool window in its title and a data attribute", () => {
        markPoolWaiting("window");
        expect(document.title).toBe("AgentMux (pre-warmed window)");
        expect(document.documentElement.dataset.poolWindow).toBe("window");
        markPoolWaiting("pane");
        expect(document.title).toContain("pane window");
    });

    it.each(["pool:promote", "pool:new-window"])("clears the marker on %s", async (event) => {
        markPoolWaiting("window");
        const done = awaitPoolPromote();
        await vi.waitFor(() => expect(h.handlers.has(event)).toBe(true));
        h.handlers.get(event)!({ workspaceId: "ws1" });
        await done;
        expect(document.documentElement.dataset.poolWindow).toBeUndefined();
        expect(document.title).toBe("AgentMux");
    });

    it("clears the marker on pool:pane-promote", async () => {
        markPoolWaiting("pane");
        const done = awaitPanePoolPromote();
        await vi.waitFor(() => expect(h.handlers.has("pool:pane-promote")).toBe(true));
        h.handlers.get("pool:pane-promote")!({ paneId: "p1", workspaceId: "ws1" });
        await done;
        expect(document.documentElement.dataset.poolWindow).toBeUndefined();
    });
});

// The splash's pulse stops while a pool window waits, hidden, and runs again
// once it is promoted: a running pulse in three hidden pool windows kept the
// GPU process at about 15% of a core on Windows (Posa, 2026-10-19).
describe("the splash pulse in a waiting pool window", () => {
    const html = readFileSync(join(process.cwd(), "index.html"), "utf8");
    const css = /<style>([\s\S]*?)<\/style>/.exec(html)![1];

    function pulse(): string {
        document.head.innerHTML = `<style>${css}</style>`;
        document.body.innerHTML = '<div id="startup-loading"><svg></svg></div>';
        return getComputedStyle(document.querySelector("#startup-loading svg")!).animationPlayState;
    }

    it("is paused while the window waits, and runs once it is promoted", async () => {
        markPoolWaiting("window");
        expect(pulse()).toBe("paused");
        const done = awaitPoolPromote();
        await vi.waitFor(() => expect(h.handlers.has("pool:promote")).toBe(true));
        h.handlers.get("pool:promote")!({ workspaceId: "ws1" });
        await done;
        expect(pulse()).not.toBe("paused");
    });

    it("runs in an ordinary window", () => {
        expect(pulse()).not.toBe("paused");
    });
});

// The pulse above is one animation. Chromium counts a hidden pool window as visible (native occlusion
// tracking is off, CEF #3638), so any other animation the waiting page runs costs the same again. This
// walks the whole page a pool window shows while it waits, so a new one fails here, not on a user's
// fans (docs/analysis/RCA_IDLE_CPU_FLOOR_PREWARMED_POOL_WINDOWS_2026_10_21.md §6).
describe("a waiting pool window runs no animation", () => {
    const html = readFileSync(join(process.cwd(), "index.html"), "utf8");
    const styles = [...html.matchAll(/<style>([\s\S]*?)<\/style>/g)].map((m) => m[1]).join("\n");
    const body = /<body[^>]*>([\s\S]*?)<\/body>/.exec(html)![1].replace(/<script[\s\S]*?<\/script>/g, "");

    function load(): void {
        document.head.innerHTML = `<style>${styles}</style>`;
        document.body.innerHTML = body;
    }

    /** Every element of the page that a stylesheet rule gives an animation, with its play state. */
    function animated(): { selector: string; state: string }[] {
        const found: { selector: string; state: string }[] = [];
        for (const sheet of Array.from(document.styleSheets)) {
            for (const rule of Array.from(sheet.cssRules) as CSSStyleRule[]) {
                const style = rule.style;
                if (!style) continue;
                const name = style.getPropertyValue("animation-name") || style.getPropertyValue("animation");
                if (!name || /^\s*none\b/.test(name)) continue;
                for (const el of Array.from(document.querySelectorAll(rule.selectorText))) {
                    found.push({ selector: rule.selectorText, state: getComputedStyle(el).animationPlayState });
                }
            }
        }
        return found;
    }

    it("has an animated element, so the walk below is not vacuous", () => {
        delete document.documentElement.dataset.poolWindow;
        load();
        expect(animated().length).toBeGreaterThan(0);
    });

    it("has every CSS animation paused", () => {
        markPoolWaiting("window");
        load();
        const running = animated().filter((a) => a.state !== "paused");
        expect(running, "running while a pool window waits").toEqual([]);
    });

    it("has no SMIL animation in the markup, and none running where the environment reports them", () => {
        markPoolWaiting("window");
        load();
        expect(document.querySelectorAll("animate, animateTransform, animateMotion, set").length).toBe(0);
        expect(document.getAnimations?.().filter((a) => a.playState === "running") ?? []).toEqual([]);
    });
});
