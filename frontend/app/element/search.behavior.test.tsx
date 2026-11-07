// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Search (the find bar over a pane), pinned as a whole ahead of splitting it into a state object and a view: each
 * scenario mounts the real bar over the atoms `useSearch` makes, with stand-ins for the text input, the icon buttons
 * and the floating-position library, drives the input, the keys and the buttons, and logs the bar as an indented
 * tree (read from the document, as it is portalled) and, in order, every call it makes (the search callbacks, the
 * auto-update, the position it computes and the offset it asks for). The logs are snapshots recorded on the unsplit
 * component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({
    input: null as any,
    update: null as null | (() => void | Promise<void>),
    rects: { reference: { x: 300, y: 200, width: 400, height: 20 }, floating: { width: 260, height: 32 } } as any,
}));

vi.mock("@floating-ui/dom", () => ({
    offset: (cb: unknown) => ({ name: "offset", cb }),
    autoUpdate: (reference: Element, floating: Element, update: () => void | Promise<void>) => {
        h.update = update;
        log(`autoUpdate ${reference.tagName} ${floating.className}`);
        return () => log("autoUpdate cleanup");
    },
    computePosition: async (_ref: Element, _floating: Element, opts: any) => {
        const result = opts.middleware[0].cb({ rects: h.rects });
        log(`computePosition ${opts.placement} offset=${JSON.stringify(result)}`);
        return { x: 111, y: 222 };
    },
}));
vi.mock("./input", () => ({
    Input: (p: any) => {
        h.input = p;
        return (
            <input
                data-stand="Input"
                placeholder={p.placeholder}
                value={p.value}
                data-autofocus={String(p.autoFocus)}
                data-autoselect={String(p.autoSelect)}
            />
        );
    },
}));
vi.mock("@/app/element/ui/DeclIconButton", () => ({
    DeclIconButton: (p: { decl: IconButtonDecl | ToggleIconButtonDecl }) => (
        <button
            data-stand="DeclIconButton"
            data-elemtype={p.decl.elemtype}
            data-icon={String(p.decl.icon)}
            title={p.decl.title}
            disabled={p.decl.disabled ?? false}
            data-active={p.decl.elemtype === "toggleiconbutton" ? String(p.decl.active()) : undefined}
            onClick={() => {
                log(`DeclIconButton ${p.decl.title} click`);
                if (p.decl.disabled) return;
                if (p.decl.elemtype === "toggleiconbutton") p.decl.active._set(!p.decl.active());
                else p.decl.click?.();
            }}
        />
    ),
}));
vi.mock("./search.scss", () => ({}));

import { Search, useSearch } from "./search";

