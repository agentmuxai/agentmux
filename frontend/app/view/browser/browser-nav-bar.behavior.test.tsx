// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the browser pane's nav bar does, pinned as ordered logs: the address bar's two-way sync
 * with the page's URL, what Enter and a picked suggestion navigate to, the suggestions (inline
 * completion, the arrow keys, Escape, blur, an IME composition), the pane shortcuts the host
 * sends (focus address, Home, a new Incognito tab), the Bookmarks menu (the first load, a refresh,
 * adding and removing, a failed save) and the Home page. The profile button and the sign-in key
 * are stand-ins; everything else is the real nav bar. A refactor of the nav bar must leave the
 * committed snapshots as they are; regenerate with `vitest -u` only for a change meant to behave
 * differently.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    setStart: (_v: string) => {},
    setSettings: (_v: Record<string, unknown>) => {},
    bookmarks: [] as unknown[],
    listGate: null as null | Promise<void>,
    listFails: false,
    setFails: false,
    startFails: false,
    shortcut: null as null | ((p: { block_id: string; action: string }) => void),
    unsubscribed: 0,
    answer: null as null | ((text: string) => Promise<unknown>),
    menu: [] as Array<{ label: string; enabled?: boolean; click: () => void }>,
}));
const log = (line: string) => h.log.push(line);

vi.mock("@/store/config-signals", async (original) => {
    const { createSignal } = await import("solid-js");
    const [start, setStart] = createSignal("");
    const [settings, setSettings] = createSignal<Record<string, unknown>>({});
    h.setStart = setStart;
    h.setSettings = setSettings;
    return { ...(await original<Record<string, unknown>>()), browserStartPageAtom: start, settingsAtom: settings };
});
vi.mock("@/app/store/rpc-api", async (original) => ({
    ...(await original<Record<string, unknown>>()),
    RpcApi: {
        ListBookmarksCommand: async () => {
            log("rpc ListBookmarks");
            if (h.listGate) await h.listGate;
            if (h.listFails) throw new Error("disk unavailable");
            return { bookmarks: JSON.parse(JSON.stringify(h.bookmarks)) };
        },
        SetBookmarksCommand: async (_c: unknown, data: { bookmarks: unknown[] }) => {
            log(`rpc SetBookmarks ${JSON.stringify(data.bookmarks)}`);
            if (h.setFails) throw new Error("disk is full");
            h.bookmarks = data.bookmarks;
        },
        SetBrowserStartPageCommand: async (_c: unknown, data: { url: string }) => {
            log(`rpc SetBrowserStartPage ${JSON.stringify(data)}`);
            if (h.startFails) throw new Error("not allowed");
            h.setStart(data.url);
        },
    },
}));
vi.mock("@/app/keybindings", async (original) => ({
    ...(await original<Record<string, unknown>>()),
    paneCommandFor: (e: KeyboardEvent) => (e.key === "N" && e.shiftKey ? "browser:newIncognito" : null),
}));
vi.mock("@/app/element/ui/open-menu-at", async (original) => ({
    ...(await original<Record<string, unknown>>()),
    openMenuAt: (items: Array<{ label: string; enabled?: boolean; click: () => void }>) => {
        h.menu = items;
        log(`openMenuAt ${JSON.stringify(items.map((i) => ({ label: i.label, enabled: i.enabled })))}`);
    },
}));
vi.mock("@/app/platform/pane-overlay", async (original) => ({ ...(await original<Record<string, unknown>>()), usePaneOverlay: () => {} }));
vi.mock("./browser-profile-menu", () => ({
    openIncognitoTab: (model: { blockId: string }, home: string) => log(`openIncognitoTab ${model.blockId} home=${home}`),
    BrowserProfileButton: (p: { home: () => string; containWithin?: () => HTMLElement | null | undefined }) => {
        const el = document.createElement("div");
        el.className = "profile-stub";
        el.dataset.contain = p.containWithin?.()?.className ?? "none";
        el.dataset.home = p.home();
        return el;
    },
}));
vi.mock("./browser-signin-key", () => ({
    BrowserSigninKey: () => {
        const el = document.createElement("div");
        el.className = "signin-stub";
        return el;
    },
}));

import { makeTestHostApi } from "@/app/host/test-host";
import { BrowserNavBar } from "./browser-nav-bar";
import type { BrowserViewModel } from "./browser-model";

const out = () => h.log.join("\n").replace(/\n+$/, "") + "\n";
const snap = (name: string) => `./__snapshots__/browser-nav-bar.behavior.${name}.txt`;
const flush = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
};

