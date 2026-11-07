// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MarkdownBlock (an agent message's markdown row), pinned as a whole ahead of splitting it into a render-throttle
 * hook, a peek hook and smaller views: each scenario mounts the real block with stand-ins for the markdown renderer,
 * the kit button, the peek panel, the shared ticker, the dormancy flag, the media options and the time formatters,
 * drives the node, the pin and the clock, and logs the block as an indented tree (attributes included) and, in order,
 * every render and call it makes. The logs are snapshots recorded on the unsplit component; the split must leave them
 * byte for byte. (MarkdownBlock.test.tsx, .dormancy, .media and .stream-pauses keep the checks they were written for.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createEffect, createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({ tick: null as any, dormant: null as any, media: null as any }));

vi.mock("@/app/element/markdown", () => ({
    Markdown: (p: { text: string; streaming?: boolean; scrollable?: boolean; media?: unknown }) => {
        createEffect(() =>
            log(
                `Markdown text=${JSON.stringify(p.text)} streaming=${p.streaming} scrollable=${p.scrollable} media=${JSON.stringify(p.media)}`
            )
        );
        return <div data-stand="Markdown">{p.text}</div>;
    },
}));
vi.mock("@/app/element/ui/Button", () => ({
    Button: (p: { variant?: string; class?: string; onClick?: () => void; "aria-expanded"?: boolean; children?: JSX.Element }) => (
        <span
            data-stand="Button"
            data-variant={p.variant}
            data-class={p.class}
            aria-expanded={p["aria-expanded"] ? "true" : "false"}
            onClick={() => (log("Button onClick"), p.onClick?.())}
        >
            {p.children}
        </span>
    ),
}));
vi.mock("./PeekOverlay", () => ({
    PeekOverlay: (p: { show: boolean; rowEl: () => HTMLElement | undefined; children?: JSX.Element }) => {
        createEffect(() => log(`PeekOverlay show=${p.show} rowEl=${p.rowEl()?.className ?? "none"}`));
        return <div data-stand="PeekOverlay">{p.children}</div>;
    },
}));
vi.mock("./PeekMetaRow", () => ({
    PeekMetaRow: (p: { time: string | null; tokens: string | null }) => (
        <div data-stand="PeekMetaRow" data-time={String(p.time)} data-tokens={String(p.tokens)} />
    ),
}));
vi.mock("@/app/hook/useTick", () => ({
    useTick: (ms: number) => (log(`useTick ${ms}`), () => (log("tick read"), h.tick[0]())),
}));
vi.mock("../agent-dormancy", () => ({ useAgentDormant: () => () => h.dormant[0]() }));
vi.mock("../agent-media", () => ({ useAgentMedia: () => () => h.media[0]() }));
vi.mock("@/util/format-time", () => ({
    formatExactTime: (ts: number) => `exact(${ts})`,
    formatTimeAgo: (ts: number) => `ago(${ts})`,
}));

import { MarkdownBlock } from "./MarkdownBlock";
import type { MarkdownNode, ViewModeTag } from "../../../agent-model/types";