const SNAP = (name: string) => `./__snapshots__/search.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder)$/;

/** The bar as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let s: SearchProps;
const bar = () => document.body.querySelector(".search-container");
const state = () =>
    `state: open=${s.isOpen()} search=${JSON.stringify(s.searchValue())} index=${s.resultsIndex()} count=${s.resultsCount()}`;
const look = (label: string) => {
    log(`-- ${label}`);
    log(bar() ? dump(bar()!).join("\n") : "-");
    log(state());
};
const flush = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
};
const click = (title: string) => {
    const b = [...document.body.querySelectorAll<HTMLElement>("[data-stand='DeclIconButton']")].find((x) =>
        x.title.startsWith(title)
    );
    fireEvent.click(b!);
};
const key = (init: { key: string; shiftKey?: boolean }) => {
    const e = { ...init, shiftKey: init.shiftKey ?? false, preventDefault: () => log(`preventDefault ${init.key}`) };
    h.input.onKeyDown(e);
};

type SearchProps = ReturnType<typeof useSearch>;

function mount(options: Parameters<typeof useSearch>[0] = {}, extra: Partial<Parameters<typeof Search>[0]> = {}) {
    s = useSearch(options);
    return render(() => <Search {...s} {...extra} />);
}

beforeEach(() => {
    T.length = 0;
    h.update = null;
    h.input = null;
    h.rects = { reference: { x: 300, y: 200, width: 400, height: 20 }, floating: { width: 260, height: 32 } };
    vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => (log("requestAnimationFrame"), cb(0), 1));
    vi.spyOn(document.documentElement, "getBoundingClientRect").mockReturnValue({
        bottom: 800,
        top: 0,
        left: 0,
        right: 1200,
        width: 1200,
        height: 800,
        x: 0,
        y: 0,
        toJSON: () => ({}),
    });
});
afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

describe("Search, pinned", () => {
    it("shows nothing while closed, and opens to the input, the result count (hidden at none) and the buttons, with the toggles only for the options given", async () => {
        const onSearch = vi.fn((q: string) => log(`onSearch ${JSON.stringify(q)}`));
        mount({}, { onSearch });
        look("closed");
        s.isOpen._set(true);
        look("open, no toggles");
        s.resultsCount._set(5);
        s.resultsIndex._set(2);
        look("open, 3 of 5");
        cleanup();
        T.push("-- with all three toggles");
        const onSearch2 = vi.fn((q: string) => log(`onSearch ${JSON.stringify(q)}`));
        mount({ regex: true, caseSensitive: false, wholeWord: true }, { onSearch: onSearch2 });
        s.isOpen._set(true);
        look("open, three toggles");
        cleanup();
        T.push("-- only one toggle");
        mount({ caseSensitive: true });
        s.isOpen._set(true);
        look("open, case only");
        return snapshot("open");
    });

    it("searches as it is typed: resets the index and count, calls onSearch with the text, and closing clears the text, the index and the count", async () => {
        const onSearch = vi.fn((q: string) => log(`onSearch ${JSON.stringify(q)}`));
        mount({}, { onSearch });
        log("-- the first call");
        s.isOpen._set(true);
        look("opened");
        s.resultsCount._set(4);
        s.resultsIndex._set(3);
        log(state());
        h.input.onChange("needle");
        look("typed 'needle'");
        s.resultsCount._set(2);
        s.resultsIndex._set(1);
        h.input.onChange("needle");
        look("the same text again, nothing re-runs");
        h.input.onChange("needles");
        look("typed 'needles'");
        s.resultsCount._set(3);
        s.resultsIndex._set(2);
        click("Close");
        look("closed with the close button");
        s.isOpen._set(true);
        look("opened again, empty");
        log(
            `input props: ${JSON.stringify({ placeholder: h.input.placeholder, value: h.input.value, autoFocus: h.input.autoFocus, autoSelect: h.input.autoSelect })}`
        );
        return snapshot("search");
    });

    it("steps through the results with Enter and Shift+Enter and the buttons, wrapping at both ends, and hands the steps to onNext and onPrev when given", async () => {
        mount({}, {});
        s.isOpen._set(true);
        s.resultsCount._set(3);
        log("-- own stepping, 3 results");
        key({ key: "Enter" });
        log(state());
        key({ key: "Enter" });
        log(state());
        key({ key: "Enter" });
        log(state());
        key({ key: "Enter", shiftKey: true });
        log(state());
        key({ key: "Enter", shiftKey: true });
        log(state());
        key({ key: "a" });
        key({ key: "Escape", shiftKey: true });
        log("-- the buttons");
        click("Next Result");
        log(state());
        click("Previous Result");
        click("Previous Result");
        log(state());
        look("buttons, 3 results");
        s.resultsCount._set(0);
        look("no results: the step buttons are off");
        click("Next Result");
        log(state());
        key({ key: "Enter" });
        log(state());
        look("Enter with no results");
        cleanup();
        T.push("-- handed to onNext and onPrev");
        const onNext = vi.fn(() => log("onNext"));
        const onPrev = vi.fn(() => log("onPrev"));
        mount({}, { onNext, onPrev });
        s.isOpen._set(true);
        s.resultsCount._set(3);
        key({ key: "Enter" });
        key({ key: "Enter", shiftKey: true });
        click("Next Result");
        click("Previous Result");
        log(state());
        cleanup();
        T.push("-- only onNext given");
        mount({}, { onNext });
        s.isOpen._set(true);
        s.resultsCount._set(3);
        key({ key: "Enter" });
        key({ key: "Enter", shiftKey: true });
        log(state());
        return snapshot("stepping");
    });

    it("flips each toggle from its button and shows it pressed", async () => {
        mount({ regex: false, caseSensitive: false, wholeWord: false });
        s.isOpen._set(true);
        look("all off");
        click("Case Sensitive");
        click("Regular Expression");
        log(`regex=${s.regex!()} caseSensitive=${s.caseSensitive!()} wholeWord=${s.wholeWord!()}`);
        look("case and regex on");
        click("Whole Word");
        click("Case Sensitive");
        log(`regex=${s.regex!()} caseSensitive=${s.caseSensitive!()} wholeWord=${s.wholeWord!()}`);
        look("whole word and regex on");
        return snapshot("toggles");
    });

    it("places itself over its anchor: auto-updates only for a real element, asks for the offset that keeps it on screen, moves to the computed spot and stops on unmount", async () => {
        const anchor = document.createElement("section");
        document.body.appendChild(anchor);
        const view = mount({ anchorRef: { current: anchor } });
        look("closed, nothing placed");
        s.isOpen._set(true);
        look("opened over an anchor");
        await h.update!();
        look("after computePosition");
        log("-- a reference near the left edge and the bottom");
        h.rects = { reference: { x: 10, y: 790, width: 100, height: 20 }, floating: { width: 260, height: 32 } };
        await h.update!();
        h.rects = { reference: { x: 700, y: 100, width: 300, height: 20 }, floating: { width: 260, height: 32 } };
        await h.update!();
        view.unmount();
        log("-- unmounted");
        cleanup();
        T.push("-- custom offsets");
        h.update = null;
        s = useSearch({ anchorRef: { current: anchor } });
        render(() => <Search {...s} offsetX={4} offsetY={0} />);
        s.isOpen._set(true);
        h.rects = { reference: { x: 10, y: 790, width: 100, height: 20 }, floating: { width: 260, height: 32 } };
        await h.update!();
        cleanup();
        T.push("-- no anchor");
        h.update = null;
        mount({});
        s.isOpen._set(true);
        look("opened with no anchor");
        log(`auto-update started: ${h.update !== null}`);
        cleanup();
        T.push("-- the anchor is not an element");
        h.update = null;
        mount({ anchorRef: { current: {} as HTMLElement } });
        s.isOpen._set(true);
        log(`auto-update started: ${h.update !== null}`);
        anchor.remove();
        return snapshot("position");
    });

    it("useSearch makes the atoms (each toggle only when its option is given), hands them to a view model, and keeps the anchor given or makes an empty one", async () => {
        const none = useSearch();
        log(
            `defaults: open=${none.isOpen()} search=${JSON.stringify(none.searchValue())} index=${none.resultsIndex()} count=${none.resultsCount()} regex=${none.regex} caseSensitive=${none.caseSensitive} wholeWord=${none.wholeWord} anchor=${JSON.stringify(none.anchorRef)}`
        );
        const vm: { searchAtoms?: SearchAtoms } = {};
        const el = document.createElement("div");
        const all = useSearch({
            viewModel: vm,
            anchorRef: { current: el },
            regex: false,
            caseSensitive: true,
            wholeWord: false,
        });
        log(
            `options: regex=${all.regex!()} caseSensitive=${all.caseSensitive!()} wholeWord=${all.wholeWord!()} anchorIsGiven=${all.anchorRef!.current === el}`
        );
        log(
            `view model got the atoms: ${vm.searchAtoms?.isOpen === all.isOpen && vm.searchAtoms?.searchValue === all.searchValue && vm.searchAtoms?.regex === all.regex}`
        );
        const noVm = useSearch({ regex: true });
        log(`only regex: regex=${noVm.regex!()} caseSensitive=${noVm.caseSensitive} wholeWord=${noVm.wholeWord}`);
        return snapshot("use-search");
    });
});