interface Rig {
    bar: HTMLElement;
    input: HTMLInputElement;
    setUrl: (v: string) => void;
    setControls: (v: boolean) => void;
    setPaneCreated: (v: boolean) => void;
    paneNavFails: { value: boolean };
}

function mount(url = "https://example.com/"): Rig {
    const paneNavFails = { value: false };
    const [urlAtom, setUrl] = createSignal(url);
    const [controls, setControls] = createSignal(true);
    const [paneCreated, setPaneCreated] = createSignal(true);
    window.api = makeTestHostApi({
        listen: (async (event: string, cb: (p: { block_id: string; action: string }) => void) => {
            log(`listen ${event}`);
            h.shortcut = cb;
            return () => {
                h.unsubscribed++;
                log("unlisten");
            };
        }) as unknown as AppApi["listen"],
        reclaimWindowFocus: (async (label: string) => {
            log(`reclaimWindowFocus ${label}`);
        }) as unknown as AppApi["reclaimWindowFocus"],
        browserPanes: {
            navigate: async (blockId: string, u: string) => {
                log(`pane navigate ${blockId} ${u}`);
                if (paneNavFails.value) throw new Error("gone");
            },
            autocomplete: (blockId: string, text: string, suggest: boolean) => {
                log(`autocomplete ${blockId} ${JSON.stringify(text)} suggest=${suggest}`);
                return (h.answer ?? (async () => undefined))(text);
            },
            reload: async () => {},
            stop: async () => {},
        } as unknown as AppApi["browserPanes"],
    });
    const model = {
        blockId: "block-1",
        urlAtom,
        titleAtom: () => "Example Domain",
        faviconUrlAtom: () => "https://example.com/favicon.ico",
        showControlsAtom: controls,
        loadingAtom: () => false,
        canGoBackAtom: () => false,
        canGoForwardAtom: () => false,
        meta: () => ({}),
        navigate: (u: string) => log(`model navigate ${u}`),
        goBack: () => {},
        goForward: () => {},
        onError: (m: string) => log(`model onError ${m}`),
    } as unknown as BrowserViewModel;
    const { container } = render(() => (
        <div class="browser-view">
            <BrowserNavBar
                model={model}
                windowLabel="main"
                diag={(m) => log(`diag ${m}`)}
                paneCreated={paneCreated}
                createPane={async (u) => {
                    log(`createPane ${u}`);
                }}
            />
        </div>
    ));
    const bar = container.querySelector<HTMLElement>(".browser-nav-bar")!;
    const input = container.querySelector<HTMLInputElement>(".browser-address-bar")!;
    return { bar, input, setUrl, setControls, setPaneCreated, paneNavFails };
}

/** Focus the bar, as the user would. */
function focus(input: HTMLInputElement) {
    input.focus();
}

/** What the user types: the value changes, then an input event of `inputType` fires. */
function type(input: HTMLInputElement, value: string, inputType = "insertText") {
    input.value = value;
    fireEvent(input, new InputEvent("input", { bubbles: true, inputType, data: value }));
}

const key = (input: HTMLElement, k: string, init: KeyboardEventInit = {}) =>
    fireEvent.keyDown(input, { key: k, ...init });

const rows = () => [...document.querySelectorAll<HTMLElement>(".browser-address-suggestion")];
const state = (rig: Rig) =>
    log(
        `state value=${JSON.stringify(rig.input.value)} sel=${rig.input.selectionStart}-${rig.input.selectionEnd} rows=${JSON.stringify(
            rows().map((r) => `${r.classList.contains("selected") ? "*" : ""}${r.querySelector(".browser-address-suggestion-text")?.textContent}`),
        )}`,
    );

const match = (over: Partial<BrowserAutocompleteMatch> = {}): BrowserAutocompleteMatch => ({
    contents: "github.com",
    description: "GitHub",
    url: "https://github.com/",
    fill_into_edit: "github.com",
    inline_autocompletion: "hub.com",
    type: "history-url",
    provider: "HistoryURL",
    relevance: 1000,
    allowed_to_be_default: true,
    ...over,
});

const menuLabels = () =>
    [...document.querySelectorAll<HTMLElement>('[role="menu"] [role="menuitem"]')].map((r) => r.querySelector(".label")?.textContent ?? "");
