// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A successful system-tool install must not revert the row to looking
 * like nothing happened. srv's own PATH is captured at process startup,
 * so a freshly-installed binary routinely still probes as "missing"
 * right after install — the row must keep saying "installed" instead of
 * silently reverting to the same "not found" state the user just fixed.
 *
 * `installedPendingRestart` is deliberately NOT local state in
 * `AgentPrereqModalPanel` — `AgentPicker.tsx`'s refresh loop calls
 * `modalLayer.replace()`, which `ModalLayer` remounts the whole panel
 * subtree for (reagent + Codex, PR #2966). The caller owns this set and
 * threads it through every `replace`; this component only ever reads it
 * from props. The tests below exercise that contract directly: they
 * simulate a full unmount + remount with fresh props, the same thing
 * `ModalLayer.replace()` does to the real component, instead of relying
 * on any internal signal surviving.
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
                installedPendingRestart={new Set()}
                onToolInstalled={() => {}}
                onRefresh={() => {}}
                onProceed={() => {}}
                onCancel={() => {}}
            />
        ));
        await screen.findByText("Install v2.47.1 now ↓");
    });

    it("reports a successful install to the caller via onToolInstalled instead of owning the state itself", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "winget",
            args: ["install", "--id", "Git.Git"],
            needsElevation: true,
            commandPreview: "winget install --id Git.Git",
            resolvedVersion: "2.47.1",
        });
        installMock.mockResolvedValue({ sessionId: "sysinstall-1" });
        const onToolInstalled = vi.fn();
        const { AgentPrereqModalPanel } = await import("./AgentPrereqModal");
        render(() => (
            <AgentPrereqModalPanel
                agent={{ name: "Claude" } as any}
                missing={missing}
                installedPendingRestart={new Set()}
                onToolInstalled={onToolInstalled}
                onRefresh={() => {}}
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

        await waitFor(() => expect(onToolInstalled).toHaveBeenCalledWith("git"));
    });

    it("keeps showing 'installed, restart AgentMux to use it' across a full remount, purely from installedPendingRestart props — even though the stale-PATH re-probe still lists the tool as missing", async () => {
        resolveMock.mockResolvedValue({
            available: true,
            program: "winget",
            args: ["install", "--id", "Git.Git"],
            needsElevation: true,
            commandPreview: "winget install --id Git.Git",
            resolvedVersion: "2.47.1",
        });
        const { AgentPrereqModalPanel } = await import("./AgentPrereqModal");

        // Mount 1: nothing installed yet — normal "not found" row.
        const { unmount } = render(() => (
            <AgentPrereqModalPanel
                agent={{ name: "Claude" } as any}
                missing={missing}
                installedPendingRestart={new Set()}
                onToolInstalled={() => {}}
                onRefresh={() => {}}
                onProceed={() => {}}
                onCancel={() => {}}
            />
        ));
        await screen.findByText(/— not found/);

        // Simulate exactly what `ModalLayer.replace()` does to the real
        // component: destroy this instance entirely and mount a fresh one.
        // `missing` still lists git (the stale-PATH re-probe outcome this
        // PR exists to handle) but `installedPendingRestart` — owned by
        // the caller, not this component — now contains it.
        unmount();
        render(() => (
            <AgentPrereqModalPanel
                agent={{ name: "Claude" } as any}
                missing={missing}
                installedPendingRestart={new Set(["git"])}
                onToolInstalled={() => {}}
                onRefresh={() => {}}
                onProceed={() => {}}
                onCancel={() => {}}
            />
        ));

        await screen.findByText(/installed, restart AgentMux to use it/);
        expect(screen.queryByText(/— not found/)).toBeNull();
    });
});
