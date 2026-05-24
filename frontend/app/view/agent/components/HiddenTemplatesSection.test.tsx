// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Phase 2 (Q2 Decision Y) hidden-templates surface
 * (`SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md`).
 *
 * Covers:
 *  - empty state: section disappears entirely (zero footprint)
 *  - non-empty: header surfaces with count, click toggles list
 *  - Unhide button fires AgentDefUnhideCommand and removes the row
 *    optimistically
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => {
    const RpcApi = {
        AgentDefListHiddenTemplatesCommand: vi.fn().mockResolvedValue([]),
        AgentDefUnhideCommand: vi.fn().mockResolvedValue({ ok: true }),
    };
    return { RpcApi };
});
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn(() => () => {}),
}));
vi.mock("@/element/ProviderLogo", () => ({
    ProviderLogo: (props: any) => (
        <span data-testid={`provider-logo-${props.provider}`} />
    ),
}));

import { HiddenTemplatesSection } from "./HiddenTemplatesSection";

let RpcApi: typeof import("@/app/store/rpc-api").RpcApi;

const makeTemplate = (over: Partial<AgentDefinition>): AgentDefinition =>
    ({
        id: "tpl-x",
        slug: "x",
        name: "Template X",
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
        agent_type: "host",
        environment: "local",
        agent_bus_id: "",
        is_seeded: 1,
        user_hidden: 1,
        ...over,
    }) as AgentDefinition;

beforeEach(async () => {
    vi.clearAllMocks();
    ({ RpcApi } = await import("@/app/store/rpc-api"));
});

afterEach(() => {
    cleanup();
});

describe("HiddenTemplatesSection", () => {
    it("renders nothing when no templates are hidden (zero footprint)", async () => {
        vi.mocked(RpcApi.AgentDefListHiddenTemplatesCommand).mockResolvedValue([]);
        render(() => <HiddenTemplatesSection />);
        // Wait one microtask so the initial load resolves.
        await Promise.resolve();
        expect(
            screen.queryByTestId("agent-hidden-templates-section"),
        ).toBeNull();
    });

    it("renders header with count when templates are hidden", async () => {
        vi.mocked(RpcApi.AgentDefListHiddenTemplatesCommand).mockResolvedValue([
            makeTemplate({ id: "tpl-claude", name: "Claude Code" }),
            makeTemplate({ id: "tpl-codex", name: "Codex CLI" }),
        ]);
        render(() => <HiddenTemplatesSection />);
        const toggle = await screen.findByTestId(
            "agent-hidden-templates-toggle",
        );
        expect(toggle.textContent).toContain("Hidden templates");
        expect(toggle.textContent).toContain("(2)");
        // Body is collapsed initially.
        expect(
            screen.queryByTestId("agent-hidden-templates-list"),
        ).toBeNull();
    });

    it("expands on click, lists hidden templates with Unhide buttons", async () => {
        vi.mocked(RpcApi.AgentDefListHiddenTemplatesCommand).mockResolvedValue([
            makeTemplate({ id: "tpl-claude", name: "Claude Code" }),
        ]);
        render(() => <HiddenTemplatesSection />);
        const toggle = await screen.findByTestId(
            "agent-hidden-templates-toggle",
        );
        fireEvent.click(toggle);
        const list = await screen.findByTestId(
            "agent-hidden-templates-list",
        );
        expect(list).not.toBeNull();
        const row = await screen.findByTestId(
            "agent-hidden-template-tpl-claude",
        );
        expect(row.textContent).toContain("Claude Code");
        expect(
            screen.queryByTestId("agent-hidden-template-unhide-tpl-claude"),
        ).not.toBeNull();
    });

    it("Unhide button fires AgentDefUnhideCommand and removes row optimistically", async () => {
        vi.mocked(RpcApi.AgentDefListHiddenTemplatesCommand).mockResolvedValue([
            makeTemplate({ id: "tpl-claude", name: "Claude Code" }),
            makeTemplate({ id: "tpl-codex", name: "Codex CLI" }),
        ]);
        render(() => <HiddenTemplatesSection />);
        const toggle = await screen.findByTestId(
            "agent-hidden-templates-toggle",
        );
        fireEvent.click(toggle);
        const unhide = await screen.findByTestId(
            "agent-hidden-template-unhide-tpl-claude",
        );
        fireEvent.click(unhide);
        await waitFor(() =>
            expect(RpcApi.AgentDefUnhideCommand).toHaveBeenCalledTimes(1),
        );
        expect(vi.mocked(RpcApi.AgentDefUnhideCommand).mock.calls[0][1]).toEqual({
            definition_id: "tpl-claude",
        });
        // Row vanishes from the DOM (optimistic update).
        await waitFor(() =>
            expect(
                screen.queryByTestId("agent-hidden-template-tpl-claude"),
            ).toBeNull(),
        );
        // The other row stays.
        expect(
            screen.queryByTestId("agent-hidden-template-tpl-codex"),
        ).not.toBeNull();
    });

    it("collapses automatically when the list empties out", async () => {
        vi.mocked(RpcApi.AgentDefListHiddenTemplatesCommand).mockResolvedValue([
            makeTemplate({ id: "tpl-only", name: "Only Hidden" }),
        ]);
        render(() => <HiddenTemplatesSection />);
        const toggle = await screen.findByTestId(
            "agent-hidden-templates-toggle",
        );
        fireEvent.click(toggle);
        // Section is expanded.
        await screen.findByTestId("agent-hidden-templates-list");
        // Unhide the only row.
        const unhide = await screen.findByTestId(
            "agent-hidden-template-unhide-tpl-only",
        );
        fireEvent.click(unhide);
        // After the optimistic removal the section unmounts entirely
        // (zero footprint when empty).
        await waitFor(() => {
            expect(
                screen.queryByTestId("agent-hidden-templates-section"),
            ).toBeNull();
        });
    });
});