const pickMenu = (label: string) => {
    const row = [...document.querySelectorAll<HTMLElement>('[role="menu"] [role="menuitem"]')].find(
        (r) => r.querySelector(".label")?.textContent === label,
    );
    if (!row) throw new Error(`no menu row ${label}`);
    fireEvent.click(row);
};
const bookmarksButton = (rig: Rig) => rig.bar.querySelector<HTMLElement>(".browser-nav-group--bookmarks button")!;
const openBookmarks = async (rig: Rig) => {
    fireEvent.click(bookmarksButton(rig));
    await flush();
};
const logBookmarksButton = (rig: Rig) =>
    log(
        `bookmarks button title=${JSON.stringify(bookmarksButton(rig).getAttribute("title"))} error=${!!rig.bar.querySelector(".browser-bookmark-btn-error")}`,
    );

beforeEach(() => {
    h.log.length = 0;
    h.bookmarks = [];
    h.listGate = null;
    h.listFails = false;
    h.setFails = false;
    h.startFails = false;
    h.shortcut = null;
    h.unsubscribed = 0;
    h.answer = null;
    h.setStart("");
    h.setSettings({});
    vi.spyOn(crypto, "randomUUID").mockReturnValue("00000000-0000-4000-8000-000000000001");
    vi.spyOn(Date, "now").mockReturnValue(1_800_000_000_000);
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    document.body.innerHTML = "";
});

describe("the address bar and the page's URL", () => {
    it("shows the page's address and follows it while the bar is not being edited", async () => {
        const rig = mount();
        log(`start value=${rig.input.value}`);
        rig.setUrl("https://example.com/next");
        log(`after nav value=${rig.input.value}`);
        focus(rig.input);
        rig.setUrl("https://example.com/while-focused");
        log(`while focused value=${rig.input.value}`);
        rig.input.blur();
        rig.setUrl("https://example.com/after-blur");
        log(`after blur value=${rig.input.value}`);
        await expect(out()).toMatchFileSnapshot(snap("sync"));
    });

    it("hides the whole bar when the pane shows no controls, and brings it back", () => {
        const rig = mount();
        rig.setControls(false);
        log(`hidden bar=${document.querySelector(".browser-nav-bar") === null}`);
        rig.setControls(true);
        log(`shown bar=${document.querySelector(".browser-nav-bar") !== null}`);
        return expect(out()).toMatchFileSnapshot(snap("controls"));
    });

    it("gives the profile button the pane it stays inside, and Home's address", () => {
        mount();
        const stub = document.querySelector<HTMLElement>(".profile-stub")!;
        log(`profile contain=${stub.dataset.contain} home=${stub.dataset.home}`);
        return expect(out()).toMatchFileSnapshot(snap("profile-props"));
    });

    it("logs the mouse, focus and blur of the address input", async () => {
        const rig = mount();
        fireEvent.mouseDown(rig.input);
        fireEvent.mouseUp(rig.input);
        focus(rig.input);
        rig.input.blur();
        await flush();
        const other = document.createElement("button");
        other.className = "other";
        document.body.append(other);
        focus(rig.input);
        other.focus();
        await flush();
        await expect(out()).toMatchFileSnapshot(snap("input-events"));
    });
});

describe("Enter and what it navigates to", () => {
    const submit = async (rig: Rig, text: string) => {
        focus(rig.input);
        type(rig.input, text);
        await flush();
        key(rig.input, "Enter");
        await flush();
    };

    it("turns what was typed into a page, a search or an about: page", async () => {
        const rig = mount();
        for (const text of ["example.org", "  localhost  ", "two words", "http://plain.example/x", "HTTPS://Up.Example", "about:blank", "a.b c", "   "]) {
            log(`-- ${JSON.stringify(text)}`);
            await submit(rig, text);
        }
        await expect(out()).toMatchFileSnapshot(snap("submit-normalize"));
    });

    it("creates the pane the first time, and drives the real pane after that", async () => {
        const rig = mount();
        rig.setPaneCreated(false);
        await submit(rig, "first.example");
        rig.setPaneCreated(true);
        await submit(rig, "second.example");
        await expect(out()).toMatchFileSnapshot(snap("submit-pane"));
    });

    it("reports a failed navigation of the pane to the model", async () => {
        const rig = mount();
        rig.paneNavFails.value = true;
        await submit(rig, "broken.example");
        await expect(out()).toMatchFileSnapshot(snap("submit-pane-fails"));
    });

    it("opens the engine's default match for what was typed, as Enter's target", async () => {
        h.answer = async () => ({ supported: true, matches: [match({ url: "http://localhost:3000/", contents: "localhost:3000", fill_into_edit: "localhost:3000", inline_autocompletion: "", type: "url-what-you-typed" })] });
        const rig = mount();
        await submit(rig, "localhost:3000");
        await expect(out()).toMatchFileSnapshot(snap("submit-default-match"));
    });
});

