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

    it("shows the node.js brand icon for toolId=node, in both the consent and installing-header views", async () => {
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
        expect(container.querySelector(".system-tool-install-brand-icon.fa-brands.fa-node-js")).not.toBeNull();

        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());
        expect(container.querySelector(".system-tool-install-log-header .fa-brands.fa-node-js")).not.toBeNull();
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
        expect(container.querySelector(".system-tool-install-brand-icon")).toBeNull();
    });
});

describe("SystemToolInstallInline — Details auto-scroll (SPEC_SYSTEM_TOOL_INSTALL_DETAILS_AUTOSCROLL_2026_09_10.md §3-§4)", () => {
    /** jsdom has no real layout engine — scrollHeight/clientHeight are
     *  always 0 unless stubbed, matching ToolOverlayLog.test.tsx's own
     *  approach to the identical problem. */
    function stubScrollHeight(px: number) {
        return vi.spyOn(HTMLPreElement.prototype, "scrollHeight", "get").mockReturnValue(px);
    }
    function stubClientHeight(px: number) {
        return vi.spyOn(HTMLPreElement.prototype, "clientHeight", "get").mockReturnValue(px);
    }

    async function startInstalling() {
        resolveMock.mockResolvedValue({
            available: true,
            program: "brew",
            args: ["install", "git"],
            needsElevation: false,
            commandPreview: "brew install git",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-scroll" });
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        const { container } = render(() => <SystemToolInstallInline toolId="git" onInstalled={() => {}} />);
        await screen.findByText("brew install git");
        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());
        const pre = container.querySelector(".system-tool-install-log-body") as HTMLPreElement;
        return { pre, container };
    }

    it("scrolls to the newest line as it streams in, while at the bottom", async () => {
        stubClientHeight(50);
        const scrollStub = stubScrollHeight(100);
        const { pre } = await startInstalling();
        pre.scrollTop = 50; // at the bottom for a 100px-tall, 50px-viewport box

        chunkHandler!({ data: { line: "line 1", stream: "stdout" } });
        scrollStub.mockReturnValue(120); // content grew
        chunkHandler!({ data: { line: "line 2", stream: "stdout" } });
        await vi.waitFor(() => expect(pre.scrollTop).toBe(120));
    });

    it("stops auto-scrolling once the user scrolls away from the bottom", async () => {
        stubClientHeight(50);
        stubScrollHeight(200);
        const { pre } = await startInstalling();
        pre.scrollTop = 0; // scrolled all the way up — 150px short of the bottom
        fireEvent.scroll(pre);

        chunkHandler!({ data: { line: "line 1", stream: "stdout" } });
        await new Promise((r) => setTimeout(r, 20));
        expect(pre.scrollTop).toBe(0);
    });

    it("resumes auto-scroll once the user scrolls back within the forgiving threshold", async () => {
        stubClientHeight(50);
        const scrollStub = stubScrollHeight(200);
        const { pre } = await startInstalling();
        pre.scrollTop = 0;
        fireEvent.scroll(pre); // unstick

        pre.scrollTop = 155; // within 40px of the bottom (200 - 50 - 155 = -5, clamps to "at bottom")
        fireEvent.scroll(pre);

        scrollStub.mockReturnValue(240);
        chunkHandler!({ data: { line: "line 2", stream: "stdout" } });
        await vi.waitFor(() => expect(pre.scrollTop).toBe(240));
    });

    it("re-syncs to the bottom when the <details> panel is (re)opened while stuck to bottom", async () => {
        stubClientHeight(50);
        stubScrollHeight(300);
        const { pre, container } = await startInstalling();
        const details = container.querySelector(".system-tool-install-details") as HTMLDetailsElement;
        pre.scrollTop = 0; // simulate whatever stale position was left while collapsed

        // A real click on <summary> flips `.open` BEFORE the browser
        // dispatches "toggle" — a bare synthetic event doesn't do that for
        // us, so set it explicitly to match real behavior.
        details.open = true;
        fireEvent(details, new Event("toggle"));
        await vi.waitFor(() => expect(pre.scrollTop).toBe(300));
    });

    it("resets stickToBottom on Retry, regardless of prior scroll-away state", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "brew",
            args: ["install", "git"],
            needsElevation: false,
            commandPreview: "brew install git",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-retry" });
        stubClientHeight(50);
        const scrollStub = stubScrollHeight(200);
        const { SystemToolInstallInline } = await import("./SystemToolInstallInline");
        const { container } = render(() => <SystemToolInstallInline toolId="git" onInstalled={() => {}} />);
        await screen.findByText("brew install git");
        fireEvent.click(screen.getByText("Install"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());

        const pre = container.querySelector(".system-tool-install-log-body") as HTMLPreElement;
        pre.scrollTop = 0;
        fireEvent.scroll(pre); // unstick

        chunkHandler!({ data: { op: "done", ok: false, error: "failed" } });
        await screen.findByText("Retry");

        chunkHandler = null;
        fireEvent.click(screen.getByText("Retry"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());

        scrollStub.mockReturnValue(400);
        chunkHandler!({ data: { line: "retry line", stream: "stdout" } });
        await vi.waitFor(() => expect(pre.scrollTop).toBe(400));
    });
});
