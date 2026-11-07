// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DevicesSection (Settings › Devices), pinned as a whole ahead of splitting it into a state object and view parts:
 * each scenario mounts the real section over a stand-in domain store and RPC whose answers the scenario controls,
 * with stand-ins for the kit's button and switch, the settings rows and toggles, the sibling sections and the
 * settings signal, on a fixed clock, drives the switches and buttons, and logs the section as an indented tree and,
 * in order, every call it makes (the store following and holding a value, each device command, each setting written).
 * The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (devices-section.test.tsx asserts what the person sees against the real kit; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const [devices, setDevices] = createSignal<any[] | null>(null);
const [storeError, setStoreError] = createSignal<string | null>(null);
const [settings, setSettings] = createSignal<Record<string, unknown>>({});
const h = vi.hoisted(() => ({
    opts: null as any,
    revoke: null as any,
    canSend: null as any,
}));

vi.mock("@/app/store/domain-store", () => ({
    createDomainStore: (opts: any) => {
        h.opts = opts;
        return {
            data: () => devices(),
            error: () => storeError(),
            loaded: () => devices() !== null,
            set: (v: unknown) => (log(`store.set ${JSON.stringify(v)}`), setDevices(() => v as any[])),
            refresh: async () => {},
            follow: () => log("store.follow"),
        };
    },
}));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => settings() }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ViewerDevicesCommand: () => (
            log("ViewerDevicesCommand"),
            Promise.resolve({ devices: [{ device_id: "loaded" }] })
        ),
        ViewerRevokeCommand: (_c: unknown, req: unknown) => (
            log(`ViewerRevokeCommand ${JSON.stringify(req)}`),
            h.revoke()
        ),
        ViewerCanSendCommand: (_c: unknown, req: unknown) => (
            log(`ViewerCanSendCommand ${JSON.stringify(req)}`),
            h.canSend()
        ),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/element/tip", () => ({ tip: (text: string) => ({ "data-tip": text }) }));
vi.mock("@/app/element/ui", () => ({
    Button: (p: { tone?: string; disabled?: boolean; onClick?: () => void; children?: JSX.Element }) => (
        <span
            data-stand="Button"
            data-tone={p.tone}
            data-disabled={String(p.disabled)}
            onClick={() => (log(`Button ${JSON.stringify(p.children)} click`), p.disabled || p.onClick?.())}
        >
            {p.children}
        </span>
    ),
    Switch: (p: { checked: boolean; disabled?: boolean; ariaLabel?: string; onChange: (v: boolean) => void }) => (
        <span
            data-stand="Switch"
            data-checked={String(p.checked)}
            data-disabled={String(p.disabled)}
            data-aria-label={p.ariaLabel}
            onClick={() => (log(`Switch ${p.ariaLabel} click`), p.disabled || p.onChange(!p.checked))}
        />
    ),
}));
vi.mock("../settings-controls", () => ({
    set: (key: string, value: unknown) => log(`set ${key} ${JSON.stringify(value)}`),
    SectionHeader: (p: { label: string }) => <h3 data-stand="SectionHeader">{p.label}</h3>,
    SettingRow: (p: { id?: string; label: string; description?: string; control: JSX.Element }) => (
        <div data-stand="SettingRow" data-id={p.id} data-label={p.label} data-description={p.description}>
            {p.control}
        </div>
    ),
    ToggleControl: (p: { checked: boolean; onChange: (v: boolean) => void }) => (
        <span
            data-stand="ToggleControl"
            data-checked={String(p.checked)}
            onClick={() => (log(`ToggleControl ${p.checked} click`), p.onChange(!p.checked))}
        />
    ),
}));
vi.mock("./cloud-presence", () => ({
    CLOUD_PRESENCE_SETTING: { id: "devices.cloud_presence", label: "Cloud presence" },
    CloudPresence: () => <div data-stand="CloudPresence" />,
}));
vi.mock("./hidden-agents", () => ({
    HIDDEN_AGENTS_SETTING: { id: "devices.hidden_agents", label: "Hidden agents" },
    HiddenAgents: () => <div data-stand="HiddenAgents" />,
}));
vi.mock("./muxbus-sign-in", () => ({
    MUXBUS_SIGN_IN_SETTING: { id: "devices.muxbus", label: "MuxBus sign-in" },
    MuxBusSignIn: () => <div data-stand="MuxBusSignIn" />,
}));
vi.mock("./pinned-senders", () => ({
    PINNED_SENDERS_SETTING: { id: "devices.pinned_senders", label: "Pinned senders" },
    PinnedSenders: () => <div data-stand="PinnedSenders" />,
}));

import { DevicesSection } from "./devices-section";