describe("the suggestions", () => {
    it("asks for what was typed, with the search engine unless the setting turns it off", async () => {
        const rig = mount();
        focus(rig.input);
        type(rig.input, "git");
        await flush();
        h.setSettings({ "browser:searchsuggestions": false });
        type(rig.input, "gith");
        await flush();
        h.setSettings({ "browser:searchsuggestions": true });
        type(rig.input, "githu");
        await flush();
        await expect(out()).toMatchFileSnapshot(snap("suggest-ask"));
    });

    it("completes the default match inline, selected after the caret, except while deleting", async () => {
        h.answer = async () => ({ supported: true, matches: [match(), match({ contents: "git-scm.com", url: "https://git-scm.com/", fill_into_edit: "git-scm.com", inline_autocompletion: "-scm.com", allowed_to_be_default: false })] });
        const rig = mount();
        focus(rig.input);
        type(rig.input, "git");
        await flush();
        state(rig);
        type(rig.input, "gi", "deleteContentBackward");
        await flush();
        state(rig);
        await expect(out()).toMatchFileSnapshot(snap("suggest-inline"));
    });

    it("moves the highlight with the arrow keys, shows its text, and Escape goes back to what was typed", async () => {
        h.answer = async () => ({ supported: true, matches: [match({ fill_into_edit: "", inline_autocompletion: "" }), match({ contents: "gitlab.com", url: "https://gitlab.com/", fill_into_edit: "gitlab.com", inline_autocompletion: "", allowed_to_be_default: false })] });
        const rig = mount();
        focus(rig.input);
        type(rig.input, "git");
        await flush();
        state(rig);
        for (const k of ["ArrowDown", "ArrowDown", "ArrowDown", "ArrowUp"]) {
            key(rig.input, k);
            log(`key ${k}`);
            state(rig);
        }
        key(rig.input, "Escape");
        log("key Escape");
        state(rig);
        await expect(out()).toMatchFileSnapshot(snap("suggest-keys"));
    });

    it("Enter opens the highlighted suggestion", async () => {
        h.answer = async () => ({ supported: true, matches: [match(), match({ contents: "gitlab.com", url: "https://gitlab.com/", fill_into_edit: "gitlab.com", inline_autocompletion: "", allowed_to_be_default: false })] });
        const rig = mount();
        focus(rig.input);
        type(rig.input, "git");
        await flush();
        key(rig.input, "ArrowDown");
        key(rig.input, "ArrowDown");
        key(rig.input, "Enter");
        await flush();
        state(rig);
        await expect(out()).toMatchFileSnapshot(snap("suggest-enter-highlight"));
    });

    it("a click on a suggestion opens it, and closes the list", async () => {
        h.answer = async () => ({ supported: true, matches: [match({ inline_autocompletion: "" }), match({ contents: "gitlab.com", url: "https://gitlab.com/", inline_autocompletion: "", allowed_to_be_default: false })] });
        const rig = mount();
        focus(rig.input);
        type(rig.input, "git");
        await flush();
        fireEvent.click(rows()[1]);
        await flush();
        state(rig);
        await expect(out()).toMatchFileSnapshot(snap("suggest-click"));
    });

    it("leaving the bar abandons a previewed suggestion: the page's own address comes back", async () => {
        h.answer = async () => ({ supported: true, matches: [match({ fill_into_edit: "", inline_autocompletion: "" }), match({ contents: "gitlab.com", url: "https://gitlab.com/", fill_into_edit: "gitlab.com", inline_autocompletion: "", allowed_to_be_default: false })] });
        const rig = mount();
        focus(rig.input);
        type(rig.input, "git");
        await flush();
        key(rig.input, "ArrowDown");
        key(rig.input, "ArrowDown");
        state(rig);
        rig.input.blur();
        await flush();
        state(rig);
        await expect(out()).toMatchFileSnapshot(snap("suggest-blur"));
    });

    it("an IME composition asks nothing and writes nothing until it ends; its keys are the IME's", async () => {
        h.answer = async () => ({ supported: true, matches: [match()] });
        const rig = mount();
        focus(rig.input);
        fireEvent.compositionStart(rig.input);
        type(rig.input, "ni", "insertCompositionText");
        key(rig.input, "Enter");
        key(rig.input, "ArrowDown");
        await flush();
        log("-- composing");
        state(rig);
        rig.input.value = "你";
        fireEvent.compositionEnd(rig.input);
        await flush();
        log("-- committed");
        state(rig);
        key(rig.input, "Enter", { isComposing: true });
        await flush();
        await expect(out()).toMatchFileSnapshot(snap("suggest-ime"));
    });

    it("a host with no autocomplete keeps the bar plain", async () => {
        h.answer = async () => ({ supported: false, matches: [] });
        const rig = mount();
        focus(rig.input);
        type(rig.input, "git");
        await flush();
        type(rig.input, "gith");
        await flush();
        state(rig);
        await expect(out()).toMatchFileSnapshot(snap("suggest-unsupported"));
    });
});

