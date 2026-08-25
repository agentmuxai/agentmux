// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Covers the machine-wide ~/.claude/CLAUDE.md read-only display in
// GlobalBrainManager. See
// docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §4.

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const listMemoriesMock = vi.fn().mockResolvedValue([]);
const getClaudeGlobalConfigMock = vi.fn().mockResolvedValue({ path: "/home/user/.claude/CLAUDE.md", content: null, exists: false });
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListMemoriesCommand: (...args: unknown[]) => listMemoriesMock(...args),
        GetClaudeGlobalConfigCommand: (...args: unknown[]) => getClaudeGlobalConfigMock(...args),
    },
}));

import { GlobalBrainManager } from "./global-brain-manager";

afterEach(() => {
    cleanup();
});

describe("GlobalBrainManager machine-wide config block", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        listMemoriesMock.mockResolvedValue([]);
        getClaudeGlobalConfigMock.mockClear();
    });

    test("renders nothing before the fetch resolves", () => {
        getClaudeGlobalConfigMock.mockReturnValue(new Promise(() => {})); // never resolves within this test
        render(() => <GlobalBrainManager />);
        expect(screen.queryByText("Machine-wide (Claude Code)")).not.toBeInTheDocument();
    });

    test("renders the path and content when the file exists", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: "# Global rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);

        expect(await screen.findByText("Machine-wide (Claude Code)")).toBeInTheDocument();
        expect(screen.getByText("/home/user/.claude/CLAUDE.md")).toBeInTheDocument();
        expect(screen.getByText("# Global rules")).toBeInTheDocument();
        expect(screen.queryByText("No file at this path yet.")).not.toBeInTheDocument();
    });

    test("renders the empty-state fallback when no file exists at that path", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: null,
            exists: false,
        });
        render(() => <GlobalBrainManager />);

        expect(await screen.findByText("Machine-wide (Claude Code)")).toBeInTheDocument();
        expect(screen.getByText("No file at this path yet.")).toBeInTheDocument();
    });

    test("is read-only — no textarea or save affordance inside the block", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: "# Global rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);
        await screen.findByText("Machine-wide (Claude Code)");

        const block = screen.getByText("Machine-wide (Claude Code)").closest(".global-brain-machine-config");
        expect(block?.querySelector("textarea")).toBeNull();
        expect(block?.querySelector("button")).toBeNull();
    });
});
