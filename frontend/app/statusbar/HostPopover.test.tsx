// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * HostPopoverPanel — Instance row removed, Data path is the file-manager link.
 * SPEC_STATUSBAR_HOST_POPOVER_INSTANCE_AND_OPEN_DATA_DIR_2026_09_25.md.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
}));
vi.mock("@/app/platform/pane-overlay", () => ({
    usePaneOverlay: vi.fn(),
}));
vi.mock("@/app/util/menu-position", () => ({
    computeMenuPosition: vi.fn(async () => ({
        style: { position: "fixed", left: "0px", top: "0px" },
    })),
}));
vi.mock("@/store/global", () => ({
    getApi: () => ({
        getAuthKey: () => "k",
        getHostName: () => "narko",
        // The real CEF mapping, over the IPC mock below.
        openDataDirInFileManager: () => invokeCommandMock("open_in_file_manager", { target: "data" }),
        getHostInfo: () => invokeCommandMock("get_host_info", {}),
    }),
    lanInstancesAtom: () => [],
    lanDiscoveryErrorAtom: () => null,
    lanDiscoverabilityAtom: () => null,
    lanFirewallAtom: () => null,
    setLanDiscoveryErrorAtom: vi.fn(),
    settingsAtom: () => ({}),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/accounts/AgentMuxConnectPanel", () => ({
    useMuxBusStatus: vi.fn(),
}));
vi.mock("qrcode", () => ({ default: { toCanvas: vi.fn() } }));

const invokeCommandMock = vi.fn(async (_cmd: string, _args?: Record<string, unknown>): Promise<unknown> => null);
vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: (cmd: string, args?: Record<string, unknown>) => invokeCommandMock(cmd, args),
}));

let platform: "win" | "mac" | "linux" = "win";
vi.mock("@/util/platformutil", () => ({
    isMacOS: () => platform === "mac",
    isLinux: () => platform === "linux",
}));

import { HostPopoverPanel } from "./HostPopover";

const DATA_DIR = "C:\\Users\\me\\.agentmux\\channels\\stable\\data";

const hostInfo = {
    hostname: "narko",
    os: "Windows x86_64",
    localIp: "192.168.1.20",
    version: "0.57.5",
    dataDir: DATA_DIR,
    pid: 4242,
    ports: { ipc: "127.0.0.1:1", web: "127.0.0.1:2", ws: "127.0.0.1:3", devtools: "127.0.0.1:4" },
};

const muxbus = {
    status: () => null,
    loading: () => false,
    error: () => null,
    refresh: async () => {},
    connect: async () => {},
    cancel: () => {},
    isConfigured: () => false,
} as any;

function renderPanel(mux = muxbus, extra: Record<string, unknown> = {}) {
    return render(() => (
        <HostPopoverPanel
            anchorRect={null}
            onClose={() => {}}
            hostname="narko"
            hostInfo={() => hostInfo}
            lanInstances={() => []}
            lanCount={() => 0}
            lanDiscoveryEnabled={() => false}
            lanDiscoveryError={() => null}
            lanDiscoverability={() => null}
            lanFirewall={() => null}
            onLanToggle={() => {}}
            muxbus={mux}
            {...(extra as any)}
        />
    ));
}

