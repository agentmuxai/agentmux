// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A successful system-tool install must not revert the row to looking
 * like nothing happened. srv's own PATH is captured at process startup,
 * so a freshly-installed binary routinely still probes as "missing"
 * right after install — the row must keep saying "installed" instead of
 * silently reverting to the same "not found" state the user just fixed.
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
vi.mock("@/app/store/global", () => ({ getApi: () => ({ openExternal: vi.fn() }) }));

afterEach(() => {
    cleanup();
    resolveMock.mockReset();
    installMock.mockReset();
    chunkHandler = null;
});

const missing = [
    { tool: "git", label: "Git", installUrl: "https://git-scm.com", installLinkText: "Install Git" },
];

describe("AgentPrereqModalPanel", () => {
    it("labels the toggle with the resolved version once resolution completes", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "winget",
            args: ["install", "--id", "Git.Git"],
            needsElevation: true,
            commandPreview: "winget install --id Git.Git",
            resolvedVersion: "2.47.1",
        });
        const { AgentPrereqModalPanel } = await import("./AgentPrereqModal");
        render(() => (
            <AgentPrereqModalPanel
                agent={{ name: "Claude" } as any}
                missing={missing}
                onRefresh={() => {}}
                onProceed={() => {}}
                onCancel={() => {}}
            />
        ));
        await screen.findByText("Install v2.47.1 now ↓");
    });

    it("keeps showing 'installed, restart AgentMux to use it' after a successful install, even if refresh still reports it missing", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "winget",
            args: ["install", "--id", "Git.Git"],
            needsElevation: true,
            commandPreview: "winget install --id Git.Git",
            resolvedVersion: "2.47.1",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-1" });
        const onRefresh = vi.fn();
        const { AgentPrereqModalPanel } = await import("./AgentPrereqModal");
        render(() => (
            <AgentPrereqModalPanel
                agent={{ name: "Claude" } as any}
                missing={missing}
                onRefresh={onRefresh}
                onProceed={() => {}}
                onCancel={() => {}}
            />
        ));

        const toggle = await screen.findByText("Install v2.47.1 now ↓");
        fireEvent.click(toggle);
        await screen.findByText("winget install --id Git.Git");

        fireEvent.click(screen.getByText("Install v2.47.1 now"));
        await waitFor(() => expect(chunkHandler).not.toBeNull());
        chunkHandler!({ data: { op: "done", ok: true } });

        await screen.findByText(/installed, restart AgentMux to use it/);
        expect(onRefresh).toHaveBeenCalledTimes(1);
        // The stale-PATH re-probe in the real app would still report "not
        // found" here (props.missing is unchanged in this test) — the
        // persisted success state must win over that, not revert to it.
        expect(screen.queryByText(/— not found/)).toBeNull();
    });
});
