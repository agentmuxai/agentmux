// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentIdentityModal must not offer a way to edit `model_vendor_base_url`
 * after the agent's bundle already exists.
 *
 * Issue #2594 ("needs a product decision" item, resolved 2026-08-16):
 * this modal used to let a user write a NEW `model_vendor_base_url`
 * straight onto `AgentDefinition` post-creation — the one remaining
 * post-creation write path for a field the Mandatory ABF architecture
 * (`ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §7.4.1) treats as
 * set-once at definition/bundle-creation time, same as `provider` (which
 * this same modal has never let a user edit). The bundle has no field to
 * mirror the raw base-url string into either, so there was no bundle
 * value to "resolve through" the way #2592/#2607 fixed provider drift —
 * the only consistent fix is to stop writing a second, divergent copy of
 * this field after creation at all. The definition-time UI
 * (`AgentCreateFromTemplateModal.tsx`) is unaffected; it still sets the
 * value once, before/at bundle provisioning.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AgentIdentityModalPanel } from "./AgentIdentityModal";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { UpdateAgentDefinitionCommand: vi.fn() },
}));

vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

vi.mock("@/app/view/identity/identity-model", () => ({
    IdentityViewModel: class {
        dispose(): void {}
    },
    serializeAgentAccounts: vi.fn(() => ""),
}));

vi.mock("./AgentIdentityPanel", () => ({
    AgentIdentityPanel: () => <div data-testid="identity-panel-stub" />,
}));

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

const claudeAgent = {
    id: "agent-1",
    name: "Claude Agent",
    icon: "✦",
    provider: "claude", // PROVIDERS["claude"].baseUrlEnvVar is set — the
    // one case where the (now-removed) vendor section used to render.
    description: "",
    working_directory: "",
    shell: "",
    provider_flags: "",
    auto_start: 0,
    restart_on_crash: 0,
    idle_timeout_minutes: 0,
    created_at: 0,
    agent_type: "host",
    environment: "",
    agent_bus_id: "",
    is_seeded: 0,
    accounts: "",
    parent_id: "",
    branch_label: "",
    updated_at: 0,
    user_hidden: 0,
    container_image: "",
    container_volumes: "[]",
    container_name: "",
    use_ambient_login: 0,
    // Already has a bundle — the exact "already exists and is supposed to
    // be immutable" state the issue calls out.
    memory_id: "mem-1",
    model_vendor_base_url: "",
    auto_continue_enabled: 0,
} as unknown as AgentDefinition;

describe("AgentIdentityModal — no post-creation model-vendor edit surface", () => {
    it("does not render a way to edit model_vendor_base_url for an agent whose bundle already exists", () => {
        render(() => (
            <AgentIdentityModalPanel agent={claudeAgent} blockId="block-1" onClose={vi.fn()} />
        ));

        expect(screen.queryByTestId("agent-identity-vendor-base-url-input")).not.toBeInTheDocument();
        expect(screen.queryByTestId("agent-identity-vendor-base-url-save")).not.toBeInTheDocument();
        expect(screen.queryByText("Model Vendor / Custom Endpoint")).not.toBeInTheDocument();
    });
});
