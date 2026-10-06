// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A click on a link opens the system browser; a middle-click opens a new
// browser pane. Neither may leave the browser's default in place: for a
// middle-click that default loads the page in the app's own window.

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const openLink = vi.fn();
const createBlock = vi.fn(() => Promise.resolve("block-1"));
const listeners = new Map<string, (payload: unknown) => void>();
vi.mock("@/app/store/global", () => ({
    openLink: (...a: unknown[]) => openLink(...a),
    createBlock: (...a: unknown[]) => createBlock(...a),
    getApi: () => ({
        listen: (name: string, cb: (payload: unknown) => void) => {
            listeners.set(name, cb);
            return Promise.resolve(() => {});
        },
    }),
}));

import { LinkifiedText } from "./linkified-text";
import { onLinkAuxClick, registerLinkInPaneListener } from "./link-open";
import { Markdown } from "./markdown";

const URL = "https://github.com/settings/installations";

afterEach(() => {
    cleanup();
    openLink.mockReset();
    createBlock.mockClear();
    listeners.clear();
});

function fire(el: Element, type: "click" | "auxclick", button: number): boolean {
    const ev = new MouseEvent(type, { bubbles: true, cancelable: true, button });
    el.dispatchEvent(ev);
    return ev.defaultPrevented;
}

const renderers: Array<[string, () => Element]> = [
    ["LinkifiedText", () => render(() => <LinkifiedText text={`see ${URL} now`} />).container],
    ["Markdown", () => render(() => <Markdown text={`- **a5af**: ${URL}`} scrollable={false} />).container],
];

describe.each(renderers)("%s links", (_name, mount) => {
    it("open the system browser on a click", () => {
        const a = mount().querySelector("a")!;
        expect(fire(a, "click", 0)).toBe(true);
        expect(openLink).toHaveBeenCalledWith(URL);
        expect(createBlock).not.toHaveBeenCalled();
    });

    it("open a new browser pane on a middle-click, and never let the window navigate", () => {
        const a = mount().querySelector("a")!;
        expect(fire(a, "auxclick", 1)).toBe(true);
        expect(createBlock).toHaveBeenCalledWith({ meta: { view: "browser", url: URL } });
        expect(openLink).not.toHaveBeenCalled();
    });

    it("leave other buttons alone", () => {
        const a = mount().querySelector("a")!;
        expect(fire(a, "auxclick", 2)).toBe(false);
        expect(createBlock).not.toHaveBeenCalled();
    });
});

describe("onLinkAuxClick", () => {
    it("hands a link that isn't a web page to its system handler", () => {
        const ev = new MouseEvent("auxclick", { button: 1, cancelable: true });
        onLinkAuxClick(ev, "mailto:someone@example.com");
        expect(ev.defaultPrevented).toBe(true);
        expect(createBlock).not.toHaveBeenCalled();
        expect(openLink).toHaveBeenCalledWith("mailto:someone@example.com");
    });
});

describe("registerLinkInPaneListener", () => {
    it("opens a pane for a middle-clicked link the host caught", () => {
        registerLinkInPaneListener();
        listeners.get("open-link-in-pane")!({ url: URL });
        expect(createBlock).toHaveBeenCalledWith({ meta: { view: "browser", url: URL } });
    });
});
