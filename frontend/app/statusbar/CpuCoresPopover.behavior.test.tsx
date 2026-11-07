// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CpuCoresPopover (the per-core CPU panel under the status bar's CPU readout), pinned as a whole ahead of splitting it
 * into a state object and smaller views: each scenario mounts the real popover with stand-ins for the anchored popover
 * and the system-info event, feeds it per-core readings, hovers, focuses and arrows through the cores, and logs the
 * panel as an indented tree (inline styles included) and, in order, every call it makes. The logs are snapshots
 * recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({ emit: null as any }));

vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (event: string, cb: (data: any) => void, scope: string) => {
        log(`onMuxEvent ${event} ${scope}`);
        h.emit = cb;
        return () => log("unsubscribe");
    },
}));
vi.mock("@/app/store/mps-events", () => ({ WpsEvent: { SysInfo: "sysinfo" } }));
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (p: {
        placement?: string;
        class?: string;
        role?: JSX.HTMLAttributes<HTMLDivElement>["role"];
        "aria-label"?: string;
        style?: JSX.CSSProperties;
        onDismiss: () => void;
        children?: JSX.Element;
    }) => (
        <div
            data-stand="AnchoredPopover"
            data-placement={p.placement}
            class={p.class}
            role={p.role}
            aria-label={p["aria-label"]}
            style={p.style}
            onClick={() => (log("onDismiss"), p.onDismiss())}
        >
            {p.children}
        </div>
    ),
}));

import { CpuCoresPopover } from "./CpuCoresPopover";