describe("Data-path link — balanced line breaks (stylesheet contract)", () => {
    // jsdom has no line layout; the behaviour was measured live via CDP
    // (spec §4.1). Guard the two declarations it depends on.
    it("breaks anywhere and balances lines", async () => {
        const { readFileSync } = await import("node:fs");
        const { join } = await import("node:path");
        const scss = readFileSync(join(__dirname, "StatusBar.scss"), "utf8");
        const start = scss.search(/^\s*\.status-bar-popover-link\s*\{/m);
        const body = scss.slice(start, scss.indexOf("}", start));
        expect(body).toMatch(/word-break:\s*break-all;/);
        expect(body).toMatch(/text-wrap:\s*balance;/);
        expect(body).not.toMatch(/text-overflow|max-width/);
    });
});

describe("HostPopoverPanel — Instance row and Data-path link", () => {
    beforeEach(() => {
        platform = "win";
        invokeCommandMock.mockReset();
        invokeCommandMock.mockResolvedValue(null);
    });

    afterEach(() => {
        cleanup();
    });

    it("no longer renders an Instance row", () => {
        renderPanel();
        expect(screen.queryByText("Instance")).not.toBeInTheDocument();
        // Neighbouring rows are still there.
        expect(screen.getByText("PID")).toBeInTheDocument();
        expect(screen.getByText("Data")).toBeInTheDocument();
    });

    it("renders the Data path as a button whose only content is the path (no icon)", () => {
        renderPanel();
        const link = screen.getByRole("button", { name: DATA_DIR });
        expect(link.tagName).toBe("BUTTON");
        expect(link).toHaveClass("status-bar-popover-link");
        expect(link.textContent).toBe(DATA_DIR);
        expect(link.children).toHaveLength(0);
        // Full text, never truncated.
        expect(link.style.textOverflow).toBe("");
        expect(link.style.maxWidth).toBe("");
    });

    it("clicking the path opens the data dir via a target, never the path string", async () => {
        renderPanel();
        fireEvent.click(screen.getByRole("button", { name: DATA_DIR }));
        await waitFor(() => expect(invokeCommandMock).toHaveBeenCalledTimes(1));
        expect(invokeCommandMock).toHaveBeenCalledWith("open_in_file_manager", { target: "data" });
    });

    it("shows the error inline when opening fails", async () => {
        invokeCommandMock.mockRejectedValueOnce("No file manager handler found (tried xdg-open and gio)");
        renderPanel();
        fireEvent.click(screen.getByRole("button", { name: DATA_DIR }));
        expect(await screen.findByText(/Couldn't open folder: No file manager handler found/)).toBeInTheDocument();
    });

    it.each([
        ["win", "Show in File Explorer"],
        ["mac", "Reveal in Finder"],
        ["linux", "Open in file manager"],
    ] as const)("tooltip on %s names the action (the path itself is shown in full)", (os, label) => {
        platform = os;
        renderPanel();
        expect(screen.getByRole("button", { name: DATA_DIR })).toHaveAttribute("data-tip", label);
    });
});

describe("HostPopoverPanel — MuxBus Cloud row", () => {
    afterEach(() => {
        cleanup();
    });

    const cloud = (status: Record<string, unknown>, connect = vi.fn(async () => {})) =>
        ({
            ...muxbus,
            isConfigured: () => true,
            status: () => ({ cognitoDomain: "", expiresAt: 0, ...status }),
            connect,
        }) as any;

    // SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md §3.5: a sign-in srv reports
    // as dead reads "Sign in again" with its email, even while its token
    // lasts, and runs the same connect action as a first sign-in.
    it("a dead sign-in offers Sign in again with the stored email", async () => {
        const connect = vi.fn(async () => {});
        renderPanel(cloud({ connected: true, valid: true, needsReauth: true, email: "me@example.com" }, connect));
        expect(screen.getByText("me@example.com")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Disconnect" })).not.toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Sign in again" }));
        await waitFor(() => expect(connect).toHaveBeenCalledTimes(1));
    });

    it("an expired but live sign-in still reads Expired — re-login", () => {
        renderPanel(cloud({ connected: true, valid: false, needsReauth: false, email: "me@example.com" }));
        expect(screen.getByRole("button", { name: "Expired — re-login" })).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Sign in again" })).not.toBeInTheDocument();
    });

    it("a working sign-in shows the email and Disconnect", () => {
        renderPanel(cloud({ connected: true, valid: true, needsReauth: false, email: "me@example.com" }));
        expect(screen.getByText("me@example.com")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Disconnect" })).toBeInTheDocument();
    });
});

describe("HostPopoverPanel — undiscoverable warning", () => {
    afterEach(() => {
        cleanup();
    });

    // Area54, 2026-10-01: it listed three peers while none of them listed it.
    it("warns that other machines cannot see this one, even with peers listed", () => {
        renderPanel(muxbus, {
            lanDiscoveryEnabled: () => true,
            lanCount: () => 3,
            lanDiscoverability: () => ({ state: "undiscoverable", missing: ["192.168.1.26"], rebuilds: 2 }),
        });
        const row = screen.getByTestId("lan-undiscoverable");
        expect(row).toHaveTextContent("Other machines can't see this one");
        expect(row).toHaveTextContent("UDP 5353");
        expect(row).toHaveTextContent("keeps retrying");
        expect(row).toHaveTextContent("2 so far");
    });

    it("shows nothing when discoverable, when LAN is off, or when there is no verdict", () => {
        renderPanel(muxbus, {
            lanDiscoveryEnabled: () => true,
            lanDiscoverability: () => ({ state: "healthy", missing: [], rebuilds: 0 }),
        });
        expect(screen.queryByTestId("lan-undiscoverable")).not.toBeInTheDocument();
        cleanup();

        renderPanel(muxbus, {
            lanDiscoveryEnabled: () => false,
            lanDiscoverability: () => ({ state: "undiscoverable", missing: ["192.168.1.26"], rebuilds: 1 }),
        });
        expect(screen.queryByTestId("lan-undiscoverable")).not.toBeInTheDocument();
        cleanup();

        renderPanel(muxbus, { lanDiscoveryEnabled: () => true, lanDiscoverability: () => null });
        expect(screen.queryByTestId("lan-undiscoverable")).not.toBeInTheDocument();
    });
});

describe("HostPopoverPanel — firewall warning", () => {
    afterEach(() => {
        cleanup();
    });

    it("explains a missing firewall rule while LAN is on", () => {
        renderPanel(muxbus, {
            lanDiscoveryEnabled: () => true,
            lanFirewall: () => ({ status: "needs-setup", adapters: [], localRulesIgnored: false }),
        });
        const row = screen.getByTestId("lan-firewall");
        expect(row).toHaveTextContent("one-time setup");
        expect(row).toHaveTextContent("Windows Firewall");
    });

    // Codex P1 on #4151: peers prove we received discovery traffic, not that anyone
    // can connect to us, so the warning shows with peers listed too, matching the
    // status bar, which keeps both facts.
    it("shows the firewall warning even when peers are listed", () => {
        for (const status of ["needs-setup", "blocked"] as const) {
            renderPanel(muxbus, {
                lanDiscoveryEnabled: () => true,
                lanCount: () => 2,
                lanFirewall: () => ({ status, adapters: [], localRulesIgnored: false }),
            });
            expect(screen.getByTestId("lan-firewall")).toBeInTheDocument();
            cleanup();
        }
    });

    it("names a Public network and a managed firewall", () => {
        renderPanel(muxbus, {
            lanDiscoveryEnabled: () => true,
            lanFirewall: () => ({ status: "public-network", adapters: [], localRulesIgnored: false }),
        });
        expect(screen.getByTestId("lan-firewall")).toHaveTextContent("Public");
        cleanup();
        renderPanel(muxbus, {
            lanDiscoveryEnabled: () => true,
            lanFirewall: () => ({ status: "managed", adapters: [], localRulesIgnored: true }),
        });
        expect(screen.getByTestId("lan-firewall")).toHaveTextContent("administrator");
    });

    it("shows nothing when the firewall is fine, unknown, off, absent, or LAN is off", () => {
        for (const status of ["ok", "unknown", "off"] as const) {
            renderPanel(muxbus, {
                lanDiscoveryEnabled: () => true,
                lanFirewall: () => ({ status, adapters: [], localRulesIgnored: false }),
            });
            expect(screen.queryByTestId("lan-firewall")).not.toBeInTheDocument();
            cleanup();
        }
        renderPanel(muxbus, { lanDiscoveryEnabled: () => true, lanFirewall: () => null });
        expect(screen.queryByTestId("lan-firewall")).not.toBeInTheDocument();
        cleanup();
        renderPanel(muxbus, {
            lanDiscoveryEnabled: () => false,
            lanFirewall: () => ({ status: "blocked", adapters: [], localRulesIgnored: false }),
        });
        expect(screen.queryByTestId("lan-firewall")).not.toBeInTheDocument();
    });
});
