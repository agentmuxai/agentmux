// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InAppLoginPanel (the in-app OAuth sign-in block), pinned as a whole ahead of splitting it into a state object and
 * smaller views: each scenario mounts the real panel with stand-ins for the kit's button and text field, the login
 * backend, the host API and the clipboard, drives its props, types, pastes and clicks, and logs the panel as an
 * indented tree (inline attributes included) and, in order, every call it makes. The logs are snapshots recorded on
 * the unsplit component; the split must leave them byte for byte. (InAppLoginPanel.test.tsx keeps the phase-gating
 * checks it was written for.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({
    submit: null as any,
    open: null as any,
    write: null as any,
    read: null as any,
}));

vi.mock("../flows/login-backend", () => ({
    loginBackend: () => ({
        submitCode: (providerId: string, code: string) => (log(`submitCode ${providerId} ${JSON.stringify(code)}`), h.submit()),
    }),
}));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({ openExternal: (url: string) => (log(`openExternal ${url}`), h.open()) }),
}));
vi.mock("@/util/clipboard", () => ({
    readText: () => (log("clipboard readText"), h.read()),
    writeText: (text: string) => (log(`clipboard writeText ${text}`), h.write()),
}));
vi.mock("@/element/button", () => ({
    Button: (p: {
        className?: string;
        title?: string;
        disabled?: boolean;
        onClick?: () => void;
        children?: JSX.Element;
    }) => (
        <span
            data-stand="Button"
            data-class={p.className}
            data-title={p.title}
            data-disabled={String(!!p.disabled)}
            onClick={() => (log(`Button ${JSON.stringify(p.children)} onClick`), p.onClick?.())}
        >
            {p.children}
        </span>
    ),
}));
vi.mock("@/app/element/ui/inputs", () => ({
    TextInput: (p: any) => (
        <input
            data-stand="TextInput"
            ref={p.ref}
            data-variant={p.variant}
            class={p.class}
            placeholder={p.placeholder}
            data-value={p.value}
            onInput={p.onInput}
            onKeyDown={p.onKeyDown}
            onPaste={p.onPaste}
        />
    ),
}));
vi.mock("./InAppLoginPanel.scss", () => ({}));

import { InAppLoginPanel, type InAppLoginPhase } from "./InAppLoginPanel";

