// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DiagPanel (the dev-mode reducer dispatch ring), pinned as a whole ahead of splitting it into a state object and
 * smaller view functions: each scenario mounts the real panel with stand-ins for the dispatch ring, the clock, the
 * shortcut table, the key command table, the perf section and the kit's button and text field, toggles it the way the
 * shortcut does, types into its filters, and logs the panel as an indented tree (inline styles included) and, in
 * order, every call it makes. The logs are snapshots recorded on the unsplit component; the split must leave them
 * byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({ tick: null as any, records: null as any, keyCommands: null as any }));

vi.mock("@/app/hook/useTick", () => ({ useTick: (ms: number) => (log(`useTick ${ms}`), h.tick[0]) }));
vi.mock("@/store/command-source", () => ({
    dispatchRecordsAtom: () => h.records[0](),
    describeSource: (s: string) => `src:${s}`,
    __resetDispatchLog: () => (log("__resetDispatchLog"), h.records[1]([])),
}));
vi.mock("./agent-pane-perf-section", () => ({ AgentPanePerfSection: () => <div data-stand="AgentPanePerfSection" /> }));
vi.mock("@/app/keybindings", () => ({
    shortcutFor: (command: string) => (log(`shortcutFor ${command}`), `<${command}>`),
}));
vi.mock("@/app/store/keymodel-dispatch", () => {
    class LogMap extends Map<string, () => boolean> {
        set(k: string, v: () => boolean) {
            log(`keyCommands.set ${k}`);
            return super.set(k, v);
        }
        delete(k: string) {
            log(`keyCommands.delete ${k}`);
            return super.delete(k);
        }
    }
    h.keyCommands = new LogMap();
    return { keyCommands: h.keyCommands };
});
function kit() {
    return {
        Button: (p: { density?: string; title?: string; onClick: () => void; children?: JSX.Element }) => (
            <span
                data-stand="Button"
                data-density={p.density}
                data-title={p.title}
                onClick={() => (log("Button onClick"), p.onClick())}
            >
                {p.children}
            </span>
        ),
        TextInput: (p: {
            variant?: string;
            placeholder?: string;
            value?: string;
            style?: JSX.CSSProperties;
            onInput: (e: any) => void;
        }) => (
            <input
                data-stand="TextInput"
                data-variant={p.variant}
                data-placeholder={p.placeholder}
                data-value={p.value}
                style={p.style}
                onInput={p.onInput}
            />
        ),
    };
}
vi.mock("@/element/ui", kit);
vi.mock("@/app/element/ui", kit);

import { DiagPanel } from "./diag-panel";

const SNAP = (name: string) => `./__snapshots__/diag-panel.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder)$/;

/** The panel as an indented tree: tags, classes, the attributes that carry state (styles included), and text. */
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

const T0 = 1_800_000_000_000;
let host: HTMLElement;
let unmount: () => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
    const select = host.querySelector("select");
    if (select) log(`select.value=${JSON.stringify(select.value)}`);
};
const advance = (ms: number) => {
    vi.advanceTimersByTime(ms);
    h.tick[1]((n: number) => n + 1);
};
const toggle = () => {
    log("toggle shortcut");
    log(`handler returned ${h.keyCommands.get("dev:diagnostics")()}`);
};
const rec = (
    slice: string,
    key: string | undefined,
    command: unknown,
    source: string,
    events: number,
    agoMs: number
) => ({
    slice,
    key,
    command,
    source,
    events: Array.from({ length: events }, () => ({})),
    at: Date.now() - agoMs,
});

function mount() {
    const view = render(() => <DiagPanel />);
    host = view.container;
    unmount = view.unmount;
}

beforeEach(() => {
    T.length = 0;
    h.tick = createSignal(0);
    h.records = createSignal<any[]>([]);
    h.keyCommands.clear();
    T.length = 0;
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "Date"] });
    vi.setSystemTime(T0);
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("DiagPanel, pinned", () => {
    it("registers its shortcut on mount, toggles open and shut with it, says none were recorded, and drops the shortcut on unmount", () => {
        mount();
        look("hidden");
        toggle();
        look("open, nothing recorded");
        toggle();
        look("shut again");
        unmount();
        log(`keyCommands has it: ${h.keyCommands.has("dev:diagnostics")}`);
        return snapshot("toggle");
    });

    it("lists the newest dispatches first with their age, slice, short key, command, source and event count, and ticks only the ages", () => {
        h.records[1]([
            rec("layout", "abcdefghij", { type: "Navigate" }, "keyboard", 2, 59_000 + 31),
            rec("browser-pane-state", undefined, { type: 7 }, "menu", 0, 400),
            rec("agent-pane-state", "k2", "not-an-object", "rpc", 3, 125_000),
            rec("layout", "ZZZ", null, "keyboard", 1, 5),
            rec("launcher-event", "kkkkkkk-long", { type: "PaneClicked" }, "mouse", 4, 2_500),
        ]);
        mount();
        toggle();
        look("five records");
        advance(1_000);
        look("a second later: only the ages move");
        return snapshot("rows");
    });

    it("filters by slice (the dropdown lists the slices seen, sorted) and by key (trimmed, case-insensitive), says when nothing matches, and clears the ring", () => {
        h.records[1]([
            rec("layout", "AbC123", { type: "A" }, "keyboard", 1, 10),
            rec("browser-pane-state", "xyz999", { type: "B" }, "menu", 1, 20),
            rec("layout", undefined, { type: "C" }, "rpc", 1, 30),
        ]);
        mount();
        toggle();
        const select = () => host.querySelector("select")!;
        const input = () => host.querySelector("input")!;
        select().value = "layout";
        fireEvent.input(select());
        look("slice: layout");
        input().value = "  abc ";
        fireEvent.input(input());
        look("slice layout, key abc");
        select().value = "browser-pane-state";
        fireEvent.input(select());
        look("slice browser, key abc: nothing matches");
        select().value = "";
        fireEvent.input(select());
        input().value = "";
        fireEvent.input(input());
        look("filters cleared");
        h.records[1]((r: any[]) => [...r, rec("a-new-slice", "n", { type: "D" }, "rpc", 1, 5)]);
        look("a new slice appears in the dropdown");
        fireEvent.click(host.querySelector("[data-stand='Button']")!);
        look("after clear");
        return snapshot("filters");
    });

    it("shows at most the newest 80 rows of a long ring", () => {
        h.records[1](
            Array.from({ length: 95 }, (_, i) => rec("layout", `k${i}`, { type: `C${i}` }, "keyboard", i % 3, 1000 - i))
        );
        mount();
        toggle();
        log(`rows: ${host.querySelectorAll("tbody tr").length}`);
        log(`first: ${host.querySelector("tbody tr")?.textContent}`);
        log(`last: ${[...host.querySelectorAll("tbody tr")].at(-1)?.textContent}`);
        log(`total shown in the header: ${host.textContent?.match(/\((\d+) total/)?.[1]}`);
        return snapshot("limit");
    });
});
