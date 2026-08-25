// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Covers the read-only "External Claude Code files" displays in
// GlobalBrainManager (files Claude Code itself manages, NOT part of
// AgentMux's own Global Memory composition — codex P1, PR #2794; renamed
// from "ambient" in PR follow-up, see §7 for why).
// See docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §4, §7.

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const listMemoriesMock = vi.fn().mockResolvedValue([]);
const getClaudeGlobalConfigMock = vi.fn().mockResolvedValue({
    path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
    content: null,
    exists: false,
});
const getClaudeHostConfigMock = vi.fn().mockResolvedValue({
    path: "/home/user/.claude/CLAUDE.md",
    content: null,
    exists: false,
});
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListMemoriesCommand: (...args: unknown[]) => listMemoriesMock(...args),
        GetClaudeGlobalConfigCommand: (...args: unknown[]) => getClaudeGlobalConfigMock(...args),
        GetClaudeHostConfigCommand: (...args: unknown[]) => getClaudeHostConfigMock(...args),
    },
}));

import { GlobalBrainManager } from "./global-brain-manager";

afterEach(() => {
    cleanup();
});

describe("GlobalBrainManager shared-provider-config block", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        listMemoriesMock.mockResolvedValue([]);
        getClaudeGlobalConfigMock.mockClear();
        getClaudeHostConfigMock.mockClear();
        getClaudeHostConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: null,
            exists: false,
        });
    });

    test("renders nothing before the fetch resolves", () => {
        getClaudeGlobalConfigMock.mockReturnValue(new Promise(() => {})); // never resolves within this test
        render(() => <GlobalBrainManager />);
        expect(screen.queryByText("Claude Code — shared provider config")).not.toBeInTheDocument();
    });

    test("renders the path and content when the file exists", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Global rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);

        expect(await screen.findByText("Claude Code — shared provider config")).toBeInTheDocument();
        expect(screen.getByText("/home/user/.agentmux/shared/providers/claude/CLAUDE.md")).toBeInTheDocument();
        expect(screen.getByText("# Global rules")).toBeInTheDocument();
        // Scoped to this block — the sibling host-config block legitimately
        // renders its own "No file..." empty state under the beforeEach
        // default, which is not this block's concern.
        const block = screen.getByText("Claude Code — shared provider config").closest(".global-brain-machine-config");
        expect(block?.querySelector(".global-brain-machine-config-empty")).toBeNull();
    });

    test("renders the empty-state fallback when no file exists at that path", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: null,
            exists: false,
        });
        // Sibling host-config block set to exists:true so only this block's
        // empty state renders — avoids a duplicate-text ambiguity when
        // both blocks are empty simultaneously.
        getClaudeHostConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: "# Host CLI rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);

        const block = (await screen.findByText("Claude Code — shared provider config")).closest(".global-brain-machine-config");
        expect(block?.querySelector(".global-brain-machine-config-empty")?.textContent).toBe("No file at this path yet.");
    });

    test("is read-only — no textarea or save affordance inside the block", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Global rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);
        await screen.findByText("Claude Code — shared provider config");

        const block = screen.getByText("Claude Code — shared provider config").closest(".global-brain-machine-config");
        expect(block?.querySelector("textarea")).toBeNull();
        expect(block?.querySelector("button")).toBeNull();
    });
});

// docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §6/§7 — a
// SEPARATE block, independently fetched, rendered alongside the
// shared-provider-config block above, under the same "External Claude
// Code files" heading.
describe("GlobalBrainManager host-config block", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        listMemoriesMock.mockResolvedValue([]);
        getClaudeGlobalConfigMock.mockClear();
        getClaudeHostConfigMock.mockClear();
    });

    test("renders nothing before the fetch resolves", () => {
        getClaudeHostConfigMock.mockReturnValue(new Promise(() => {})); // never resolves within this test
        render(() => <GlobalBrainManager />);
        expect(screen.queryByText("Claude Code — host CLI config")).not.toBeInTheDocument();
    });

    test("renders the path and content when the file exists", async () => {
        getClaudeHostConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: "# Host CLI rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);

        expect(await screen.findByText("Claude Code — host CLI config")).toBeInTheDocument();
        expect(screen.getByText("/home/user/.claude/CLAUDE.md")).toBeInTheDocument();
        expect(screen.getByText("# Host CLI rules")).toBeInTheDocument();
    });

    test("renders the empty-state fallback when no file exists at that path", async () => {
        getClaudeHostConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: null,
            exists: false,
        });
        // Sibling shared-provider-config block set to exists:true so only
        // this block's empty state renders — avoids a duplicate-text
        // ambiguity when both blocks are empty simultaneously.
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Shared rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);

        const block = (await screen.findByText("Claude Code — host CLI config")).closest(".global-brain-machine-config");
        expect(block?.querySelector(".global-brain-machine-config-empty")?.textContent).toBe("No file at this path yet.");
    });

    test("is read-only — no textarea or save affordance inside the block", async () => {
        getClaudeHostConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: "# Host CLI rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);
        await screen.findByText("Claude Code — host CLI config");

        const block = screen.getByText("Claude Code — host CLI config").closest(".global-brain-machine-config");
        expect(block?.querySelector("textarea")).toBeNull();
        expect(block?.querySelector("button")).toBeNull();
    });

    test("renders alongside the shared-provider-config block, both visible with distinct badges", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Shared rules\n",
            exists: true,
        });
        getClaudeHostConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: "# Host CLI rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);

        expect(await screen.findByText("Claude Code — shared provider config")).toBeInTheDocument();
        expect(await screen.findByText("Claude Code — host CLI config")).toBeInTheDocument();
    });

    test("both blocks render under the shared 'External Claude Code files' heading", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Shared rules\n",
            exists: true,
        });
        getClaudeHostConfigMock.mockResolvedValue({
            path: "/home/user/.claude/CLAUDE.md",
            content: "# Host CLI rules\n",
            exists: true,
        });
        render(() => <GlobalBrainManager />);

        const heading = await screen.findByText(/External Claude Code files/);
        const wrapper = heading.closest(".global-brain-external-files");
        expect(wrapper?.textContent).toContain("Claude Code — shared provider config");
        expect(wrapper?.textContent).toContain("Claude Code — host CLI config");
    });
});