const SNAP = (name: string) => `./__snapshots__/InAppLoginPanel.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder)$/;

/** The panel as an indented tree: tags, classes, the attributes that carry state, and text. */
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

const URL1 = "https://claude.com/cai/oauth/authorize?code=true&state=abc";
const URL2 = "https://claude.com/cai/oauth/authorize?code=true&state=def";

let host: HTMLElement;
let setProps: (p: { phase?: InAppLoginPhase; authUrl?: string | undefined; label?: string }) => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const flush = () => new Promise<void>((r) => setTimeout(r, 0));
const input = () => host.querySelector<HTMLInputElement>("input")!;
const button = (text: string) =>
    [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")].find((b) => b.textContent === text)!;
const click = (text: string) => fireEvent.click(button(text));
const type = (value: string) => {
    input().value = value;
    fireEvent.input(input());
};
const focused = () => (document.activeElement === input() ? "paste input focused" : "paste input not focused");
const defer = () => {
    let resolve!: (v?: any) => void;
    let reject!: (e: any) => void;
    const promise = new Promise((res, rej) => ((resolve = res), (reject = rej)));
    return { promise, resolve, reject };
};

function mount(initial: { phase?: InAppLoginPhase; authUrl?: string | undefined } = {}) {
    const [props, set] = createSignal({ phase: "waiting-authorize" as InAppLoginPhase, authUrl: URL1 as string | undefined, label: "Claude", ...initial });
    setProps = (p) => set((prev) => ({ ...prev, ...p }));
    const view = render(() => (
        <InAppLoginPanel
            providerId="claude"
            providerLabel={props().label}
            authUrl={props().authUrl}
            phase={props().phase}
            onCancel={() => log("onCancel")}
            onUseTerminal={() => log("onUseTerminal")}
        />
    ));
    host = view.container;
}

beforeEach(() => {
    T.length = 0;
    h.submit = () => Promise.resolve();
    h.open = () => undefined;
    h.write = () => Promise.resolve();
    h.read = () => Promise.resolve("");
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame"] });
    vi.spyOn(console, "warn").mockImplementation((...a) => log(`console.warn ${a.join(" ")}`));
    vi.spyOn(console, "log").mockImplementation((...a) => log(`console.log ${a.map(String).join(" ")}`));
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("InAppLoginPanel, pinned", () => {
    it("says what each phase is doing, shows the link and paste steps only while starting or waiting with a link, and offers the terminal only then", () => {
        mount({ phase: "starting", authUrl: undefined });
        look("starting, no link yet");
        setProps({ authUrl: URL1 });
        look("starting, link arrived");
        setProps({ phase: "waiting-authorize" });
        look("waiting for the browser");
        setProps({ phase: "fallback" });
        look("fallback: the dead link and paste box are gone, no terminal button");
        setProps({ phase: "terminal-polling", label: "Codex" });
        look("terminal polling, another provider label");
        setProps({ phase: "waiting-authorize", authUrl: undefined });
        look("waiting, link gone");
        click("Cancel login");
        return snapshot("phases");
    });

    it("opens the link in the browser (warning when that throws), copies it with the app clipboard (logging when that fails), and shows it as the link's tooltip", async () => {
        mount();
        look("link step");
        click("Open link");
        h.open = () => {
            throw new Error("no browser");
        };
        click("Open link");
        h.open = () => {
            throw "plain string";
        };
        click("Open link");
        click("Copy link");
        h.write = () => Promise.reject("clipboard denied");
        click("Copy link");
        await flush();
        click("Use terminal instead");
        return snapshot("link-actions");
    });

    it("moves focus to the paste input a frame after a link appears, once per link, and not when the link goes", async () => {
        mount({ authUrl: undefined });
        log(`no link yet: ${focused()}`);
        setProps({ authUrl: URL1 });
        log(`link set, same frame: ${focused()}`);
        vi.advanceTimersByTime(20);
        log(`a frame later: ${focused()}`);
        input().blur();
        setProps({ phase: "starting" });
        vi.advanceTimersByTime(20);
        log(`phase changed, same link: ${focused()}`);
        setProps({ authUrl: URL2 });
        vi.advanceTimersByTime(20);
        log(`new link: ${focused()}`);
        input().blur();
        setProps({ authUrl: undefined });
        vi.advanceTimersByTime(20);
        log(`link gone: ${focused()}`);
        return snapshot("focus");
    });

    it("submits the typed code trimmed (from the button or Enter), disables the button while it is out, says it was accepted and clears the box, and says why when it fails", async () => {
        const call = defer();
        h.submit = () => call.promise;
        mount();
        type("  abc-123  ");
        look("typed");
        click("Submit");
        look("submitting");
        call.resolve();
        await flush();
        look("accepted, box cleared");
        h.submit = () => Promise.reject(new Error("bad code"));
        type("second");
        fireEvent.keyDown(input(), { key: "Enter" });
        await flush();
        look("failed with an Error");
        h.submit = () => Promise.reject("plain failure");
        type("third");
        fireEvent.keyDown(input(), { key: "a" });
        log("(a key other than Enter submits nothing)");
        click("Submit");
        look("submitting again clears the old result first");
        await flush();
        look("failed with a string");
        return snapshot("submit");
    });

    it("submits nothing for an empty or blank code, and reads the live input when the typed value never reached the signal", async () => {
        mount();
        click("Submit");
        type("   ");
        click("Submit");
        await flush();
        log("(blank codes submit nothing)");
        input().value = "from-the-dom";
        click("Submit");
        await flush();
        look("a value set without an input event is still submitted");
        return snapshot("submit-live-input");
    });

    it("submits a pasted code at once, ignores an empty paste, and paste-and-submit reads the clipboard (ignoring empty text, and saying so when it cannot read)", async () => {
        mount();
        fireEvent.paste(input(), { clipboardData: { getData: () => "  pasted-code \n" } });
        await flush();
        look("pasted code submitted");
        fireEvent.paste(input(), { clipboardData: { getData: () => "   " } });
        fireEvent.paste(input());
        await flush();
        log("(empty pastes submit nothing)");
        h.read = () => Promise.resolve("  from-clipboard ");
        click("Paste & submit");
        await flush();
        look("clipboard code submitted");
        h.read = () => Promise.resolve(undefined);
        click("Paste & submit");
        await flush();
        log("(an empty clipboard submits nothing)");
        h.read = () => Promise.reject(new Error("denied"));
        click("Paste & submit");
        await flush();
        look("clipboard unreadable");
        return snapshot("paste");
    });
});
