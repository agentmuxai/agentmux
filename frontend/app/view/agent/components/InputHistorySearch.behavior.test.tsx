// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InputHistorySearch (Ctrl+R in the agent composer), pinned as a whole ahead of splitting it into a state object and
 * views: each scenario mounts the real search over stand-ins for srv's two history reads, the forget and clear calls,
 * the kit's button and text input, the hover tip and the time format, on a fake clock, types, presses keys, hovers,
 * clicks and blurs, and logs the search as an indented tree (with where the focus is) and, in order, every call it
 * makes (each read with its request, each forget, clear and scroll, each time it hands a result to its parent).
 * The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (InputHistorySearch.test.tsx asserts what the person sees against the real kit; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({ all: null as any, search: null as any, forget: null as any, clear: null as any }));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentInputHistoryCommand: (_c: unknown, req: unknown) => (log(`AgentInputHistoryCommand ${JSON.stringify(req)}`), h.all()),
        AgentInputHistorySearchCommand: (_c: unknown, req: unknown) => (
            log(`AgentInputHistorySearchCommand ${JSON.stringify(req)}`),
            h.search(req)
        ),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("../agent-input-history", () => ({
    forgetInputHistoryEntry: (agent: string, id: number) => (log(`forgetInputHistoryEntry ${agent} ${id}`), h.forget()),
    clearInputHistory: (agent: string) => (log(`clearInputHistory ${agent}`), h.clear()),
}));
vi.mock("@/app/element/tip", () => ({ tip: (text: string) => ({ "data-tip": text }) }));
vi.mock("@/app/i18n/i18n", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/i18n/i18n")>()),
    formatLocale: () => "xx-XX",
}));
vi.mock("@/util/format-time", () => ({
    formatTimeOrDate: (ms: number, now: Date, locale?: string) => `at ${ms} (${now instanceof Date ? "now" : "?"}, ${locale})`,
}));
vi.mock("@/app/element/ui", () => ({
    Button: (p: {
        density?: string;
        tone?: string;
        disabled?: boolean;
        onMouseDown?: (e: MouseEvent) => void;
        onClick?: () => void;
        children?: JSX.Element;
    }) => (
        <span
            data-stand="Button"
            data-density={p.density}
            data-tone={p.tone}
            data-disabled={String(!!p.disabled)}
            tabindex="0"
            onMouseDown={(e) => p.onMouseDown?.(e)}
            onClick={() => (log(`Button ${JSON.stringify(p.children)} click`), p.disabled || p.onClick?.())}
        >
            {p.children}
        </span>
    ),
    TextInput: (p: any) => (
        <input
            data-stand="TextInput"
            ref={p.ref}
            data-variant={p.variant}
            data-value={p.value}
            placeholder={p.placeholder}
            spellcheck={p.spellcheck}
            aria-controls={p["aria-controls"]}
            onInput={(e) => p.onInput(e)}
            onKeyDown={(e) => p.onKeyDown(e)}
            onBlur={(e) => p.onBlur(e)}
        />
    ),
}));

import { InputHistorySearch } from "./InputHistorySearch";

