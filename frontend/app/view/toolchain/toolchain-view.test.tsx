// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    ResolveCliCommand: vi.fn(),
    ToolchainEnvCommand: vi.fn(),
    ToolchainVersionsCommand: vi.fn(),
    WidgetHealthCommand: vi.fn(),
    ToolchainPruneCommand: vi.fn(),
    ContainerRuntimeAvailableCommand: vi.fn(),
}));
const openExternal = vi.hoisted(() => vi.fn());
const createBlock = vi.hoisted(() => vi.fn());

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/store/global", () => ({ getApi: () => ({ openExternal }), createBlock }));
vi.mock("@/app/view/agent/install-check-cache", () => ({
    checkProviderInstalled: vi.fn().mockResolvedValue({ installed: false }),
    invalidateInstallCheck: vi.fn(),
    onInstallCheckInvalidated: vi.fn(() => () => {}),
}));
vi.mock("@/app/store/toolchain-capabilities", () => ({
    ensureCapability: vi.fn().mockResolvedValue(undefined),
    getCapability: () => ({ status: "unknown" }),
    isAvailable: () => true,
    watchCapability: () => () => {},
}));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn().mockResolvedValue(undefined) }));
vi.mock("./SystemToolInstallInline", () => ({ SystemToolInstallInline: () => <div data-testid="install-inline" /> }));
vi.mock("./CliDiskSpace", () => ({ CliDiskSpace: () => <div data-testid="cli-disk-space" /> }));

import { ToolchainView } from "./toolchain-view";

beforeEach(() => {
    localStorage.clear();
    Object.values(rpc).forEach((f) => f.mockReset());
    // git and node are found; everything else is not.
    rpc.ResolveCliCommand.mockImplementation((_c: unknown, data: { provider_id: string }) =>
        ["git", "node"].includes(data.provider_id)
            ? Promise.resolve({ version: "9.9.9", cli_path: `/bin/${data.provider_id}`, source: "system_path" })
            : Promise.reject(new Error("not found"))
    );
    rpc.ToolchainEnvCommand.mockResolvedValue({
        path: "/bin:/usr/bin",
        pathSource: "login-shell",
        os: "linux",
        arch: "x64",
    });
    rpc.ToolchainVersionsCommand.mockResolvedValue({});
    rpc.WidgetHealthCommand.mockResolvedValue({ healthy: false });
    rpc.ContainerRuntimeAvailableCommand.mockResolvedValue({ available: false });
});
afterEach(() => cleanup());

describe("ToolchainView", () => {
    it("shows each tool as a card with its status once probed", async () => {
        render(() => <ToolchainView />);
        const git = await screen.findByRole("button", { name: "Git" });
        await waitFor(() => expect(within(git).getByText("9.9.9")).toBeInTheDocument());
        expect(within(git).getByRole("img", { name: "Installed" })).toBeInTheDocument();
        // A required tool that is missing is flagged for the user; an optional one is not.
        const python = screen.getByRole("button", { name: "Python" });
        await waitFor(() => expect(within(python).getByRole("img", { name: "Not found" })).toBeInTheDocument());
        expect(python.className).toContain("is-attention");
        const ollama = screen.getByRole("button", { name: "Ollama" });
        await waitFor(() =>
            expect(within(ollama).getByRole("img", { name: "Not installed (optional)" })).toBeInTheDocument()
        );
        expect(ollama.className).not.toContain("is-attention");
    });

    it("groups tools under headings", async () => {
        render(() => <ToolchainView />);
        await screen.findByRole("button", { name: "Git" });
        for (const name of ["Core tools", "Local models", "Agent CLIs", "External widgets"]) {
            expect(screen.getByRole("heading", { name })).toBeInTheDocument();
        }
    });

    it("opens a missing tool's details with its install action, one card at a time", async () => {
        render(() => <ToolchainView />);
        const python = await screen.findByRole("button", { name: "Python" });
        await waitFor(() => expect(within(python).getByRole("img", { name: "Not found" })).toBeInTheDocument());
        fireEvent.click(python);
        expect(python).toHaveAttribute("aria-expanded", "true");
        expect(await screen.findByText("Install command")).toBeInTheDocument();
        fireEvent.click(within(python).getByRole("button", { name: "Install" }));
        expect(openExternal).toHaveBeenCalledTimes(1);
        // Another card closes the first, and the same card again closes itself.
        fireEvent.click(screen.getByRole("button", { name: "Git" }));
        expect(python).toHaveAttribute("aria-expanded", "false");
        expect(screen.getByRole("button", { name: "Git" })).toHaveAttribute("aria-expanded", "true");
        fireEvent.click(screen.getByRole("button", { name: "Git" }));
        expect(screen.getByRole("button", { name: "Git" })).toHaveAttribute("aria-expanded", "false");
    });

    it("narrows every group with the filter, and says when nothing matches", async () => {
        render(() => <ToolchainView />);
        await screen.findByRole("button", { name: "Git" });
        const filter = screen.getByPlaceholderText("Filter tools");
        fireEvent.input(filter, { target: { value: "git" } });
        expect(screen.getByRole("button", { name: "Git" })).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Python" })).toBeNull();
        expect(screen.queryByRole("heading", { name: "Local models" })).toBeNull();
        fireEvent.input(filter, { target: { value: "zzzz" } });
        expect(screen.getByText(/No tools match/)).toBeInTheDocument();
    });

    it("probes again on Refresh and shows it busy meanwhile", async () => {
        render(() => <ToolchainView />);
        await screen.findByRole("button", { name: "Git" });
        await waitFor(() => expect(rpc.WidgetHealthCommand).toHaveBeenCalled());
        const before = rpc.ResolveCliCommand.mock.calls.length;
        let finish: () => void = () => {};
        rpc.ResolveCliCommand.mockImplementation(() => new Promise<void>((resolve) => (finish = resolve)));
        fireEvent.click(screen.getByRole("button", { name: /Refresh/ }));
        await waitFor(() => expect(rpc.ResolveCliCommand.mock.calls.length).toBeGreaterThan(before));
        expect(screen.getByRole("button", { name: /Refresh/ })).toBeDisabled();
        finish();
    });

    it("changes a widget's port, remembers it, and checks the widget there", async () => {
        render(() => <ToolchainView />);
        const widgets = await screen.findByRole("group", { name: "External widgets" });
        const first = within(widgets).getAllByRole("button")[0];
        fireEvent.click(first);
        const port = await screen.findByRole("spinbutton");
        rpc.WidgetHealthCommand.mockClear();
        fireEvent.input(port, { target: { value: "9123" } });
        fireEvent.blur(port); // a pending change is applied at once on blur
        await waitFor(() =>
            expect(rpc.WidgetHealthCommand).toHaveBeenCalledWith(
                {},
                expect.objectContaining({ port: 9123 }),
                expect.anything()
            )
        );
        expect(Object.values(JSON.parse(localStorage.getItem("agentmux:widget-ports") ?? "{}"))).toEqual([9123]);
        // The reset button puts the default back.
        fireEvent.click(await screen.findByRole("button", { name: "Reset to the default port" }));
        await waitFor(() => expect(localStorage.getItem("agentmux:widget-ports")).toBe("{}"));
    });
});
