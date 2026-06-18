// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for AgentCreateFromTemplateModalPanel
 * (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md — Phase 1).
 *
 * Covered:
 *  - default name field pre-fills with the template's name
 *  - Identity + Memory selects render the bundles list (empty option
 *    represents ambient creds / vanilla CLI sentinels)
 *  - clicking Create fires onSubmit with the form snapshot
 *  - the Create button is disabled while submitting
 *  - error from onSubmit surfaces in the panel body
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListIdentityBundlesCommand: vi.fn(),
        ListMemoriesCommand: vi.fn(),
        // Mount-time Docker probe (drives the host/container dropdown).
        // Default rejects → no sandbox available → host-only.
        ResolveCliCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { AgentCreateFromTemplateModalPanel } from "./AgentCreateFromTemplateModal";

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

const template: AgentDefinition = {
    id: "tpl-claude",
    slug: "claude",
    name: "Claude Code",
    icon: "",
    provider: "claude",
    description: "Anthropic's coding agent",
    working_directory: "",
    shell: "",
    provider_flags: "",
    auto_start: 0,
    restart_on_crash: 0,
    idle_timeout_minutes: 0,
    created_at: 0,
    agent_type: "host",
    environment: "local",
    agent_bus_id: "",
    is_seeded: 1,
} as AgentDefinition;

beforeEach(async () => {
    vi.clearAllMocks();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
    vi.mocked(RpcApi.ListIdentityBundlesCommand).mockResolvedValue([
        { id: "id-work", name: "Work", description: "", is_blank: false } as any,
    ]);
    vi.mocked(RpcApi.ListMemoriesCommand).mockResolvedValue([
        { id: "mem-notes", name: "Notes", is_blank: false } as any,
    ]);
    // Default: no container runtime resolvable → host-only.
    vi.mocked(RpcApi.ResolveCliCommand).mockRejectedValue(new Error("not found"));
});

afterEach(() => cleanup());

const flush = () => new Promise<void>((r) => setTimeout(r, 0));

describe("AgentCreateFromTemplateModalPanel", () => {
    it("defaults the name field to the template name", async () => {
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={vi.fn().mockResolvedValue(undefined)}
                onCancel={vi.fn()}
            />
        ));
        const input = screen.getByTestId(
            "create-from-template-name-input",
        ) as HTMLInputElement;
        expect(input.value).toBe("Claude Code");
    });

    it("clicking Create fires onSubmit with form snapshot", async () => {
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        // Bundles auto-pick once loaded.
        await flush();
        await flush();
        await flush();

        const input = screen.getByTestId(
            "create-from-template-name-input",
        ) as HTMLInputElement;
        fireEvent.input(input, { target: { value: "Mary" } });

        const submit = screen.getByTestId("create-from-template-submit");
        fireEvent.click(submit);

        await waitFor(() => {
            expect(onSubmit).toHaveBeenCalledTimes(1);
        });
        const args = onSubmit.mock.calls[0][0];
        expect(args.name).toBe("Mary");
        // Identity + Memory auto-picked from the first non-blank
        // bundle each.
        expect(args.identityId).toBe("id-work");
        expect(args.memoryId).toBe("mem-notes");
        // No Docker → runtime defaults to host (never a mode that
        // can't actually start).
        expect(args.agentType).toBe("host");
    });

    it("disables the container option when no Docker runtime is found", async () => {
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={vi.fn().mockResolvedValue(undefined)}
                onCancel={vi.fn()}
            />
        ));
        // Let the mount-time Docker probe settle (rejects in beforeEach).
        await flush();
        await flush();
        const select = screen.getByTestId(
            "create-from-template-runtime-select",
        ) as HTMLSelectElement;
        const containerOpt = Array.from(select.options).find(
            (o) => o.value === "container",
        )!;
        expect(containerOpt.disabled).toBe(true);
        expect(select.value).toBe("host");
    });

    it("defaults to container when Docker is available and the template suggests it", async () => {
        vi.mocked(RpcApi.ResolveCliCommand).mockResolvedValue({
            cli_path: "/usr/bin/docker",
            version: "27.0",
        } as any);
        const containerTemplate = { ...template, agent_type: "container" } as AgentDefinition;
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={containerTemplate}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        // Wait for the probe to resolve and the default-pick effect to run.
        const select = screen.getByTestId(
            "create-from-template-runtime-select",
        ) as HTMLSelectElement;
        await waitFor(() => expect(select.value).toBe("container"));
        const containerOpt = Array.from(select.options).find(
            (o) => o.value === "container",
        )!;
        expect(containerOpt.disabled).toBe(false);
    });

    it("surfaces an error from onSubmit", async () => {
        const onSubmit = vi.fn().mockRejectedValue(new Error("name exists"));
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        const submit = screen.getByTestId("create-from-template-submit");
        fireEvent.click(submit);
        const err = await screen.findByTestId("create-from-template-error");
        expect(err.textContent).toContain("name exists");
    });

    it("trims whitespace and rejects empty names", async () => {
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        const input = screen.getByTestId(
            "create-from-template-name-input",
        ) as HTMLInputElement;
        fireEvent.input(input, { target: { value: "   " } });
        const submit = screen.getByTestId(
            "create-from-template-submit",
        ) as HTMLButtonElement;
        expect(submit.disabled).toBe(true);
        fireEvent.click(submit);
        await flush();
        expect(onSubmit).not.toHaveBeenCalled();
    });
});
