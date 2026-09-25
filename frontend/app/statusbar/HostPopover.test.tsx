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
    getApi: () => ({ getAuthKey: () => "k", getHostName: () => "narko" }),
    lanInstancesAtom: () => [],
    lanDiscoveryErrorAtom: () => null,
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

function renderPanel() {
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
            onLanToggle={() => {}}
            muxbus={muxbus}
        />
    ));
}

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