const SNAP = (name: string) => `./__snapshots__/InputHistorySearch.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id|placeholder|spellcheck)$/;

/** The search as an indented tree: tags, classes, the attributes that carry state, and text. */
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

let host: HTMLElement;
let unmount: () => void;
const look = (label: string) => {
    log(`-- ${label} (focus: ${(document.activeElement as HTMLElement | null)?.dataset?.stand ?? document.activeElement?.tagName})`);
    log(dump(host).join("\n") || "-");
};
const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const defer = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
    return { promise, resolve, reject };
};
const field = () => host.querySelector<HTMLInputElement>("[data-stand='TextInput']")!;
const typeIn = (value: string) => {
    field().value = value;
    fireEvent.input(field());
};
const key = (k: string, init: KeyboardEventInit = {}) => fireEvent.keyDown(field(), { key: k, ...init });
const rows = () => [...host.querySelectorAll<HTMLElement>(".input-history-search__row")];
const press = (label: string) => {
    const found = [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")].find((b) => b.textContent?.trim() === label)!;
    fireEvent.click(found);
};
/** Lets the typing settle and the answer arrive. */
const settle = async (ms = 0) => {
    vi.advanceTimersByTime(ms);
    await flush();
};

const ENTRIES = [
    { id: 1, text: "first message", sends: 1, sent_at_ms: 1000 },
    { id: 2, text: "two lines\nsecond line", sends: 3, sent_at_ms: 2000 },
    { id: 3, text: "third", sends: 2, sent_at_ms: 3000 },
];
const answer = (entries = ENTRIES) => ({ entries });

function mount() {
    const view = render(() => <InputHistorySearch agentId="agent-1" onDone={(text) => log(`onDone ${JSON.stringify(text)}`)} />, {
        container: document.body.appendChild(document.createElement("div")),
    });
    host = view.container;
    unmount = view.unmount;
}

beforeEach(() => {
    T.length = 0;
    h.all = async () => answer();
    h.search = async () => answer([ENTRIES[1]]);
    h.forget = async () => undefined;
    h.clear = async () => undefined;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    (Element.prototype as any).scrollIntoView = function (this: HTMLElement, opts: unknown) {
        log(`scrollIntoView ${this.textContent?.slice(0, 12)} ${JSON.stringify(opts)}`);
    };
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    delete (Element.prototype as any).scrollIntoView;
    document.body.innerHTML = "";
});

describe("InputHistorySearch, pinned", () => {
    it("focuses the query on mount, reads the agent's latest entries at once with no query, lists each with its first line, how many times it was sent and when, selects the first, and says when there are none", async () => {
        mount();
        look("before the read");
        await settle();
        look("the latest entries");
        unmount();
        document.body.innerHTML = "";
        h.all = async () => answer([]);
        mount();
        await settle();
        look("nothing yet");
        return snapshot("load");
    });

    it("waits 120 ms after typing before searching, searches with the query and a limit of 50, goes back to the latest entries when the query is cleared, says when nothing matches, drops an answer that arrives after a later query's, and ignores a failed read", async () => {
        mount();
        await settle();
        typeIn("two");
        await settle(100);
        look("typed, not yet asked");
        await settle(20);
        look("searched");
        typeIn("tw");
        typeIn("two lines");
        await settle(120);
        log("-- one search for the last of two quick edits");
        typeIn("");
        await settle();
        look("query cleared");
        const slow = defer<any>();
        const fast = defer<any>();
        const queue = [slow, fast];
        h.search = () => queue.shift()!.promise;
        typeIn("a");
        await settle(120);
        typeIn("ab");
        await settle(120);
        fast.resolve(answer([ENTRIES[2]]));
        await flush();
        slow.resolve(answer([ENTRIES[0]]));
        await flush();
        look("the later answer won");
        h.search = async () => answer([]);
        typeIn("zzz");
        await settle(120);
        look("nothing matches");
        h.search = async () => {
            throw new Error("srv is down");
        };
        typeIn("zz");
        await settle(120);
        look("a failed read leaves the list");
        unmount();
        log("unmounted");
        return snapshot("search");
    });

    it("moves the selection with the arrows and Ctrl+R (wrapping, and scrolling the row into view), with the mouse, and picks with Enter, a click or Escape, which hands the parent the entry or nothing", async () => {
        mount();
        await settle();
        key("ArrowDown");
        key("ArrowDown");
        look("down twice");
        key("ArrowDown");
        look("wrapped to the first");
        key("ArrowUp");
        look("up wrapped to the last");
        key("r", { ctrlKey: true });
        look("Ctrl+R wrapped down");
        key("r");
        key("Delete");
        key("a");
        fireEvent.mouseEnter(rows()[1]);
        look("hovered the second");
        key("Enter");
        key("Escape");
        const prevented = fireEvent.mouseDown(rows()[2]);
        log(`mouse down on the third, default allowed: ${prevented}`);
        h.all = async () => answer([]);
        unmount();
        document.body.innerHTML = "";
        mount();
        await settle();
        key("ArrowDown");
        key("ArrowUp");
        key("Enter");
        key("Delete", { shiftKey: true });
        return snapshot("keys");
    });

    it("forgets the selected entry with Shift+Delete: the row gone and the selection kept in range, the failure shown as an error's message or plain text and dropped by the next try, and nothing with no entry", async () => {
        mount();
        await settle();
        key("ArrowUp");
        key("Delete", { shiftKey: true });
        await flush();
        look("the last forgotten");
        h.forget = async () => {
            throw new Error("srv refused");
        };
        key("Delete", { shiftKey: true });
        await flush();
        look("forget failed");
        h.forget = async () => {
            throw "plain refusal";
        };
        key("Delete", { shiftKey: true });
        await flush();
        look("a plain refusal");
        h.forget = async () => undefined;
        key("Delete", { shiftKey: true });
        await flush();
        look("forgot the second, the error gone");
        key("Delete", { shiftKey: true });
        await flush();
        look("none left");
        key("Delete", { shiftKey: true });
        return snapshot("forget");
    });

    it("clears the history after asking: Clear history is off with nothing listed and no query, asks again with 'Forget everything', empties the list and refocuses, shows a failure and goes back to asking, and the buttons keep the query's focus on mouse down", async () => {
        h.all = async () => answer([]);
        mount();
        await settle();
        look("empty, Clear history off");
        typeIn("q");
        look("a query turns it on");
        unmount();
        document.body.innerHTML = "";
        h.all = async () => answer();
        mount();
        await settle();
        const clear = [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")][0];
        log(`mouse down on Clear history, default allowed: ${fireEvent.mouseDown(clear)}`);
        press("Clear history");
        look("asking");
        const sure = [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")][0];
        log(`mouse down on Forget everything, default allowed: ${fireEvent.mouseDown(sure)}`);
        h.clear = async () => {
            throw new Error("srv refused");
        };
        press("Forget everything typed to this agent");
        await flush();
        look("clear failed");
        h.clear = async () => {
            throw "plain refusal";
        };
        press("Clear history");
        press("Forget everything typed to this agent");
        await flush();
        look("a plain refusal");
        h.clear = async () => undefined;
        press("Clear history");
        press("Forget everything typed to this agent");
        await flush();
        look("cleared");
        return snapshot("clear");
    });

    it("closes when the query loses focus to something outside the search or to nothing, and stays open when the focus moves to a row or button inside it", async () => {
        mount();
        await settle();
        const button = host.querySelector<HTMLElement>("[data-stand='Button']")!;
        fireEvent.blur(field(), { relatedTarget: button });
        log("-- focus moved to a button inside");
        fireEvent.blur(field(), { relatedTarget: document.body });
        log("-- focus moved outside");
        fireEvent.blur(field(), { relatedTarget: null });
        log("-- focus moved nowhere");
        return snapshot("blur");
    });
});
