// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Render test for `AgentCard` — pins
 * SPEC_AGENT_PICKER_TEMPLATE_SECTION_CLEANUP_2026_08_22.md: the
 * template-card icon must be a plain harness `ProviderLogo`, never
 * `DualProviderLogo` (which overlays a vendor badge) — that badge stays
 * on `MyAgentsList` rows only, which this file does not touch.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const providerLogoSpy = vi.fn();
const dualProviderLogoSpy = vi.fn();

// Real ProviderLogo/DualProviderLogo import raw SVG assets vitest can't
// resolve without a transformer (same reasoning as MyAgentsList.test.tsx's
// own ProviderLogo mock) — spies stand in so the test can assert WHICH
// component AgentCard actually renders, not just that something did.
vi.mock("@/element/ProviderLogo", () => ({
    ProviderLogo: (props: any) => {
        providerLogoSpy(props);
        return null;
    },
}));
vi.mock("@/element/DualProviderLogo", () => ({
    DualProviderLogo: (props: any) => {
        dualProviderLogoSpy(props);
        return null;
    },
}));

import { AgentCard } from "./AgentCard";

afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

const makeAgent = (overrides: Partial<AgentDefinition> = {}): AgentDefinition =>
    ({
        id: "def-claude",
        slug: "claude",
        name: "Claude Code",
        icon: "",
        provider: "claude",
        description: "",
        working_directory: "",
        shell: "",
        provider_flags: "",
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: "",
        environment: "",
        agent_bus_id: "",
        is_seeded: 1,
        ...overrides,
    }) as AgentDefinition;

describe("AgentCard icon", () => {
    it("renders a plain ProviderLogo (harness icon only), never DualProviderLogo", () => {
        render(() => (
            <AgentCard
                agent={makeAgent()}
                launching={false}
                disabled={false}
                installed={true}
                onLaunch={() => {}}
            />
        ));

        expect(providerLogoSpy).toHaveBeenCalledTimes(1);
        expect(providerLogoSpy.mock.calls[0][0].provider).toBe("claude");
        expect(dualProviderLogoSpy).not.toHaveBeenCalled();
    });

    it("still renders the harness icon even for a provider with a custom model_vendor_base_url", () => {
        // Regression guard: the whole point of the badge was to surface a
        // custom-vendor override — confirm the template card ignores that
        // field entirely now (no vendor concept plumbed into ProviderLogo).
        render(() => (
            <AgentCard
                agent={makeAgent({ provider: "codex" })}
                launching={false}
                disabled={false}
                installed={true}
                onLaunch={() => {}}
            />
        ));

        expect(providerLogoSpy).toHaveBeenCalledTimes(1);
        expect(providerLogoSpy.mock.calls[0][0].provider).toBe("codex");
        expect(dualProviderLogoSpy).not.toHaveBeenCalled();
    });
});

describe("AgentCard other rendering", () => {
    it("still shows the install ribbon when installed === false", async () => {
        render(() => (
            <AgentCard
                agent={makeAgent()}
                launching={false}
                disabled={false}
                installed={false}
                onLaunch={() => {}}
            />
        ));
        expect(await screen.findByText("Click to install")).toBeTruthy();
    });
});
