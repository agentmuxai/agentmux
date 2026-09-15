// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Covers the read-only "Claude Code provider config" display in
// GlobalBundleManager — the CLAUDE.md in the isolated config dir a spawned
// agent actually launches with, NOT part of AgentMux's own Global Memory
// composition (codex P1, PR #2794).
// See docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §4, §7 and
// docs/specs/SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md (which
// removed the sibling ~/.claude host-CLI-config display).

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const listMemoriesMock = vi.fn().mockResolvedValue([]);
const getClaudeGlobalConfigMock = vi.fn().mockResolvedValue({
    path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
    content: null,
    exists: false,
});
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListBundlesCommand: (...args: unknown[]) => listMemoriesMock(...args),
        GetClaudeGlobalConfigCommand: (...args: unknown[]) => getClaudeGlobalConfigMock(...args),
    },
}));

import { GlobalBundleManager } from "./global-bundle-manager";

afterEach(() => {
    cleanup();
});

describe("GlobalBundleManager shared-provider-config block", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        listMemoriesMock.mockResolvedValue([]);
        getClaudeGlobalConfigMock.mockClear();
    });

    test("renders nothing before the fetch resolves", () => {
        getClaudeGlobalConfigMock.mockReturnValue(new Promise(() => {})); // never resolves within this test
        render(() => <GlobalBundleManager />);
        expect(screen.queryByText("Claude Code — shared provider config")).not.toBeInTheDocument();
    });

    test("renders the path and content when the file exists", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Global rules\n",
            exists: true,
        });
        render(() => <GlobalBundleManager />);

        expect(await screen.findByText("Claude Code — shared provider config")).toBeInTheDocument();
        expect(screen.getByText("/home/user/.agentmux/shared/providers/claude/CLAUDE.md")).toBeInTheDocument();
        // Rendered as markdown now, not a raw <pre> dump (this test's own
        // history predates that change) — "# Global rules" becomes an <h1>
        // reading "Global rules", the "#" consumed by markdown parsing
        // rather than appearing as literal text. Asserting on the rendered
        // heading proves markdown rendering actually happened, not just
        // that the raw string made it into the DOM somewhere.
        expect(screen.getByText("Global rules", { selector: ".heading" })).toBeInTheDocument();
        // Scoped to this block rather than the whole pane. Kept scoped even
        // though only one block renders now (the sibling host-config block it
        // originally disambiguated from is gone) — a pane-wide assertion
        // would silently start covering any block added later.
        const block = screen.getByText("Claude Code — shared provider config").closest(".global-bundle-machine-config");
        expect(block?.querySelector(".global-bundle-machine-config-empty")).toBeNull();
    });

    test("renders the empty-state fallback when no file exists at that path", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: null,
            exists: false,
        });
        render(() => <GlobalBundleManager />);

        const block = (await screen.findByText("Claude Code — shared provider config")).closest(".global-bundle-machine-config");
        expect(block?.querySelector(".global-bundle-machine-config-empty")?.textContent).toBe("No file at this path yet.");
    });

    test("is read-only — no textarea or save affordance inside the block", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Global rules\n",
            exists: true,
        });
        render(() => <GlobalBundleManager />);
        await screen.findByText("Claude Code — shared provider config");

        const block = screen.getByText("Claude Code — shared provider config").closest(".global-bundle-machine-config");
        expect(block?.querySelector("textarea")).toBeNull();
        expect(block?.querySelector("button")).toBeNull();
    });
});

// The former "GlobalBundleManager host-config block" describe (and the
// "both blocks render under the shared heading" test) were removed with the
// block itself — SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md. Once
// REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md proved a spawned
// agent never reads ~/.claude/CLAUDE.md, surfacing it in Armory was noise.
describe("GlobalBundleManager — host CLI config block is gone", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        listMemoriesMock.mockResolvedValue([]);
        getClaudeGlobalConfigMock.mockClear();
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: "/home/user/.agentmux/shared/providers/claude/CLAUDE.md",
            content: "# Shared rules\n",
            exists: true,
        });
    });

    test("renders the shared-provider block but no host CLI config block", async () => {
        render(() => <GlobalBundleManager />);

        expect(await screen.findByText("Claude Code — shared provider config")).toBeInTheDocument();
        expect(screen.queryByText("Claude Code — host CLI config")).toBeNull();
        // The host path must not appear anywhere in the rendered pane.
        expect(document.body.textContent).not.toContain("/home/user/.claude/CLAUDE.md");
    });

    test("the section heading no longer claims to list multiple external files", async () => {
        render(() => <GlobalBundleManager />);

        expect(await screen.findByText(/Claude Code provider config — reference only/)).toBeInTheDocument();
        expect(screen.queryByText(/External Claude Code files/)).toBeNull();
    });
});