const SNAP = (name: string) => `./__snapshots__/CpuCoresPopover.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS =
    /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder|tabindex)$/;

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

let host: HTMLElement;
let unmount: () => void;

const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
/** A sysinfo event with the given per-core readings (and anything else a real one carries). */
const send = (cores: number[], extra: Record<string, number> = {}) => {
    const values: Record<string, number> = { cpu: 42.5, mem: 8, "cpu:x": 99, ...extra };
    cores.forEach((pct, i) => (values[`cpu:${i}`] = pct));
    log(`sysinfo ${cores.length} cores`);
    h.emit({ values });
};
const ramp = (n: number) => Array.from({ length: n }, (_, i) => (i * 100) / Math.max(1, n - 1));
const active = () => {
    const el = document.activeElement as HTMLElement | null;
    return el?.dataset?.coreIdx ? `focus: core ${el.dataset.coreIdx}` : "focus: elsewhere";
};
const squares = () => [...host.querySelectorAll<HTMLElement>(".cpu-core-square")];
const readout = () => host.querySelector(".cpu-cores-subtitle > span")?.textContent;

function mount() {
    const view = render(() => <CpuCoresPopover anchor={null} onClose={() => log("onClose")} />, {
        container: document.body.appendChild(document.createElement("div")),
    });
    host = view.container;
    unmount = view.unmount;
}

beforeEach(() => {
    T.length = 0;
    h.emit = null;
});
afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
});

describe("CpuCoresPopover, pinned", () => {
    it("subscribes to the local system info on mount, says it is reading until the first event, passes a click to onDismiss, and unsubscribes on unmount", () => {
        mount();
        look("before any reading");
        fireEvent.click(host.querySelector("[data-stand='AnchoredPopover']")!);
        send([10, 20]);
        look("first reading");
        unmount();
        log("unmounted");
        return snapshot("lifecycle");
    });

    it("lists up to 16 cores as labelled rows in index order (ignoring other keys), caps the bar at 100%, colours the average, and has the readout follow the hovered core live", () => {
        mount();
        log("sysinfo (unordered keys)");
        h.emit({ values: { "cpu:2": 57.4, cpu: 96, "cpu:0": 3, "cpu:1": 130, "cpu:x": 5, "gpu:0": 1 } });
        look("three cores, average above 95");
        const row = (i: number) => host.querySelectorAll<HTMLElement>(".cpu-core")[i];
        fireEvent.mouseEnter(row(2));
        log(`readout: ${readout()}`);
        send([5, 6, 70.6], { cpu: 85 });
        log(`readout after a tick: ${readout()}`);
        look("hovering core 2 after a tick, average above 80");
        fireEvent.mouseLeave(row(2));
        log(`readout after leaving: ${readout()}`);
        send([5, 6, 70.6], { cpu: 10 });
        fireEvent.mouseEnter(row(1));
        h.emit({ values: { "cpu:0": 1, cpu: 10 } });
        log(`readout when the hovered core is gone: ${readout()}`);
        return snapshot("rows");
    });

    it("switches tiers at 16/17 and 64/65 cores: rows (260px), cells (360px, short labels, grid), heatmap (360px, legend)", () => {
        mount();
        for (const n of [16, 17, 64, 65, 16]) {
            send(ramp(n));
            const c = host.querySelector("[data-stand='AnchoredPopover']")!;
            log(
                `${n} cores: class=${c.className} width=${(c as HTMLElement).style.width} list=${
                    host.querySelector(".cpu-cores-list")?.className ?? "none"
                } heatmap=${host.querySelector(".cpu-cores-heatmap") ? "yes" : "no"} legend=${
                    host.querySelector(".cpu-cores-legend") ? "yes" : "no"
                }`
            );
        }
        send(ramp(20));
        look("twenty cores as cells");
        return snapshot("tiers");
    });

    it("sizes the heatmap squares to the largest that fit the target height, down to the minimum", () => {
        mount();
        for (const n of [65, 128, 129, 200, 256, 400, 600, 700, 2000]) {
            send(ramp(n));
            const g = host.querySelector<HTMLElement>(".cpu-cores-heatmap")!;
            log(`${n} cores: --cols=${g.style.getPropertyValue("--cols")} --sq=${g.style.getPropertyValue("--sq")}`);
        }
        return snapshot("heat-sizing");
    });

    it("draws the heatmap squares with one tab stop, shows a hovered or focused core in the readout, and clears it on leave and blur", () => {
        mount();
        send(ramp(70));
        const sq = squares();
        log(`squares: ${sq.length}, tab stops: ${sq.filter((s) => s.tabIndex === 0).length}`);
        look("heatmap, first squares only");
        fireEvent.mouseEnter(sq[5]);
        log(`hover 5: ${readout()}`);
        fireEvent.mouseLeave(sq[5]);
        log(`leave: ${readout()}`);
        fireEvent.focus(sq[9]);
        log(`focus 9: ${readout()}, tab stop on ${squares().findIndex((s) => s.tabIndex === 0)}`);
        fireEvent.blur(sq[9]);
        log(`blur: ${readout()}, tab stop still on ${squares().findIndex((s) => s.tabIndex === 0)}`);
        return snapshot("heat-focus");
    });

    it("moves the heatmap focus with the arrow keys by one or by a row, to Home and End, stops at the ends, and ignores other keys", () => {
        mount();
        send(ramp(70));
        const grid = host.querySelector<HTMLElement>(".cpu-cores-heatmap")!;
        const cols = Number(grid.style.getPropertyValue("--cols"));
        log(`cols ${cols}`);
        const press = (key: string) => {
            const ev = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
            grid.dispatchEvent(ev);
            log(
                `${key}: prevented=${ev.defaultPrevented} readout=${readout()} tab stop=${squares().findIndex(
                    (s) => s.tabIndex === 0
                )} ${active()}`
            );
        };
        for (const key of ["ArrowRight", "ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp", "ArrowUp", "ArrowLeft", "End", "ArrowRight", "ArrowDown", "Home", "ArrowLeft", "ArrowUp", "a", "Tab", "Enter"]) {
            press(key);
        }
        return snapshot("heat-keys");
    });

    it("keeps the tab stop inside a shrinking list, leaves it alone when the list grows, and ignores keys before any reading", () => {
        mount();
        const grid0 = () => host.querySelector<HTMLElement>(".cpu-cores-heatmap");
        send(ramp(80));
        const grid = grid0()!;
        grid.dispatchEvent(new KeyboardEvent("keydown", { key: "End", bubbles: true, cancelable: true }));
        log(`End on 80: tab stop ${squares().findIndex((s) => s.tabIndex === 0)} ${active()}`);
        send(ramp(70));
        log(`shrunk to 70: tab stop ${squares().findIndex((s) => s.tabIndex === 0)}`);
        send(ramp(90));
        log(`grown to 90: tab stop ${squares().findIndex((s) => s.tabIndex === 0)}`);
        send(ramp(10));
        log(`shrunk to 10 (rows): tab stop state kept, list rows ${host.querySelectorAll(".cpu-core").length}`);
        send(ramp(70));
        log(`back to 70: tab stop ${squares().findIndex((s) => s.tabIndex === 0)}`);
        send([]);
        look("no cores any more");
        log(`heatmap present: ${grid0() ? "yes" : "no"}`);
        grid.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true, cancelable: true }));
        log(`key on a stale grid ignored: readout ${readout()}`);
        return snapshot("roving-clamp");
    });
});
