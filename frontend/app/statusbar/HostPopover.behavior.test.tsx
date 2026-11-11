// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The host popover's panel, pinned as a whole ahead of splitting it into a state object and view parts: each
 * scenario feeds it the host's details, LAN state and MuxBus status through signals (so changes land too) and logs
 * the panel as a tree and, in order, every call its controls make. The popover, the peer list, the pairing panel and
 * the sign-out button are stand-ins that print what they are given. The logs are snapshots recorded on the unsplit
 * component; the split must leave them byte for byte. (HostPopover.test.tsx asserts what the person is told; this
 * pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    platform: "win" as "win" | "mac" | "linux",
    devMode: true,
    openDir: (async () => {}) as () => Promise<void>,
    dismiss: () => {},
}));

vi.mock("@/store/global", () => ({
    getHostName: () => "narko",
    getApi: () => ({
        openDataDirInFileManager: () => {
            h.log.push("host openDataDirInFileManager");
            return h.openDir();
        },
    }),
    lanInstancesAtom: () => [],
    lanDiscoveryErrorAtom: () => null,
    lanDiscoverabilityAtom: () => null,
    lanFirewallAtom: () => null,
    setLanDiscoveryErrorAtom: () => {},
    settingsAtom: () => ({}),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/dev-mode", () => ({ devModeOn: () => h.devMode }));
vi.mock("@/app/view/accounts/AgentMuxConnectPanel", () => ({ useMuxBusStatus: () => ({}) }));
vi.mock("@/util/platformutil", () => ({
    isMacOS: () => h.platform === "mac",
    isLinux: () => h.platform === "linux",
    isWindows: () => h.platform === "win",
}));
vi.mock("@/app/element/anchored-popover", () => ({
    AnchoredPopover: (props: any) => {
        h.dismiss = props.onDismiss;
        h.log.push(
            `popover placement=${props.placement} role=${props.role} aria-label=${JSON.stringify(props["aria-label"])} class=${JSON.stringify(props.class)} anchor=${props.anchor?.id ?? props.anchor}`,
        );
        return (
            <div data-popover="" role={props.role} aria-label={props["aria-label"]} class={props.class} style={props.style}>
                {props.children}
            </div>
        );
    },
}));
vi.mock("./LanPeerRow", () => ({
    LanPeerList: (props: any) => (
        <ul data-peers={JSON.stringify(props.peers.map((p: any) => p.name))} data-self={props.selfHost} />
    ),
}));
vi.mock("./PairDevicePanel", () => ({
    PairDevicePanel: (props: any) => <div data-pair-device="" data-lan={String(props.lanDiscoveryEnabled())} />,
}));
vi.mock("@/app/view/accounts/muxbus-sign-out", () => ({
    MuxBusSignOutButton: (props: any) => (
        <button data-signout="" class={props.class} onClick={() => h.log.push("muxbus sign out clicked")}>
            {props.label}
        </button>
    ),
}));

import { HostPopoverPanel } from "./HostPopover";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/HostPopover.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href|style|checked)$/;
/** The panel as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim().replace(/\s+/g, " ");
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n"));
};
const flush = async () => {
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
};

const INFO = {
    hostname: "NARKO",
    os: "Windows x86_64",
    localIp: "203.0.113.20",
    version: "0.60.17",
    dataDir: "C:\\Users\\me\\.agentmux\\channels\\stable\\data",
    pid: 4242,
    ports: { ipc: "127.0.0.1:1", web: "127.0.0.1:2", ws: "127.0.0.1:3", devtools: "127.0.0.1:4" },
};
const peer = (name: string) => ({ name }) as any;
const signIn = (extra: Record<string, unknown> = {}) => ({ connected: true, valid: true, needsReauth: false, email: "me@example.com", ...extra });

/** Every input of the panel as a signal, so a scenario can change it while the panel is mounted. */
function mount(initial: Record<string, any> = {}) {
    const [info, setInfo] = createSignal<typeof INFO | null>("info" in initial ? initial.info : INFO);
    const [peers, setPeers] = createSignal<any[]>(initial.peers ?? []);
    const [lanOn, setLanOn] = createSignal<boolean>(initial.lanOn ?? false);
    const [lanError, setLanError] = createSignal<string | null>(initial.lanError ?? null);
    const [discoverability, setDiscoverability] = createSignal<any>(initial.discoverability ?? null);
    const [firewall, setFirewall] = createSignal<any>(initial.firewall ?? null);
    const [status, setStatus] = createSignal<any>("status" in initial ? initial.status : null);
    const [loading, setLoading] = createSignal<boolean>(initial.loading ?? false);
    const [error, setError] = createSignal<string | null>(initial.error ?? null);
    const [configured, setConfigured] = createSignal<boolean>(initial.configured ?? true);
    const [showCloud, setShowCloud] = createSignal<boolean>(initial.showCloud ?? true);
    const muxbus = {
        status,
        loading,
        error,
        isConfigured: configured,
        refresh: async () => void log("muxbus refresh"),
        connect: async () => void log("muxbus connect"),
        cancel: () => void log("muxbus cancel"),
        disconnect: async () => void log("muxbus disconnect"),
    } as any;
    const anchor = document.createElement("div");
    anchor.id = "trigger";
    const view = render(() => (
        <HostPopoverPanel
            anchor={anchor}
            onClose={() => log("onClose")}
            hostname="narko"
            hostInfo={info}
            lanInstances={peers}
            lanCount={() => peers().length}
            lanDiscoveryEnabled={lanOn}
            lanDiscoveryError={lanError}
            lanDiscoverability={discoverability}
            lanFirewall={firewall}
            onLanToggle={(on) => log(`onLanToggle ${on}`)}
            muxbus={muxbus}
            {...(initial.omitShowCloud ? {} : { showMuxbusCloud: showCloud })}
        />
    ));
    host = view.container;
    return { setInfo, setPeers, setLanOn, setLanError, setDiscoverability, setFirewall, setStatus, setLoading, setError, setConfigured, setShowCloud };
}
const byText = (text: string) => [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === text)!;
const click = async (text: string) => {
    fireEvent.click(byText(text));
    await flush();
};

beforeEach(() => {
    T.length = 0;
    h.platform = "win";
    h.devMode = true;
    h.openDir = async () => {};
});
afterEach(() => cleanup());

describe("Host popover panel: the host", () => {
    it("shows the hostname given until the host's details arrive, then those, and the ports with DevTools only in dev mode", () => {
        const c = mount({ info: null });
        look("no details");
        c.setInfo(INFO);
        look("details");
        h.devMode = false;
        cleanup();
        mount();
        look("details, not dev mode");
        return snapshot("host");
    });

    it("names the file manager for each platform on the data path", () => {
        for (const p of ["win", "mac", "linux"] as const) {
            h.platform = p;
            mount();
            log(`-- ${p}: ${host.querySelector(".status-bar-popover-link")!.getAttribute("data-tip")}`);
            cleanup();
        }
        return snapshot("file-manager");
    });

    it("opens the data folder from the path, shows why it could not, and clears that on the next try", async () => {
        mount();
        await click(INFO.dataDir);
        look("opened");
        h.openDir = async () => {
            throw new Error("no handler");
        };
        await click(INFO.dataDir);
        look("failed");
        let finish!: () => void;
        h.openDir = () => new Promise<void>((r) => (finish = r));
        await click(INFO.dataDir);
        look("trying again, error cleared");
        finish();
        await flush();
        look("opened again");
        h.openDir = () => Promise.reject("plain string");
        await click(INFO.dataDir);
        look("failed with a string");
        return snapshot("data-dir");
    });

    it("hands the popover its anchor and placement, and dismissing closes the panel", () => {
        mount();
        h.dismiss();
        return snapshot("popover");
    });
});

describe("Host popover panel: LAN", () => {
    it("the switch turns discovery on and off, and says what each does", () => {
        const c = mount();
        look("off");
        fireEvent.click(host.querySelector('[aria-label="LAN discovery"]')!);
        c.setLanOn(true);
        look("on");
        fireEvent.click(host.querySelector('[aria-label="LAN discovery"]')!);
        return snapshot("lan-switch");
    });

    it("searches for peers while on with none and no error, lists them singular or plural, and says nothing while off", () => {
        const c = mount({ lanOn: true });
        look("searching");
        c.setPeers([peer("one")]);
        look("one peer");
        c.setPeers([peer("one"), peer("two")]);
        look("two peers");
        c.setLanOn(false);
        look("off with peers");
        c.setPeers([]);
        c.setLanOn(true);
        c.setLanError("Failed to update setting: boom");
        look("error, no peers");
        return snapshot("lan-peers");
    });

    it("warns about the firewall, in the words for each verdict, only while on", () => {
        const c = mount({ lanOn: true });
        for (const status of ["ok", "unknown", "off", "needs-setup", "blocked", "public-network", "managed"]) {
            c.setFirewall({ status, adapters: [], localRulesIgnored: false });
            log(`-- ${status}: ${host.querySelector('[data-testid="lan-firewall"]')?.textContent?.trim() ?? "nothing"}`);
        }
        c.setLanOn(false);
        log(`-- off: ${host.querySelector('[data-testid="lan-firewall"]')?.textContent ?? "nothing"}`);
        c.setLanOn(true);
        c.setFirewall({ status: "blocked", adapters: [], localRulesIgnored: false });
        look("blocked");
        return snapshot("lan-firewall");
    });

    it("warns that other machines can't see this one, counting the rebuilds, only while on and undiscoverable", () => {
        const c = mount({ lanOn: true });
        for (const state of ["healthy", "degraded", "off"]) {
            c.setDiscoverability({ state, missing: [], rebuilds: 1 });
            log(`-- ${state}: ${host.querySelector('[data-testid="lan-undiscoverable"]') ? "warns" : "nothing"}`);
        }
        c.setDiscoverability({ state: "undiscoverable", missing: ["203.0.113.4"], rebuilds: 3 });
        look("undiscoverable, 3 rebuilds");
        c.setDiscoverability({ state: "undiscoverable", missing: [] });
        log(`-- rebuilds absent: ${host.querySelector('[data-testid="lan-undiscoverable"]')!.textContent!.replace(/\s+/g, " ").trim()}`);
        c.setLanOn(false);
        log(`-- off: ${host.querySelector('[data-testid="lan-undiscoverable"]') ? "warns" : "nothing"}`);
        return snapshot("lan-undiscoverable");
    });

    it("stacks the error, the firewall warning, the undiscoverable warning and the peers in that order", () => {
        mount({
            lanOn: true,
            lanError: "daemon failed",
            firewall: { status: "needs-setup", adapters: [], localRulesIgnored: false },
            discoverability: { state: "undiscoverable", missing: [], rebuilds: 2 },
            peers: [peer("alpha"), peer("beta")],
        });
        look("everything");
        return snapshot("lan-stacked");
    });

    it("hands the pairing panel whether discovery is on, as it changes", () => {
        const c = mount();
        log(`pair: ${host.querySelector("[data-pair-device]")!.getAttribute("data-lan")}`);
        c.setLanOn(true);
        log(`pair: ${host.querySelector("[data-pair-device]")!.getAttribute("data-lan")}`);
        return snapshot("pairing");
    });
});

describe("Host popover panel: MuxBus Cloud", () => {
    it("is hidden when not configured or when the setting says so, shown when the setting is absent", () => {
        const c = mount({ configured: false });
        log(`not configured: ${host.querySelector(".status-bar-popover-label + .muxbus-login-chip, .muxbus-login-chip") ? "block" : "none"}`);
        c.setConfigured(true);
        look("configured");
        c.setShowCloud(false);
        log(`setting false: ${host.textContent!.includes("MuxBus Cloud") ? "block" : "none"}`);
        cleanup();
        mount({ omitShowCloud: true });
        log(`setting absent: ${host.textContent!.includes("MuxBus Cloud") ? "block" : "none"}`);
        return snapshot("muxbus-visibility");
    });

    it("signed out: Sign in, with the keychain notice on macOS and the keyring notice on Linux", async () => {
        for (const p of ["win", "mac", "linux"] as const) {
            h.platform = p;
            mount({ status: null });
            look(`${p}, no status`);
            await click("Sign in");
            cleanup();
        }
        mount({ status: signIn({ connected: false, valid: false }) });
        look("status, not connected");
        return snapshot("muxbus-signed-out");
    });

    it("while signing in the button reads Cancel and cancels; otherwise Sign in connects", async () => {
        const c = mount({ status: null });
        await click("Sign in");
        c.setLoading(true);
        look("loading");
        await click("Cancel");
        c.setLoading(false);
        await click("Sign in");
        return snapshot("muxbus-loading");
    });

    it("an expired sign-in reads Expired — re-login, a dead one Sign in again with its email, a working one Disconnect", async () => {
        const c = mount({ status: signIn({ valid: false }) });
        look("expired");
        await click("Expired — re-login");
        c.setStatus(signIn({ needsReauth: true }));
        look("dead");
        await click("Sign in again");
        c.setStatus(signIn());
        look("working");
        await click("Disconnect");
        h.platform = "mac";
        cleanup();
        mount({ status: signIn() });
        log(`mac, working: notice=${host.querySelector(".status-bar-popover-info-notice") ? "yes" : "no"}`);
        return snapshot("muxbus-signed-in");
    });

    it("shows an error under the row, whatever the state", () => {
        const c = mount({ status: signIn(), error: "cloud unreachable" });
        look("working, error");
        c.setStatus(null);
        c.setError("another failure");
        look("signed out, error");
        return snapshot("muxbus-error");
    });
});
