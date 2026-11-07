// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PairDevicePanel ("Pair a device" in the host popover), pinned as a whole ahead of splitting it into a state object
 * and view parts: each scenario mounts the real panel over stand-ins for the kit's button, the pairing RPC, the
 * clipboard, the paired-device signal and the QR library, on a fake clock, drives the buttons and the clock, and logs
 * the panel as an indented tree and, in order, every call it makes (the pairing it starts, the code it draws, the
 * text it copies, what it logs). The logs are snapshots recorded on the unsplit component; the split must leave them
 * byte for byte. (PairDevicePanel.test.tsx asserts what the person sees against the real kit.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type Accessor, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const [paired, setPaired] = createSignal<{ deviceId: string; deviceName: string } | null>(null);
const h = vi.hoisted(() => ({
    start: null as any,
    copy: null as any,
    qr: null as any,
}));

vi.mock("@/app/element/ui", () => ({
    Button: (p: { density?: string; disabled?: boolean; onClick?: () => void; children?: JSX.Element }) => (
        <span
            data-stand="Button"
            data-density={p.density}
            data-disabled={String(p.disabled)}
            onClick={() => {
                log(`Button ${JSON.stringify(p.children)} click`);
                if (!p.disabled) p.onClick?.();
            }}
        >
            {p.children}
        </span>
    ),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ViewerPairStartCommand: () => (log("ViewerPairStartCommand"), h.start()) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/clipboard", () => ({ writeText: (text: string) => (log(`writeText ${text}`), h.copy()) }));
vi.mock("@/store/global", () => ({ viewerPairedAtom: () => paired() }));
vi.mock("qrcode", () => ({
    default: {
        toCanvas: (canvas: HTMLCanvasElement, url: string, opts: unknown, cb: (err?: Error) => void) => {
            log(`toCanvas ${canvas.tagName.toLowerCase()} ${url} ${JSON.stringify(opts)}`);
            h.qr(cb);
        },
    },
}));

import { PairDevicePanel } from "./PairDevicePanel";

const SNAP = (name: string) => `./__snapshots__/PairDevicePanel.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type)$/;

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

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
    // The QR library arrives through a dynamic import; the timers it needs are not faked.
    await new Promise((resolve) => setTimeout(resolve, 5));
};
const click = (text: string) => {
    const b = [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")].find(
        (x) => x.textContent?.trim() === text
    );
    fireEvent.click(b!);
};
const URL_ = "agentmux://pair?v=1&host=198.51.100.7&code=ABCDEFGH23";
const NOW = 1_800_000_000_000;
/** The clock moves a second at a time, as the panel's own timer does. */
const tick = (seconds: number) => {
    for (let i = 0; i < seconds; i++) vi.advanceTimersByTime(1000);
};
const defer = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
    return { promise, resolve, reject };
};

function mount(lan: Accessor<boolean> = () => true) {
    const view = render(() => <PairDevicePanel lanDiscoveryEnabled={lan} />);
    host = view.container;
    return view;
}

beforeEach(() => {
    T.length = 0;
    setPaired(null);
    h.start = async () => ({ url: URL_, expires_ms: Date.now() + 120_000 });
    h.copy = async () => {};
    h.qr = (cb: (err?: Error) => void) => cb();
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
    vi.setSystemTime(NOW);
    vi.spyOn(console, "error").mockImplementation((...a: unknown[]) => log(`console.error ${a.map(String).join(" ")}`));
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
});

describe("PairDevicePanel, pinned", () => {
    it("starts a pairing: the button is off while LAN discovery is, busy while the code is asked for, then a QR drawn from the one-time URL with a countdown that runs out", async () => {
        const [lan, setLan] = createSignal(false);
        mount(lan);
        look("LAN discovery off");
        setLan(true);
        look("LAN discovery on");
        const out = defer<any>();
        h.start = () => out.promise;
        click("Pair a device");
        look("asking for a code");
        click("Pair a device");
        out.resolve({ url: URL_, expires_ms: Date.now() + 120_000 });
        await flush();
        look("the code is up");
        tick(61);
        look("a minute on");
        tick(58);
        look("the last second");
        tick(2);
        look("expired");
        return snapshot("start");
    });

    it("hides the code, asks for a new one, copies the link (and says so, until a new code), and refuses to copy an expired code or when the clipboard fails", async () => {
        mount();
        click("Pair a device");
        await flush();
        click("Copy link");
        await flush();
        look("copied");
        click("New code");
        await flush();
        look("a new code: copy is offered again");
        h.copy = async () => {
            throw new Error("denied");
        };
        click("Copy link");
        await flush();
        look("the clipboard failed");
        click("Hide");
        look("hidden, the error stays");
        click("Pair a device");
        await flush();
        look("a code again, the error cleared");
        tick(121);
        click("Copy link");
        look("expired: copy is off and does nothing");
        return snapshot("copy");
    });

    it("shows why a pairing didn't start (an error's message, or plain text) and drops it on the next try", async () => {
        h.start = async () => {
            throw new Error("viewer is off");
        };
        mount();
        click("Pair a device");
        await flush();
        look("an error");
        h.start = async () => {
            throw "plain refusal";
        };
        click("Pair a device");
        await flush();
        look("plain text");
        h.start = async () => ({ url: URL_, expires_ms: Date.now() + 120_000 });
        click("Pair a device");
        await flush();
        look("working again");
        return snapshot("errors");
    });

    it("takes the QR down and names the device when one pairs while it is up, ignores a device that pairs with no code up, and clears the note on the next code; and turning LAN discovery off takes the code down", async () => {
        const [lan, setLan] = createSignal(true);
        mount(lan);
        setPaired({ deviceId: "d0", deviceName: "Early phone" });
        look("a device paired with no code up: nothing");
        click("Pair a device");
        await flush();
        setPaired({ deviceId: "d1", deviceName: "Pat's phone" });
        look("paired while the code was up");
        click("Pair a device");
        await flush();
        look("a new code: the note is gone");
        setLan(false);
        look("LAN discovery turned off");
        return snapshot("paired");
    });

    it("loads the QR library with the first code, says so when it can't draw or can't load, offers to try again", async () => {
        h.qr = (cb: (err?: Error) => void) => cb(new Error("canvas is gone"));
        mount();
        click("Pair a device");
        await flush();
        look("the draw failed");
        cleanup();
        T.push("-- the library can't load");
        h.qr = () => {
            throw new Error("no such chunk");
        };
        mount();
        click("Pair a device");
        await flush();
        look("the library failed");
        h.qr = (cb: (err?: Error) => void) => cb();
        click("Try again");
        await flush();
        look("tried again: the note is gone");
        return snapshot("qr");
    });
});
