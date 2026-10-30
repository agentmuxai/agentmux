// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The structure browser-view.scss's two layouts depend on: one grid holding the
 * navigation buttons, the address bar and Profile, in a bar that follows chrome
 * zoom. There is no Go button; Enter submits.
 * jsdom does no layout, container queries or `zoom`; the layouts themselves are
 * checked live (SPEC_BROWSER_PANE_NAV_BAR_RESPONSIVE_AND_CHROME_ZOOM_2026_10_14.md §6).
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

import { isOverChrome } from "@/app/store/chrome-zoom-target";
import { makeTestHostApi } from "@/app/host/test-host";
import { BrowserNavBar } from "./browser-nav-bar";
import type { BrowserViewModel } from "./browser-model";

afterEach(() => cleanup());

function renderBar(opts: { loading?: () => boolean; reload?: () => Promise<void>; stop?: () => Promise<void> } = {}) {
    window.api = makeTestHostApi({
        browserPanes: {
            reload: opts.reload ?? (() => Promise.resolve()),
            stop: opts.stop ?? (() => Promise.resolve()),
        } as unknown as AppApi["browserPanes"],
    });
    const model = {
        loadingAtom: opts.loading ?? (() => false),
        blockId: "block-1",
        urlAtom: () => "https://example.com/",
        titleAtom: () => "Example",
        faviconUrlAtom: () => "",
        showControlsAtom: () => true,
        canGoBackAtom: () => false,
        canGoForwardAtom: () => false,
        meta: () => ({}),
        navigate: () => {},
        goBack: () => {},
        goForward: () => {},
        onError: () => {},
    } as unknown as BrowserViewModel;
    const { container } = render(() => (
        <BrowserNavBar
            model={model}
            windowLabel="main"
            diag={() => {}}
            paneCreated={() => false}
            createPane={async () => {}}
        />
    ));
    return container.querySelector<HTMLElement>(".browser-nav-bar")!;
}

const titles = (el: Element) => [...el.querySelectorAll<HTMLElement>(":scope > *")].map((c) => c.getAttribute("title") ?? c.getAttribute("aria-label") ?? c.className);

describe("BrowserNavBar layout structure", () => {
    it("is a chrome-zoom scope, so Ctrl+wheel over it zooms chrome", () => {
        const bar = renderBar();
        expect(bar.dataset.zoomScope).toBe("chrome");
        expect(isOverChrome(bar.querySelector<HTMLElement>(".browser-address-bar")!)).toBe(true);
    });

    it("holds the navigation buttons, Bookmarks, the thin layout's line, the address bar and Profile in one grid, with no Go button", () => {
        const bar = renderBar();
        const grid = bar.querySelector(":scope > .browser-nav-bar-grid")!;
        expect(grid).toBeTruthy();
        const children = [...grid.children].map((c) => c.classList[c.classList.length - 1]);
        // In this order, which is also the Tab order in both layouts.
        expect(children).toEqual([
            "browser-nav-group--nav",
            "browser-nav-group--bookmarks",
            "browser-nav-break",
            "browser-address-wrap",
            "browser-nav-group--profile",
        ]);
        expect(grid.querySelector(".browser-nav-break")!.getAttribute("aria-hidden")).toBe("true");
        expect(bar.querySelector('[title="Go"]')).toBeNull();
        expect(grid.querySelector(".browser-address-wrap > input.browser-address-bar")).toBeTruthy();
    });

    it("puts Back, Forward, Reload and Home in the nav group, Bookmarks in its own cell and Profile alone on the right", () => {
        const bar = renderBar();
        const nav = bar.querySelector(".browser-nav-group--nav")!;
        expect(titles(nav).slice(0, 3)).toEqual(["Back", "Forward", "Reload"]);
        expect(nav.children).toHaveLength(4); // and Home
        expect(bar.querySelector('.browser-nav-group--bookmarks [title="Bookmarks"]')).toBeTruthy();
        const profile = bar.querySelector(".browser-nav-group--profile")!;
        expect(profile.children).toHaveLength(1);
        expect(profile.querySelector(".browser-profile-btn")).toBeTruthy();
    });
});