describe("the pane shortcuts the host sends", () => {
    const send = async (block_id: string, action: string) => {
        h.shortcut!({ block_id, action });
        await flush();
    };

    it("subscribes on mount and lets go on cleanup", async () => {
        mount();
        await flush();
        log(`unsubscribed=${h.unsubscribed}`);
        cleanup();
        log(`unsubscribed=${h.unsubscribed}`);
        await expect(out()).toMatchFileSnapshot(snap("shortcut-subscription"));
    });

    it("focus-address reclaims the window and selects the address; another pane's shortcut is ignored", async () => {
        const rig = mount();
        await flush();
        await send("block-2", "focus-address");
        log(`ignored active=${document.activeElement === rig.input}`);
        await send("block-1", "focus-address");
        log(`active=${document.activeElement === rig.input} sel=${rig.input.selectionStart}-${rig.input.selectionEnd}`);
        await send("block-1", "unknown-action");
        await expect(out()).toMatchFileSnapshot(snap("shortcut-focus-address"));
    });

    it("home goes to the Home page, else the default page", async () => {
        mount();
        await flush();
        await send("block-1", "home");
        h.setStart("https://start.example/");
        await send("block-1", "home");
        await expect(out()).toMatchFileSnapshot(snap("shortcut-home"));
    });

    it("new-incognito opens an Incognito tab at Home, and so does Ctrl+Shift+N anywhere in the bar", async () => {
        const rig = mount();
        await flush();
        await send("block-1", "new-incognito");
        const ev = new KeyboardEvent("keydown", { key: "N", shiftKey: true, bubbles: true, cancelable: true });
        rig.input.dispatchEvent(ev);
        log(`prevented=${ev.defaultPrevented}`);
        const plain = new KeyboardEvent("keydown", { key: "n", bubbles: true, cancelable: true });
        rig.input.dispatchEvent(plain);
        log(`plain prevented=${plain.defaultPrevented}`);
        await expect(out()).toMatchFileSnapshot(snap("shortcut-incognito"));
    });
});

describe("the Home button", () => {
    it("goes Home on a click, and its right-click offers to make this page Home only when it isn't already", async () => {
        const rig = mount();
        const home = rig.bar.querySelector<HTMLElement>('[data-testid="browser-home"]')!;
        log(`title=${home.getAttribute("title")}`);
        fireEvent.click(home);
        h.setStart("https://example.com/");
        log(`title=${home.getAttribute("title")}`);
        fireEvent.click(home);
        await expect(out()).toMatchFileSnapshot(snap("home-button"));
    });
});

