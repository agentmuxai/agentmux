// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins the reagentx P2 finding on PR #2382 round 2: the rename-conflict
 * hint must check the FULL global skill catalog (fetched via
 * skill.catalog.list per SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md §4.1
 * point 2), not just the subset of skills the preview response already
 * flagged "name_conflict" against itself -- a rename to some OTHER,
 * unrelated existing global skill name previously showed no client-side
 * warning at all.
 */

import { cleanup, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";

import { BundleImportPreviewModalPanel } from "./BundleImportPreviewModal";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SkillCatalogListCommand: vi.fn(),
    },
}));

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import type { BundleImportPreviewResponse } from "@/app/store/rpc-api";

afterEach(() => {
    cleanup();
});

function makePreview(): BundleImportPreviewResponse {
    return {
        // The server has always sent this list; it was simply missing from the
        // hand-written response type, so this fixture could omit it. Generating
        // the type from Rust surfaced that.
        project_instructions: [],
        name: "test-bundle",
        description: "",
        instructions_preview: "",
        instructions_truncated: false,
        instructions_total_chars: 0,
        context_files: [],
        skills: [
            {
                source_dir: "skills/deploy",
                slug: "deploy",
                description: "d",
                collision: "duplicate_in_bundle",
            },
        ],
        mcp_servers: [],
        requirements: [],
        warnings: [],
        warnings_truncated: false,
        name_collision: false,
        content_digest: "abc123",
    };
}

describe("BundleImportPreviewModalPanel — rename conflict check", () => {
    it("warns when a rename matches a global skill name outside this bundle's own flagged rows", async () => {
        vi.mocked(RpcApi.SkillCatalogListCommand).mockResolvedValue([
            { id: "x", name: "taken-elsewhere", trigger: "taken-elsewhere", skill_type: "agent_skill", description: "", content: "", is_global: true, created_at: 0, updated_at: 0, bound_count: 0 },
        ]);

        render(() => (
            <BundleImportPreviewModalPanel preview={makePreview()} onNext={() => {}} onCancel={() => {}} />
        ));

        await waitFor(() => expect(RpcApi.SkillCatalogListCommand).toHaveBeenCalled());

        const renameInput = screen.getByPlaceholderText("rename to import (leave blank to skip)");
        await userEvent.type(renameInput, "taken-elsewhere");

        expect(await screen.findByText("This name is also taken — pick another.")).toBeInTheDocument();
    });

    it("shows no conflict hint for a rename that matches nothing in the fetched catalog", async () => {
        vi.mocked(RpcApi.SkillCatalogListCommand).mockResolvedValue([]);

        render(() => (
            <BundleImportPreviewModalPanel preview={makePreview()} onNext={() => {}} onCancel={() => {}} />
        ));

        await waitFor(() => expect(RpcApi.SkillCatalogListCommand).toHaveBeenCalled());

        const renameInput = screen.getByPlaceholderText("rename to import (leave blank to skip)");
        await userEvent.type(renameInput, "genuinely-free-name");

        expect(screen.queryByText("This name is also taken — pick another.")).not.toBeInTheDocument();
    });
});

// A bundle carries no MCP servers: the archive's are listed as not imported,
// with nothing to select (SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md §3.1).
describe("BundleImportPreviewModalPanel — MCP servers", () => {
    it("lists them as not imported, without a checkbox", async () => {
        vi.mocked(RpcApi.SkillCatalogListCommand).mockResolvedValue([]);
        const preview = makePreview();
        preview.skills = [];
        preview.mcp_servers = [
            { source_path: "mcp/github.server.json", display: { name: "github", command: "gh-mcp" } },
        ];
        const onNext = vi.fn();

        render(() => <BundleImportPreviewModalPanel preview={preview} onNext={onNext} onCancel={() => {}} />);

        expect(screen.getByText("Not imported: MCP servers")).toBeInTheDocument();
        const row = screen.getByText("github").closest(".bundle-import-checkbox-row")!;
        expect(row.querySelector("input")).toBeNull();
    });
});
