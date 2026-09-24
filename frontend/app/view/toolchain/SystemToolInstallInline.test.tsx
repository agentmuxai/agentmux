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
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (sub: { handler: (event: unknown) => void }) => {
        chunkHandler = sub.handler;
        return () => { chunkHandler = null; };
    },
}));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn().mockResolvedValue(undefined) }));

import { resetInstallDetailsPrefs } from "@/element/install/InstallProgress";

afterEach(() => {
    cleanup();
    resolveMock.mockReset();
    installMock.mockReset();
    chunkHandler = null;
    resetInstallDetailsPrefs();
});

const logRows = (container: HTMLElement) =>
    Array.from(container.querySelectorAll<HTMLElement>(".install-log-line")).map((el) => el.textContent);
const stepStatus = (container: HTMLElement) =>
    Array.from(container.querySelectorAll<HTMLElement>(".install-step")).map(
        (li) => `${li.querySelector(".install-step-label")?.textContent}:${li.dataset.status}`,
    );

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

    it("labels the install button with the resolved version when the backend reports one", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "brew",
            args: ["install", "git"],
            needsElevation: false,
            commandPreview: "brew install git",
            resolvedVersion: "2.47.1",
        });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        render(() => <SystemToolInstallInline toolId="git" onInstalled={() => {}} />);
        await screen.findByText("Install v2.47.1 now");
    });

    it("uses caller-provided resolvedInfo instead of self-resolving", async () => {
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        render(() => (
            <SystemToolInstallInline
                toolId="git"
                onInstalled={() => {}}
                resolvedInfo={{ commandPreview: "brew install git", needsElevation: false, resolvedVersion: "2.47.1" }}
            />
        ));
        await screen.findByText("Install v2.47.1 now");
        expect(resolveMock).not.toHaveBeenCalled();
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

    it("streams install_chunk lines into Details and calls onInstalled on a successful done event", async () => {
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
        const { container } = render(() => <SystemToolInstallInline toolId="git" onInstalled={onInstalled} />);
        await screen.findByText("brew install git");

        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(installMock).toHaveBeenCalledWith({}, { toolId: "git" }));
        await waitFor(() => expect(chunkHandler).not.toBeNull());

        chunkHandler!({ data: { line: "$ brew install git", stream: "stdout" } });
        chunkHandler!({ data: { line: "==> Installing git", stream: "stdout" } });
        await waitFor(() => expect(logRows(container)).toContain("==> Installing git"));
        // Layer 1 shows the plain step, with the latest output as its subline.
        expect(stepStatus(container)).toEqual(["Get ready:done", "Install Git:active", "Check Git is available:pending"]);
        expect(container.querySelector(".install-step-subline")?.textContent).toBe("Installing git");
        // Details stays collapsed by default.
        expect((container.querySelector(".install-details") as HTMLDetailsElement).open).toBe(false);

        chunkHandler!({ data: { op: "done", ok: true } });
        await screen.findByText("Installed");
        expect(stepStatus(container)).toEqual(["Get ready:done", "Install Git:done", "Check Git is available:done"]);
        expect(onInstalled).toHaveBeenCalledTimes(1);
    });

    it("explains a failed done event on the step, opens Details with the reason, and offers Retry", async () => {
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
        const { container } = render(() => <SystemToolInstallInline toolId="git" onInstalled={onInstalled} />);
        await screen.findByText("brew install git");

        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());

        chunkHandler!({ data: { line: "$ brew install git", stream: "stdout" } });
        chunkHandler!({ data: { line: "Error: git: no bottle available!", stream: "stderr" } });
        chunkHandler!({ data: { op: "done", ok: false, error: "brew exited Some(1)" } });
        await screen.findByText("Failed");
        await screen.findByText('Something went wrong while running "Install Git".');
        await waitFor(() => expect((container.querySelector(".install-details") as HTMLDetailsElement).open).toBe(true));
        expect(logRows(container)).toContain("Install failed: brew exited Some(1)");
        await screen.findByText("Retry");
        expect(onInstalled).not.toHaveBeenCalled();
    });

    it("adds the permission step when the command needs elevation, and maps a dismissed prompt to it", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "pkexec",
            args: ["apt-get", "install", "-y", "git"],
            needsElevation: true,
            commandPreview: "pkexec apt-get install -y git",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-pk" });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        const { container } = render(() => <SystemToolInstallInline toolId="git" onInstalled={() => {}} />);
        await screen.findByText("pkexec apt-get install -y git");
        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());

        chunkHandler!({ data: { line: "$ pkexec apt-get install -y git", stream: "stdout" } });
        await waitFor(() => expect(stepStatus(container)[1]).toBe("Ask for permission:active"));
        chunkHandler!({ data: { op: "done", ok: false, error: "pkexec exited Some(126)" } });
        await screen.findByText("AgentMux wasn't allowed to write the files.");
        expect(stepStatus(container)[1]).toBe("Ask for permission:failed");
    });

    it("shows a restart step instead of a free-text note when AgentMux can't see the new tool yet", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "winget",
            args: ["install", "--id", "Git.Git", "-e"],
            needsElevation: false,
            commandPreview: "winget install --id Git.Git -e",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-path" });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        const { container } = render(() => <SystemToolInstallInline toolId="git" onInstalled={() => {}} />);
        await screen.findByText("winget install --id Git.Git -e");
        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());

        chunkHandler!({ data: { line: "$ winget install --id Git.Git -e", stream: "stdout" } });
        chunkHandler!({
            data: {
                line: 'Note: winget finished successfully, but AgentMux\'s current session doesn\'t see "git" yet — restart AgentMux to pick up the updated PATH.',
                stream: "stdout",
            },
        });
        chunkHandler!({ data: { op: "done", ok: true } });
        await screen.findByText("Restart AgentMux to finish");
        expect(stepStatus(container)).toEqual([
            "Get ready:done",
            "Install Git:done",
            "Check Git is available:failed",
            "Restart AgentMux to finish:pending",
        ]);
    });

    it("shows the node.js brand icon for toolId=node, in both the consent and installing views", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "winget",
            args: ["install", "--id", "OpenJS.NodeJS.LTS", "-e"],
            needsElevation: false,
            commandPreview: "winget install --id OpenJS.NodeJS.LTS -e",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-node" });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        const { container } = render(() => <SystemToolInstallInline toolId="node" onInstalled={() => {}} />);
        await screen.findByText("winget install --id OpenJS.NodeJS.LTS -e");
        expect(container.querySelector(".install-confirm-brand-icon.fa-brands.fa-node-js")).not.toBeNull();

        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());
        expect(container.querySelector(".system-tool-install-status .fa-brands.fa-node-js")).not.toBeNull();
    });

    it("shows no brand icon for a tool with no Font Awesome brand glyph (uv)", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "curl",
            args: [],
            needsElevation: false,
            commandPreview: "curl -LsSf https://astral.sh/uv/install.sh | sh",
        });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        const { container } = render(() => <SystemToolInstallInline toolId="uv" onInstalled={() => {}} />);
        await screen.findByText("curl -LsSf https://astral.sh/uv/install.sh | sh");
        expect(container.querySelector(".install-confirm-brand-icon")).toBeNull();
    });

    it("starts Retry from a clean log and step list", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "brew",
            args: ["install", "git"],
            needsElevation: false,
            commandPreview: "brew install git",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-retry" });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        const { container } = render(() => <SystemToolInstallInline toolId="git" onInstalled={() => {}} />);
        await screen.findByText("brew install git");
        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());
        chunkHandler!({ data: { line: "first run", stream: "stdout" } });
        chunkHandler!({ data: { op: "done", ok: false, error: "failed" } });
        await screen.findByText("Retry");

        chunkHandler = null;
        fireEvent.click(screen.getByText("Retry"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());
        expect(logRows(container)).toEqual([]);
        expect(stepStatus(container)[0]).toBe("Get ready:active");
    });
});
