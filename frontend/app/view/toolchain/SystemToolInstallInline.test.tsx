// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SystemToolInstallInline — the state machine for one-click system-tool
 * installs (SPEC_SYSTEM_TOOLCHAIN_INSTALLER_2026_08_24.md §3.3-§3.4):
 * renders nothing when unavailable (caller keeps its own link+copy-
 * command fallback), shows the exact resolved command before running
 * anything, and streams install_chunk output through to done/failed.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const resolveMock = vi.fn();
const installMock = vi.fn();
let chunkHandler: ((event: unknown) => void) | null = null;

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ToolchainResolveInstallCommandCommand: (...args: unknown[]) => resolveMock(...args),
        ToolchainInstallSystemToolCommand: (...args: unknown[]) => installMock(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: (sub: { handler: (event: unknown) => void }) => {
        chunkHandler = sub.handler;
        return () => { chunkHandler = null; };
    },
}));

afterEach(() => {
    cleanup();
    resolveMock.mockReset();
    installMock.mockReset();
    chunkHandler = null;
});

describe("SystemToolInstallInline", () => {
    it("renders nothing when the backend reports no installable command", async () => {
        resolveMock.mockResolvedValue({ available: false });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        const { container } = render(() => (
            <SystemToolInstallInline toolId="git" onInstalled={() => {}} />
        ));
        await waitFor(() => expect(resolveMock).toHaveBeenCalledWith({}, { toolId: "git" }));
        expect(container.textContent).toBe("");
    });

    it("calls onUnavailable when resolution reports no installable command, so callers can hide their toggle", async () => {
        resolveMock.mockResolvedValue({ available: false });
        const onUnavailable = vi.fn();
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        render(() => (
            <SystemToolInstallInline toolId="git" onInstalled={() => {}} onUnavailable={onUnavailable} />
        ));
        await waitFor(() => expect(onUnavailable).toHaveBeenCalledTimes(1));
    });

    it("calls onUnavailable when the resolve RPC itself throws", async () => {
        resolveMock.mockRejectedValue(new Error("boom"));
        const onUnavailable = vi.fn();
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        render(() => (
            <SystemToolInstallInline toolId="git" onInstalled={() => {}} onUnavailable={onUnavailable} />
        ));
        await waitFor(() => expect(onUnavailable).toHaveBeenCalledTimes(1));
    });

    it("renders the resolved command as a consent step before installing anything", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "brew",
            args: ["install", "git"],
            needsElevation: false,
            commandPreview: "brew install git",
        });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        render(() => <SystemToolInstallInline toolId="git" onInstalled={() => {}} />);
        await screen.findByText("brew install git");
        // No install call fired just from resolving/rendering the preview.
        expect(installMock).not.toHaveBeenCalled();
    });

    it("shows the elevation note only when the resolved step needs it", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "pkexec",
            args: ["apt-get", "install", "-y", "git"],
            needsElevation: true,
            commandPreview: "pkexec apt-get install -y git",
        });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        render(() => <SystemToolInstallInline toolId="git" onInstalled={() => {}} />);
        await screen.findByText(/system permission prompt/);
    });

    it("streams install_chunk lines and calls onInstalled on a successful done event", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "brew",
            args: ["install", "git"],
            needsElevation: false,
            commandPreview: "brew install git",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-1" });
        const onInstalled = vi.fn();
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        render(() => <SystemToolInstallInline toolId="git" onInstalled={onInstalled} />);
        await screen.findByText("brew install git");

        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(installMock).toHaveBeenCalledWith({}, { toolId: "git" }));
        await waitFor(() => expect(chunkHandler).not.toBeNull());

        chunkHandler!({ data: { line: "==> Installing git", stream: "stdout" } });
        await screen.findByText("==> Installing git");

        chunkHandler!({ data: { op: "done", ok: true } });
        await screen.findByText("Installed");
        expect(onInstalled).toHaveBeenCalledTimes(1);
    });

    it("shows the error + a Retry button on a failed done event, without calling onInstalled", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "brew",
            args: ["install", "git"],
            needsElevation: false,
            commandPreview: "brew install git",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-2" });
        const onInstalled = vi.fn();
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        render(() => <SystemToolInstallInline toolId="git" onInstalled={onInstalled} />);
        await screen.findByText("brew install git");

        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());

        chunkHandler!({ data: { op: "done", ok: false, error: "brew: command failed" } });
        await screen.findByText("Failed");
        await screen.findByText("brew: command failed");
        await screen.findByText("Retry");
        expect(onInstalled).not.toHaveBeenCalled();
    });
});