describe("the Reload / Stop button", () => {
    const button = (bar: Element) => bar.querySelector<HTMLButtonElement>('[data-testid="browser-reload-stop"]')!;

    it("is Reload while the page is idle, and a press reloads", () => {
        const reload = vi.fn(() => Promise.resolve());
        const stop = vi.fn(() => Promise.resolve());
        const bar = renderBar({ reload, stop });
        const b = button(bar);
        expect(b.textContent).toBe("↻");
        expect(b.getAttribute("title")).toBe("Reload");
        expect(b.getAttribute("aria-label")).toBe("Reload");
        fireEvent.click(b);
        expect(reload).toHaveBeenCalledWith("block-1");
        expect(stop).not.toHaveBeenCalled();
    });

    it("is Stop while the page loads, and a press stops the load instead of reloading", () => {
        const reload = vi.fn(() => Promise.resolve());
        const stop = vi.fn(() => Promise.resolve());
        const bar = renderBar({ loading: () => true, reload, stop });
        const b = button(bar);
        expect(b.textContent).toBe("✕");
        expect(b.getAttribute("title")).toBe("Stop loading");
        expect(b.getAttribute("aria-label")).toBe("Stop loading");
        fireEvent.click(b);
        expect(stop).toHaveBeenCalledWith("block-1");
        expect(reload).not.toHaveBeenCalled();
    });

    it("goes back to Reload, with its tooltip and name, when the load ends", () => {
        const [loading, setLoading] = createSignal(true);
        const bar = renderBar({ loading });
        const b = button(bar);
        expect(b.getAttribute("aria-label")).toBe("Stop loading");
        setLoading(false);
        expect(b.textContent).toBe("↻");
        expect(b.getAttribute("title")).toBe("Reload");
        expect(b.getAttribute("aria-label")).toBe("Reload");
    });
});

describe("the Home and Bookmarks icons", () => {
    it("are outlines, not Font Awesome's solid glyphs", () => {
        const bar = renderBar();
        const home = bar.querySelector('[data-testid="browser-home"]')!;
        const bookmarks = bar.querySelector('.browser-nav-group--bookmarks button[title="Bookmarks"]')!;
        for (const el of [home, bookmarks]) {
            const svg = el.querySelector("svg")!;
            expect(svg.getAttribute("fill")).toBe("none");
            expect(svg.getAttribute("stroke")).toBe("currentColor");
            expect(el.querySelector(".fa-solid")).toBeNull();
        }
    });

    it("keep Home's tooltip and accessible name", () => {
        const home = renderBar().querySelector('[data-testid="browser-home"]')!;
        expect(home.getAttribute("title")).toMatch(/^Home: /);
        expect(home.getAttribute("aria-label")).toMatch(/^Home: /);
    });
});

describe("the toolbar's grid areas", () => {
    const scss = readFileSync(resolve(process.cwd(), "frontend/app/view/browser/browser-view.scss"), "utf8");

    it("keeps Bookmarks just left of Profile, at the right end, in the wide row and the thin one", () => {
        const wide = scss.match(/grid-template-areas:\s*"([^"]+)";/)![1].split(/\s+/);
        expect(wide).toEqual(["nav", "address", "bookmarks", "profile"]);
        const thin = scss.match(/@container[^{]*\{[\s\S]*?grid-template-areas:\s*"([^"]+)"/)![1].split(/\s+/);
        expect(thin.slice(-2)).toEqual(["bookmarks", "profile"]);
    });
});

describe("isOverChrome", () => {
    it.each([
        ["window-header", '<div class="window-header"><span></span></div>'],
        ["status-bar", '<div class="status-bar"><span></span></div>'],
        ["pane header", '<div class="block-frame-default-header"><span></span></div>'],
        ["chrome zoom scope", '<div data-zoom-scope="chrome"><span></span></div>'],
    ])("is true inside the %s", (_name, html) => {
        const host = document.createElement("div");
        host.innerHTML = html;
        document.body.append(host);
        expect(isOverChrome(host.querySelector("span")!)).toBe(true);
        host.remove();
    });

    it("is false in pane content", () => {
        const el = document.createElement("div");
        el.dataset.blockid = "b";
        document.body.append(el);
        expect(isOverChrome(el)).toBe(false);
        el.remove();
    });
});