describe("the Bookmarks menu", () => {
    it("shows Loading… on the first open only, then the saved bookmarks and the page's own rows", async () => {
        h.bookmarks = [
            { id: "b1", title: "Docs", url: "https://docs.example/", favicon_url: "https://docs.example/f.ico" },
            { id: "b2", title: "", url: "https://plain.example/", favicon_url: "" },
        ];
        let release!: () => void;
        h.listGate = new Promise<void>((r) => (release = r));
        const rig = mount();
        fireEvent.click(bookmarksButton(rig));
        await flush();
        log(`loading rows=${JSON.stringify(menuLabels())}`);
        release();
        await flush();
        log(`loaded rows=${JSON.stringify(menuLabels())}`);
        const firstImg = document.querySelector('[role="menu"] img');
        fireEvent.keyDown(document.body, { key: "Escape" });
        await flush();
        h.listGate = null;
        await openBookmarks(rig);
        log(`reopened rows=${JSON.stringify(menuLabels())} sameImg=${document.querySelector('[role="menu"] img') === firstImg}`);
        await expect(out()).toMatchFileSnapshot(snap("bookmarks-load"));
    });

    it("says there are no bookmarks, with or without the page's rows", async () => {
        const rig = mount("");
        await openBookmarks(rig);
        log(`no page rows=${JSON.stringify(menuLabels())}`);
        rig.setUrl("https://example.com/");
        await flush();
        log(`page rows=${JSON.stringify(menuLabels())}`);
        await expect(out()).toMatchFileSnapshot(snap("bookmarks-empty"));
    });

    it("bookmarks this page, then the row turns into Remove Bookmark, and removing it saves the empty list", async () => {
        const rig = mount();
        await openBookmarks(rig);
        pickMenu("Bookmark This Page");
        await flush();
        await openBookmarks(rig);
        log(`rows=${JSON.stringify(menuLabels())}`);
        pickMenu("Remove Bookmark");
        await flush();
        await openBookmarks(rig);
        log(`rows=${JSON.stringify(menuLabels())}`);
        await expect(out()).toMatchFileSnapshot(snap("bookmarks-toggle"));
    });

    it("a saved bookmark navigates the pane to it", async () => {
        h.bookmarks = [{ id: "b1", title: "Docs", url: "https://docs.example/", favicon_url: "" }];
        const rig = mount();
        await openBookmarks(rig);
        pickMenu("Docs");
        await flush();
        log(`value=${rig.input.value}`);
        await expect(out()).toMatchFileSnapshot(snap("bookmarks-navigate"));
    });

    it("a failed save rolls the list back and tints the button with the reason", async () => {
        const rig = mount();
        await openBookmarks(rig);
        h.setFails = true;
        pickMenu("Bookmark This Page");
        await flush();
        logBookmarksButton(rig);
        await openBookmarks(rig);
        log(`rows=${JSON.stringify(menuLabels())}`);
        await expect(out()).toMatchFileSnapshot(snap("bookmarks-save-fails"));
    });

    it("a failed load tints the button; the next open that works clears it", async () => {
        const rig = mount();
        h.listFails = true;
        await openBookmarks(rig);
        logBookmarksButton(rig);
        h.listFails = false;
        fireEvent.keyDown(document.body, { key: "Escape" });
        await flush();
        await openBookmarks(rig);
        logBookmarksButton(rig);
        await expect(out()).toMatchFileSnapshot(snap("bookmarks-load-fails"));
    });

    it("a favicon that fails to load falls back to the globe", async () => {
        h.bookmarks = [{ id: "b1", title: "Docs", url: "https://docs.example/", favicon_url: "https://docs.example/f.ico" }];
        const rig = mount();
        await openBookmarks(rig);
        const img = document.querySelector<HTMLElement>('[role="menu"] img')!;
        log(`img=${!!img} globe=${!!document.querySelector('[role="menu"] .fa-globe')}`);
        fireEvent.error(img);
        log(`img=${!!document.querySelector('[role="menu"] img')} globe=${!!document.querySelector('[role="menu"] .fa-globe')}`);
        await expect(out()).toMatchFileSnapshot(snap("bookmarks-favicon"));
    });
});

describe("the Home page", () => {
    it("Set as Home Page saves this page, and the row then says it already is", async () => {
        const rig = mount();
        await openBookmarks(rig);
        pickMenu("Set as Home Page");
        await flush();
        await openBookmarks(rig);
        log(`rows=${JSON.stringify(menuLabels())}`);
        await expect(out()).toMatchFileSnapshot(snap("home-page-set"));
    });

    it("a failed save tints the button; a retry that works clears it", async () => {
        const rig = mount();
        await openBookmarks(rig);
        h.startFails = true;
        pickMenu("Set as Home Page");
        await flush();
        logBookmarksButton(rig);
        fireEvent.keyDown(document.body, { key: "Escape" });
        await flush();
        h.startFails = false;
        await openBookmarks(rig);
        pickMenu("Set as Home Page");
        await flush();
        logBookmarksButton(rig);
        await expect(out()).toMatchFileSnapshot(snap("home-page-fails"));
    });

    it("the right-click on Home saves this page as Home, and is disabled with no page or when it already is Home", async () => {
        const rig = mount("");
        const home = rig.bar.querySelector<HTMLElement>('[data-testid="browser-home"]')!;
        fireEvent.contextMenu(home);
        h.menu[0].click();
        await flush();
        rig.setUrl("https://example.com/");
        fireEvent.contextMenu(home);
        h.menu[0].click();
        await flush();
        fireEvent.contextMenu(home);
        await expect(out()).toMatchFileSnapshot(snap("home-page-context"));
    });
});
