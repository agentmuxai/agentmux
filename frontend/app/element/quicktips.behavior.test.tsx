// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * QuickTips (the Help pane's content), pinned as a whole ahead of splitting it into a filter model and view parts:
 * each scenario mounts the real component with stand-ins for the shortcut table and the hidden tips (so the lists
 * are small and the same on every platform), types into the filter and logs the pane as an indented tree and, in
 * order, every lookup it makes. The logs are snapshots recorded on the unsplit component; the split must leave them
 * byte for byte. (quicktips.test.tsx asserts what the person finds; this pins the markup and the lookups.)
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

vi.mock("@/app/keybindings", () => ({
    keyPlatform: () => (log("keyPlatform"), "mac"),
    shortcutFor: (command: string) => (log(`shortcutFor ${command}`), command === "pane:changeConnection" ? "" : `Ctrl+${command.split(":")[1]}`),
    shortcutHelp: () => (
        log("shortcutHelp"),
        [
            { category: "Panes", entries: [{ label: "Close the pane", keys: ["Ctrl+W"] }, { label: "Split right", keys: ["⌘D", "Ctrl+Shift+D"] }] },
            { category: "Tabs", entries: [{ label: "New tab", keys: ["⌘T"] }] },
            { category: "Search", entries: [] },
        ]
    ),
}));
vi.mock("@/app/keybindings/tips", () => ({
    tipsFor: (os: string) => (
        log(`tipsFor ${os}`),
        [
            { area: "Panes", id: "drag", where: "on the header" },
            { area: "Panes", id: "swap" },
            { area: "Terminal", id: "zoom" },
        ]
    ),
    gestureLabels: (tip: { id: string }, platform: string) => (
        log(`gestureLabels ${tip.id} ${platform}`),
        tip.id === "drag" ? [["Drag"], ["Alt+Drag", "Shift"]] : tip.id === "swap" ? [["⌘Click"]] : [["Ctrl+Wheel"]]
    ),
    tipLabel: (tip: { id: string }, platform: string) => (log(`tipLabel ${tip.id} ${platform}`), `Gesture ${tip.id}`),
}));
vi.mock("@/util/platformutil", () => ({ getPlatform: () => (log("getPlatform"), "darwin") }));

import { QuickTips } from "./quicktips";

const SNAP = (name: string) => `./__snapshots__/quicktips.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|href|target|rel)$/;

/** The pane as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
let setFilter: (v: string | undefined) => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const type = (v: string | undefined, label: string) => {
    log(`filter ${JSON.stringify(v)}`);
    setFilter(v);
    look(label);
};

function mount(initial?: string) {
    const [filter, set] = createSignal<string | undefined>(initial);
    setFilter = set;
    host = render(() => <QuickTips filter={filter()} />).container;
}

beforeEach(() => {
    T.length = 0;
});
afterEach(cleanup);

describe("QuickTips, pinned", () => {
    it("lays out every card and the alpha notice when nothing is typed, and builds its lookups in order", () => {
        mount();
        look("unfiltered");
        type("  ", "blank filter is no filter");
        type(undefined, "no filter again");
        return snapshot("layout");
    });

    it("narrows each card to what matches the words typed, by title, text, keys or section name, and drops the alpha notice", () => {
        mount("close");
        look("close");
        type("ctrl w", "keys of a shortcut");
        type("panes drag", "section and gesture");
        type("web view gear", "a tip");
        type("discord", "a link");
        type("header icons", "section name of the icons");
        return snapshot("filter");
    });

    it("says nothing in Help matches when every card is empty, quoting what was typed without its padding", () => {
        mount("zzz");
        look("nothing");
        type("  qqq  ", "padded");
        type("", "cleared");
        return snapshot("nothing");
    });
});
