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

// vi.mock(...) calls below are hoisted above these — and above the
// AgentCard import that triggers module resolution — so the spies
// referenced inside the mock factories must be created via vi.hoisted()
// too, or the factories close over a still-temporal-dead-zone binding
// (codex P1 on PR #2731).
const { providerLogoSpy, dualProviderLogoSpy } = vi.hoisted(() => ({
    providerLogoSpy: vi.fn(),
    dualProviderLogoSpy: vi.fn(),
}));

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

const { claimFocusOnMount } = vi.hoisted(() => ({
    claimFocusOnMount: vi.fn((_blockId: string, give: () => boolean): void => {
        give();
    }),
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { claimFocusOnMount },
}));

import { AgentCard } from "./AgentCard";
import type { AgentDefinition } from "@/app/store/rpc-api";

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

describe("AgentCard upgrade badge", () => {
    const renderWith = (props: { installed?: boolean; drift?: any }) =>
        render(() => (
            <AgentCard
                agent={makeAgent()}
                launching={false}
                disabled={false}
                installed={props.installed}
                drift={props.drift}
                onLaunch={() => {}}
            />
        ));

    it("shows the badge when the installed CLI is behind the pin", async () => {
        renderWith({ installed: true, drift: "behind-pin" });
        expect(await screen.findByText(/Update available/)).toBeTruthy();
    });

    it("stays silent for every non-actionable drift state", () => {
        // `ahead-of-pin` is untested-but-working, `unknown` means we never
        // made the comparison, `current` is fine. None of these is the user's
        // problem, and a card is a launch affordance, not a version dashboard.
        for (const drift of ["current", "ahead-of-pin", "unknown", undefined]) {
            cleanup();
            renderWith({ installed: true, drift });
            expect(screen.queryByText(/Update available/)).toBeNull();
        }
    });

    it("yields to the install ribbon — never two CTAs on one card", () => {
        // "Not installed" supersedes "out of date": you cannot be running a
        // stale CLI if you are not running one at all.
        renderWith({ installed: false, drift: "behind-pin" });
        expect(screen.queryByText(/Update available/)).toBeNull();
    });
});

// REPORT_AGENT_PANE_SIDE_BY_SIDE_SCROLL_AND_FOCUS_QUIRKS_2026_09_23.md §2: the
// default card used to call a bare `focus()` on mount — scrolling its picker
// down to the "New Agent" header and taking the caret from whatever pane the
// user was typing in.
describe("AgentCard default focus", () => {
    const renderDefault = (over: { disabled?: boolean } = {}) =>
        render(() => (
            <AgentCard
                agent={makeAgent()}
                launching={false}
                disabled={over.disabled ?? false}
                installed={true}
                onLaunch={() => {}}
                blockId="blk-1"
                defaultFocus={true}
            />
        ));

    it("claims focus through the shared pane guard, scoped to its own block", () => {
        renderDefault();
        expect(claimFocusOnMount).toHaveBeenCalledTimes(1);
        expect(claimFocusOnMount.mock.calls[0][0]).toBe("blk-1");
    });

    it("never scrolls its picker when it takes focus", () => {
        const focusSpy = vi.spyOn(HTMLElement.prototype, "focus");
        renderDefault();
        expect(focusSpy).toHaveBeenCalledWith({ preventScroll: true });
        focusSpy.mockRestore();
    });

    it("does not take focus when the pane's guard declines (another pane selected, or caret in an input)", () => {
        claimFocusOnMount.mockImplementationOnce(() => {});
        const focusSpy = vi.spyOn(HTMLElement.prototype, "focus");
        renderDefault();
        expect(focusSpy).not.toHaveBeenCalled();
        focusSpy.mockRestore();
    });

    it("does not claim focus while disabled", () => {
        renderDefault({ disabled: true });
        expect(claimFocusOnMount).not.toHaveBeenCalled();
    });
});
