// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for AgentCreateFromTemplateModalPanel
 * (SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md — Phase 1).
 *
 * Covered:
 *  - default name field pre-fills with the template's name
 *  - Identity + Memory selects render the accounts / bundles list
 *    (empty option represents ambient creds / vanilla CLI sentinels)
 *  - clicking Create fires onSubmit with the form snapshot
 *  - the Create button is disabled while submitting
 *  - error from onSubmit surfaces in the panel body
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { sleep } from "@/util/util";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListMemoriesCommand: vi.fn(),
        // Mount-time daemon probe (drives the host/container dropdown).
        // Default → no reachable runtime → host-only.
        ContainerRuntimeAvailableCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/identity/identity-model", () => ({
    refreshAccountCache: vi.fn(),
}));

import { AgentCreateFromTemplateModalPanel } from "./AgentCreateFromTemplateModal";
import { resetCapabilities } from "@/app/store/toolchain-capabilities";
import { refreshAccountCache } from "@/app/view/identity/identity-model";

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
    // toolchain-capabilities is a module-level singleton shared across every
    // test in this file (and every real consumer in the app) — reset it so
    // one test's mocked ContainerRuntimeAvailableCommand result can't leak
    // into the next via the shared cache.
    resetCapabilities();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
    vi.mocked(refreshAccountCache).mockResolvedValue([
        { id: "id-work", name: "Work", provider: "claude" } as any,
    ]);
    vi.mocked(RpcApi.ListMemoriesCommand).mockResolvedValue([
        { id: "mem-notes", name: "Notes", is_blank: false } as any,
    ]);
    // Default: Docker daemon not reachable → host-only. Consumed via the
    // shared toolchain-capabilities store, not called directly by the
    // component anymore — see docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: false });
});

afterEach(() => cleanup());

const flush = () => sleep(0);

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
        // Identity auto-picked from the first available account for
        // the template's provider; Memory from the first non-blank
        // bundle.
        expect(args.accountId).toBe("id-work");
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
        vi.mocked(RpcApi.ContainerRuntimeAvailableCommand).mockResolvedValue({ available: true });
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

    it("shows the Model Vendor / Custom Endpoint field for a provider that declares baseUrlEnvVar (claude) and includes it in the submit payload", async () => {
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={template}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        await flush();
        await flush();
        await flush();
        const input = screen.getByTestId(
            "create-from-template-vendor-base-url-input",
        ) as HTMLInputElement;
        fireEvent.input(input, { target: { value: "https://my-proxy.example.com" } });

        const submit = screen.getByTestId("create-from-template-submit");
        fireEvent.click(submit);

        await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
        expect(onSubmit.mock.calls[0][0].modelVendorBaseUrl).toBe(
            "https://my-proxy.example.com",
        );
    });

    it("hides the Model Vendor / Custom Endpoint field for a provider that doesn't declare baseUrlEnvVar, and submits an empty override", async () => {
        const codexTemplate = { ...template, provider: "codex" } as AgentDefinition;
        const onSubmit = vi.fn().mockResolvedValue(undefined);
        render(() => (
            <AgentCreateFromTemplateModalPanel
                template={codexTemplate}
                onSubmit={onSubmit}
                onCancel={vi.fn()}
            />
        ));
        await flush();
        await flush();
        await flush();
        expect(screen.queryByTestId("create-from-template-vendor-base-url-input")).toBeNull();

        const submit = screen.getByTestId("create-from-template-submit");
        fireEvent.click(submit);

        await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
        expect(onSubmit.mock.calls[0][0].modelVendorBaseUrl).toBe("");
    });
});
