// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A click on a link opens the system browser; a middle-click opens a new
// browser pane. Neither may leave the browser's default in place: for a
// middle-click that default loads the page in the app's own window.

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const openLink = vi.fn();
const createBlock = vi.fn((..._args: unknown[]) => Promise.resolve("block-1"));
const listeners = new Map<string, (payload: unknown) => void>();
const openPathLink = vi.fn((..._args: unknown[]) => Promise.resolve());
const openTaskLink = vi.fn((..._args: unknown[]) => Promise.resolve());
const openWebLinkFloating = vi.fn((..._args: unknown[]) => Promise.resolve());
vi.mock("./link-destinations", () => ({
    openPathLink: (...a: unknown[]) => openPathLink(...a),
    openTaskLink: (...a: unknown[]) => openTaskLink(...a),
    openWebLinkFloating: (...a: unknown[]) => openWebLinkFloating(...a),
}));
vi.mock("@/app/host/host-caps", () => ({ hostHas: () => true }));
vi.mock("@/util/platformutil", () => ({ isModKey: (e: { ctrlKey: boolean }) => e.ctrlKey }));
vi.mock("@/app/store/global", () => ({
    openLink: (...a: unknown[]) => openLink(...a),
    createBlock: (...a: unknown[]) => createBlock(...a),
    // Markdown reads its `markdown:streamtailinplace` kill switch on mount.
    getSettingsKeyAtom: () => () => undefined,
    getApi: () => ({
        listen: (name: string, cb: (payload: unknown) => void) => {
            listeners.set(name, cb);
            return Promise.resolve(() => {});
        },
    }),
}));

import { noteClick } from "@/app/store/float-modifier";
import { LinkifiedText } from "./linkified-text";
import { onLinkAuxClick, registerLinkInPaneListener } from "./link-open";
import { Markdown } from "./markdown";

const URL = "https://github.com/settings/installations";

afterEach(() => {
    cleanup();
    openLink.mockReset();
    createBlock.mockClear();
    openPathLink.mockClear();
    openTaskLink.mockClear();
    openWebLinkFloating.mockClear();
    noteClick({ ctrlKey: false, metaKey: false, shiftKey: false, altKey: false });
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


describe("Markdown links into AgentMux", () => {
    const link = (target: string) => render(() => <Markdown text={`[the thing](${target})`} scrollable={false} />).container.querySelector("a");
    /** The open is a dynamic import away, so give it a turn. */
    const settle = () => new Promise((r) => setTimeout(r, 0));

    it.each([
        ["C:/Users/me/notes.md", "C:/Users/me/notes.md"],
        ["/home/me/notes.md", "/home/me/notes.md"],
        ["~/notes.md", "~/notes.md"],
        ["file:///C:/Users/me/a%20b.md", "C:/Users/me/a b.md"],
    ])("open the file %s in the Editor, not the system's app", async (href, path) => {
        const a = link(href)!;
        expect(a).not.toBeNull();
        expect(fire(a, "click", 0)).toBe(true);
        await settle();
        expect(openPathLink).toHaveBeenCalledWith(path, { floating: false });
        expect(openLink).not.toHaveBeenCalled();
    });

    it("open a task on the Kanban", async () => {
        const a = link("agentmux://task/14995")!;
        expect(fire(a, "click", 0)).toBe(true);
        await settle();
        expect(openTaskLink).toHaveBeenCalledWith(14995, { floating: false });
        expect(openLink).not.toHaveBeenCalled();
    });

    it("never hand any other agentmux: URL to the system, or open it", async () => {
        const a = link("agentmux://agents/x/stop")!;
        expect(fire(a, "click", 0)).toBe(true);
        await settle();
        expect(openLink).not.toHaveBeenCalled();
        expect(openTaskLink).not.toHaveBeenCalled();
        expect(openPathLink).not.toHaveBeenCalled();
    });

    it("open a file in the Editor on a middle-click too, not a browser pane", async () => {
        const a = link("/home/me/notes.md")!;
        expect(fire(a, "auxclick", 1)).toBe(true);
        await settle();
        expect(openPathLink).toHaveBeenCalledWith("/home/me/notes.md", { floating: false });
        expect(createBlock).not.toHaveBeenCalled();
    });

    it("still drop a javascript: link", () => {
        const a = link("javascript:alert(1)");
        expect(a?.getAttribute("href") ?? null).toBeNull();
    });
});


describe("links clicked with the float modifier", () => {
    const ctrlClick = (target: string) => {
        const a = render(() => <Markdown text={`[the thing](${target})`} scrollable={false} />).container.querySelector("a")!;
        // The capture listener (installFloatModifier) notes the click before the link's handler runs.
        noteClick({ ctrlKey: true, metaKey: false, shiftKey: false, altKey: false });
        fire(a, "click", 0);
    };
    const settle = () => new Promise((r) => setTimeout(r, 0));

    it("open a web page in a floating browser pane, not the system browser", async () => {
        ctrlClick(URL);
        await settle();
        expect(openWebLinkFloating).toHaveBeenCalledWith(URL);
        expect(openLink).not.toHaveBeenCalled();
        expect(createBlock).not.toHaveBeenCalled();
    });

    it("open a file in a floating Editor and a task on a floating Kanban", async () => {
        ctrlClick("/home/me/notes.md");
        await settle();
        expect(openPathLink).toHaveBeenCalledWith("/home/me/notes.md", { floating: true });
        cleanup();
        ctrlClick("agentmux://task/12");
        await settle();
        expect(openTaskLink).toHaveBeenCalledWith(12, { floating: true });
    });

    it("leave a link for the system's handler alone (nothing opens in a pane)", async () => {
        ctrlClick("mailto:someone@example.com");
        await settle();
        expect(openLink).toHaveBeenCalledWith("mailto:someone@example.com");
        expect(openWebLinkFloating).not.toHaveBeenCalled();
    });
});
