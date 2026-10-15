// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The structure browser-view.scss's two layouts depend on: one grid holding the
 * navigation buttons, the address bar and Profile, in a bar that follows chrome
 * zoom. There is no Go button; Enter submits.
 * jsdom does no layout, container queries or `zoom`; the layouts themselves are
 * checked live (SPEC_BROWSER_PANE_NAV_BAR_RESPONSIVE_AND_CHROME_ZOOM_2026_10_14.md §6).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";

import { isOverChrome } from "@/app/store/chrome-zoom-target";
import { makeTestHostApi } from "@/app/host/test-host";
import { BrowserNavBar } from "./browser-nav-bar";
import type { BrowserViewModel } from "./browser-model";

afterEach(() => cleanup());

function renderBar() {
    window.api = makeTestHostApi();
    const model = {
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

    it("holds the navigation buttons, the address bar and Profile in one grid, with no Go button", () => {
        const bar = renderBar();
        const grid = bar.querySelector(":scope > .browser-nav-bar-grid")!;
        expect(grid).toBeTruthy();
        const children = [...grid.children].map((c) => c.classList[c.classList.length - 1]);
        expect(children).toEqual(["browser-nav-group--nav", "browser-address-wrap", "browser-nav-group--profile"]);
        expect(bar.querySelector('[title="Go"]')).toBeNull();
        expect(grid.querySelector(".browser-address-wrap > input.browser-address-bar")).toBeTruthy();
    });

    it("puts Back, Forward, Reload, Home and Bookmarks on the left and Profile alone on the right", () => {
        const bar = renderBar();
        const nav = bar.querySelector(".browser-nav-group--nav")!;
        expect(titles(nav).slice(0, 3)).toEqual(["Back", "Forward", "Reload"]);
        expect(nav.children).toHaveLength(5); // and Home, Bookmarks
        expect(nav.querySelector('[title="Bookmarks"]')).toBeTruthy();
        const profile = bar.querySelector(".browser-nav-group--profile")!;
        expect(profile.children).toHaveLength(1);
        expect(profile.querySelector(".browser-profile-btn")).toBeTruthy();
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