const SNAP = (name: string) => `./__snapshots__/devices-section.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id)$/;

/** The section as an indented tree: tags, classes, the attributes that carry state, and text. */
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
};
const defer = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
    return { promise, resolve, reject };
};
const click = (label: string, nth = 0) => {
    const found = [...host.querySelectorAll<HTMLElement>("[data-stand='Button'],[data-stand='Switch']")].filter(
        (x) => x.dataset.ariaLabel === label || x.textContent?.trim() === label
    );
    fireEvent.click(found[nth]);
};
const toggle = (id: string) => fireEvent.click(host.querySelector(`[data-id='${id}'] [data-stand='ToggleControl']`)!);

const NOW = 1_800_000_000_000;
const MIN = 60_000;
const HOUR = 3_600_000;
const dev = (id: string, name: string, extra: Record<string, unknown> = {}) => ({
    device_id: id,
    device_name: name,
    created_ms: NOW - 2 * HOUR,
    last_seen_ms: NOW - 10 * MIN,
    has_key: true,
    can_send: false,
    ...extra,
});
const LIST = () => [
    dev("d1", "Pixel 9", { can_send: true }),
    dev("d2", "Tablet", { created_ms: NOW - 30_000, last_seen_ms: 0 }),
    dev("d3", "Old phone", { has_key: false, created_ms: NOW - 5 * MIN, last_seen_ms: NOW - 20_000 }),
];

function mount() {
    const view = render(() => <DevicesSection />);
    host = view.container;
    return view;
}

beforeEach(() => {
    T.length = 0;
    setDevices(null);
    setStoreError(null);
    setSettings({});
    h.revoke = async () => ({});
    h.canSend = async (): Promise<any> => ({ can_send: true });
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(NOW);
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("DevicesSection, pinned", () => {
    it("follows the store, says it is loading until the devices arrive (and shows the load's error beside it), says when none are paired, and loads the devices from srv", async () => {
        mount();
        look("loading");
        setStoreError("srv is down");
        look("the load failed");
        setStoreError(null);
        setDevices([]);
        look("none paired");
        log(`store: events=${JSON.stringify(h.opts.events)} initial=${JSON.stringify(h.opts.initial)}`);
        log(`load -> ${JSON.stringify(await h.opts.load())}`);
        return snapshot("load");
    });

    it("lists each device with when it paired and was last seen (or never), a switch for Can send, 'Pair again' for a device with no key, and Revoke", async () => {
        setDevices(LIST());
        mount();
        look("three devices");
        return snapshot("list");
    });

    it("turns Can send on or off for a device: disabled while it is out, the list updated from srv's answer, and the failure shown (and dropped by the next success)", async () => {
        setDevices(LIST());
        mount();
        const out = defer<any>();
        h.canSend = () => out.promise;
        click("Pixel 9 can send messages");
        look("changing Pixel 9");
        click("Pixel 9 can send messages");
        out.resolve({ can_send: false });
        await flush();
        look("Pixel 9 can no longer send");
        h.canSend = async () => {
            throw new Error("not allowed");
        };
        click("Pixel 9 can send messages");
        await flush();
        look("the change failed");
        h.canSend = async () => {
            throw "plain refusal";
        };
        click("Pixel 9 can send messages");
        await flush();
        look("a plain refusal");
        h.canSend = async () => ({ can_send: true });
        click("Pixel 9 can send messages");
        await flush();
        look("working again");
        return snapshot("can-send");
    });

    it("revokes a device: 'Revoking…' and off while it is out, the device gone from the list, and the failure shown", async () => {
        setDevices(LIST());
        mount();
        const out = defer<any>();
        h.revoke = () => out.promise;
        click("Revoke", 1);
        look("revoking Tablet");
        click("Revoking…");
        out.resolve({});
        await flush();
        look("Tablet revoked");
        h.revoke = async () => {
            throw new Error("srv refused");
        };
        click("Revoke", 0);
        await flush();
        look("the revoke failed");
        h.revoke = async () => {
            throw "plain refusal";
        };
        click("Revoke", 0);
        await flush();
        look("a plain refusal");
        h.revoke = async () => ({});
        click("Revoke", 0);
        await flush();
        look("revoked, the error gone");
        return snapshot("revoke");
    });

    it("shows the section's other settings with their switches from the settings (off is the absent key for terminals, on is the default for the rest) and writes them back", async () => {
        setDevices([]);
        mount();
        look("defaults");
        toggle("devices.share_terminals");
        toggle("devices.publish_presence");
        toggle("devices.share_summaries");
        toggle("devices.statusbar_muxbus_cloud");
        setSettings({
            "viewer:shareterminals": true,
            "cloud:publishpresence": false,
            "swarm:sharesummaries": false,
            "statusbar:showmuxbuscloud": false,
        });
        look("all turned the other way");
        toggle("devices.share_terminals");
        toggle("devices.publish_presence");
        return snapshot("settings");
    });
});