const SNAP = (name: string) => `./__snapshots__/MarkdownBlock.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder)$/;

/** The block as an indented tree: tags, classes, the attributes that carry state, and text. */
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

type Node_ = MarkdownNode & ViewModeTag;
const md = (content: string, extra: Partial<Node_> = {}): Node_ =>
    ({ type: "markdown", id: "md-1", content, ...extra }) as Node_;

let host: HTMLElement;
let unmount: () => void;
let setNode: (n: Node_) => void;
let setPinned: (p: boolean) => void;
let clock = 0;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
/** Move the clock (timers and performance.now) to `ms`, letting the timers up to it run. */
const at = (ms: number) => {
    vi.advanceTimersByTime(ms - clock);
    clock = ms;
};
/** Move the clock to `ms`, then set the node's content (as a streamed chunk would). */
const chunk = (ms: number, node: Node_) => {
    at(ms);
    log(`chunk at ${ms}: ${JSON.stringify(node.content)}`);
    setNode(node);
};

function mount(node: Node_, opts: { pinned?: boolean; onTogglePin?: boolean } = {}) {
    const [n, setN] = createSignal(node);
    const [pinned, setP] = createSignal(opts.pinned ?? false);
    setNode = setN;
    setPinned = setP;
    const view = render(() => (
        <MarkdownBlock
            node={n()}
            pinned={pinned()}
            onTogglePin={opts.onTogglePin === false ? undefined : () => log("onTogglePin")}
        />
    ));
    host = view.container;
    unmount = view.unmount;
}

beforeEach(() => {
    T.length = 0;
    h.tick = createSignal(0);
    h.dormant = createSignal(false);
    h.media = createSignal<unknown>(undefined);
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "performance"] });
    vi.advanceTimersByTime(1000);
    clock = 1000;
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("MarkdownBlock, pinned", () => {
    it("renders plain text in a peek anchor with no thinking style, and a thinking clump with it, passing the media options and never scrolling", () => {
        h.media[1]({ baseDir: "/work" });
        mount(md("Hello **world**"));
        look("plain text");
        cleanup();
        T.push("-- thinking clump");
        mount(md("Let me think", { metadata: { thinking: true } }));
        look("a thinking clump (no view mode: always open, no header)");
        return snapshot("plain");
    });

    it("commits the first chunk of a stream cheaply on the leading edge, holds later ones inside the window, and settles once updates stop", () => {
        mount(md("a"));
        at(1200);
        chunk(1200, md("ab"));
        chunk(1210, md("abc"));
        chunk(1250, md("abcd"));
        chunk(1295, md("abcde"));
        log("-- a quiet window: settle");
        at(1385);
        chunk(1500, md("abcdef"));
        at(1600);
        log("(settled)");
        chunk(1610, md("abcdef"));
        at(1700);
        look("end of stream");
        return snapshot("stream");
    });

    it("re-renders only when the text or the streaming flag really changes", () => {
        mount(md("same"));
        at(1100);
        chunk(1100, md("same"));
        at(1300);
        chunk(1400, md("same", { id: "md-other" }));
        at(1500);
        log("(no further render for equal values)");
        return snapshot("stream-equal");
    });

    it("does no rendering while the pane is dormant, drops its pending settle, and draws what arrived meanwhile once, settled, when shown", () => {
        mount(md("one"));
        at(1100);
        chunk(1100, md("one two"));
        h.dormant[1](true);
        log("pane backgrounded (the settle timer was pending)");
        at(1300);
        log("(a dormant pane never paints the pending settle)");
        chunk(1310, md("one two three"));
        chunk(1330, md("one two three four"));
        at(1500);
        h.dormant[1](false);
        log("pane shown again");
        at(1700);
        look("settled once");
        h.dormant[1](true);
        h.dormant[1](false);
        log("(hidden and shown with nothing new)");
        chunk(1800, md("one two three four five"));
        at(1900);
        return snapshot("dormant");
    });

    it("folds a canceled thought under a header with its partial text on open, and a view-folded thought under a Thinking header, following the pin", () => {
        mount(md("partial", { metadata: { canceled: true } }));
        look("canceled, closed");
        setPinned(true);
        look("canceled, pinned open: its own content, not the throttled view");
        fireEvent.click(host.querySelector("[data-stand='Button']")!);
        setNode(md("partial more", { metadata: { canceled: true } }));
        look("canceled content follows the node");
        cleanup();
        T.push("-- a view-folded thought");
        mount(md("deep thought", { metadata: { thinking: true }, viewMode: "collapsed" } as any));
        look("thinking, collapsed by the view");
        setPinned(true);
        look("thinking, pinned open: the throttled view");
        setNode(md("deep thought, longer", { metadata: { thinking: true }, viewMode: "collapsed" } as any));
        look("thinking content follows the node");
        cleanup();
        T.push("-- a view-expanded thought");
        mount(md("open thought", { metadata: { thinking: true }, viewMode: "expanded" } as any));
        look("thinking, expanded by the view");
        cleanup();
        T.push("-- no toggle handler");
        mount(md("x", { metadata: { canceled: true } }), { onTogglePin: false });
        fireEvent.click(host.querySelector("[data-stand='Button']")!);
        return snapshot("fold");
    });

    it("shows the peek only after hover, subscribes to the ticker only while its panel is up, and shows time and token estimate (or only one of them)", () => {
        mount(md("Some text that is long enough to count", { timestamp: 5000 }));
        log("-- before hover");
        look("not hovered");
        const anchor = () => host.querySelector<HTMLElement>(".agent-markdown-peek-anchor")!;
        fireEvent.mouseEnter(anchor());
        log("mouse entered; waiting out the enter delay");
        at(1030);
        log("(not yet)");
        at(1200);
        look("hovered");
        h.tick[1](1);
        log("ticker fired once");
        fireEvent.mouseLeave(anchor());
        look("left");
        h.tick[1](2);
        log("ticker fired while no panel");
        cleanup();
        T.push("-- no timestamp");
        mount(md("Some text that is long enough to count"));
        fireEvent.mouseEnter(anchor());
        at(1400);
        look("hovered, no timestamp");
        cleanup();
        T.push("-- empty text, with a timestamp");
        mount(md("", { timestamp: 7000 }));
        fireEvent.mouseEnter(anchor());
        at(1600);
        look("hovered, empty text");
        cleanup();
        T.push("-- empty text, no timestamp");
        mount(md(""));
        fireEvent.mouseEnter(anchor());
        at(1800);
        look("hovered, nothing to show");
        return snapshot("peek");
    });

    it("stops its settle timer when unmounted, so nothing renders afterwards", () => {
        mount(md("x"));
        at(1100);
        chunk(1100, md("xy"));
        unmount();
        log("unmounted with a settle pending");
        at(2000);
        log("(nothing rendered after unmount)");
        return snapshot("unmount");
    });
});
