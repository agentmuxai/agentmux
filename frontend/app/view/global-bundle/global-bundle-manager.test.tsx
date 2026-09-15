// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Covers the read-only "Claude Code provider config" display in
// GlobalBundleManager — the CLAUDE.md in the isolated config dir a spawned
// agent actually launches with, NOT part of AgentMux's own Global Memory
// composition (codex P1, PR #2794).
// See docs/specs/SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md §4, §7 and
// docs/specs/SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md (which
// removed the sibling ~/.claude host-CLI-config display).
//
// Updated for the unified `.global-bundle-file` row shape
// (SPEC_ARMORY_GLOBAL_MEMORY_DECLUTTER_2026_09_15.md): the block no longer
// carries a visible "Claude Code — shared provider config" badge or a
// section heading — its identifying text is now the path itself (still
// visible, in the row's <code> label), so these tests scope off that
// instead. The badge/description text moved to a `title` attribute (see the
// markup) rather than being removed outright — it isn't asserted on here
// since testing-library's text queries don't match `title`.

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const CLAUDE_CONFIG_PATH = "/home/user/.agentmux/shared/providers/claude/CLAUDE.md";

const listMemoriesMock = vi.fn().mockResolvedValue([]);
const getClaudeGlobalConfigMock = vi.fn().mockResolvedValue({
    path: CLAUDE_CONFIG_PATH,
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
        expect(screen.queryByText(CLAUDE_CONFIG_PATH)).not.toBeInTheDocument();
    });

    test("renders the path and content when the file exists", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: CLAUDE_CONFIG_PATH,
            content: "# Global rules\n",
            exists: true,
        });
        render(() => <GlobalBundleManager />);

        expect(await screen.findByText(CLAUDE_CONFIG_PATH)).toBeInTheDocument();
        // Rendered as markdown now, not a raw <pre> dump — "# Global rules"
        // becomes an <h1> reading "Global rules", the "#" consumed by
        // markdown parsing rather than appearing as literal text. Asserting
        // on the rendered heading proves markdown rendering actually
        // happened, not just that the raw string made it into the DOM
        // somewhere.
        expect(screen.getByText("Global rules", { selector: ".heading" })).toBeInTheDocument();
        // Scoped to this row rather than the whole pane, so a pane-wide
        // assertion doesn't silently start covering some other row added
        // later.
        const block = screen.getByText(CLAUDE_CONFIG_PATH).closest(".global-bundle-file");
        expect(block?.querySelector(".global-bundle-file-empty")).toBeNull();
    });

    test("renders the empty-state fallback when no file exists at that path", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: CLAUDE_CONFIG_PATH,
            content: null,
            exists: false,
        });
        render(() => <GlobalBundleManager />);

        const block = (await screen.findByText(CLAUDE_CONFIG_PATH)).closest(".global-bundle-file");
        expect(block?.querySelector(".global-bundle-file-empty")?.textContent).toBe("No file at this path yet.");
    });

    test("is read-only — no textarea or save affordance inside the block", async () => {
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: CLAUDE_CONFIG_PATH,
            content: "# Global rules\n",
            exists: true,
        });
        render(() => <GlobalBundleManager />);
        await screen.findByText(CLAUDE_CONFIG_PATH);

        const block = screen.getByText(CLAUDE_CONFIG_PATH).closest(".global-bundle-file");
        expect(block?.querySelector("textarea")).toBeNull();
        expect(block?.querySelector("button")).toBeNull();
    });
});

// The former "GlobalBundleManager host-config block" describe (and the
// "both blocks render under the shared heading" test) were removed with the
// block itself — SPEC_ARMORY_DROP_HOST_CLI_CONFIG_BLOCK_2026_09_01.md. Once
// REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md proved a spawned
// agent never reads ~/.claude/CLAUDE.md, surfacing it in Armory was noise.
// The "section heading no longer claims to list multiple external files"
// test that used to live here was removed alongside the heading itself
// (SPEC_ARMORY_GLOBAL_MEMORY_DECLUTTER_2026_09_15.md) — there's no longer
// any heading text to assert on; the "no host path anywhere in the pane"
// assertion below is what actually matters and still applies.
describe("GlobalBundleManager — host CLI config block is gone", () => {
    beforeEach(() => {
        listMemoriesMock.mockClear();
        listMemoriesMock.mockResolvedValue([]);
        getClaudeGlobalConfigMock.mockClear();
        getClaudeGlobalConfigMock.mockResolvedValue({
            path: CLAUDE_CONFIG_PATH,
            content: "# Shared rules\n",
            exists: true,
        });
    });

    test("renders the shared-provider path but no host CLI config path", async () => {
        render(() => <GlobalBundleManager />);

        expect(await screen.findByText(CLAUDE_CONFIG_PATH)).toBeInTheDocument();
        // The host path must not appear anywhere in the rendered pane.
        expect(document.body.textContent).not.toContain("/home/user/.claude/CLAUDE.md");
    });
});
