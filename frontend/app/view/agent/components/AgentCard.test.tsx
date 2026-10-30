// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Render test for `AgentCard` — pins
 * SPEC_AGENT_PICKER_TEMPLATE_SECTION_CLEANUP_2026_08_22.md: the
 * template-card icon must be a plain harness `ProviderLogo`, never
 * `DualProviderLogo` (which overlays a vendor badge) — that badge stays
 * on `MyAgentsList` rows only, which this file does not touch.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
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

    it("titles the card with the CLI name and captions it with the provider blurb", () => {
        const { container } = render(() => (
            <AgentCard agent={makeAgent()} launching={false} disabled={false} installed={true} onLaunch={() => {}} />
        ));
        expect(container.querySelector(".agent-card-title")?.textContent).toBe("Claude Code");
        expect(container.querySelector(".agent-card-caption")?.textContent).toBe("Anthropic's coding agent");
    });

    it("names the launch action after the CLI, not the blurb", () => {
        render(() => (
            <AgentCard agent={makeAgent()} launching={false} disabled={false} installed={true} onLaunch={() => {}} />
        ));
        expect(screen.getByRole("button", { name: "Launch Claude Code" })).toBeTruthy();
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

// SPEC_FOCUS_FOLLOWS_SELECTION_2026_10_08.md §2d: the picker's focus lands on
// its first card, but typing must still filter, so a printable key goes to the
// pane's filter box.
describe("AgentCard type to filter", () => {
    const renderInPane = (onLaunch = vi.fn()) => {
        render(() => (
            <div data-blockid="blk-1">
                <input class="filter" data-pane-focus />
                <AgentCard agent={makeAgent()} launching={false} disabled={false} installed={true} onLaunch={onLaunch} blockId="blk-1" />
            </div>
        ));
        const card = screen.getByRole("button");
        const filter = document.querySelector<HTMLInputElement>("input.filter")!;
        return { card, filter, onLaunch };
    };

    it("moves a printable key into the pane's filter box", () => {
        const { card, filter } = renderInPane();
        const onInput = vi.fn();
        filter.addEventListener("input", onInput);
        card.focus();

        fireEvent.keyDown(card, { key: "c" });

        expect(document.activeElement).toBe(filter);
        expect(filter.value).toBe("c");
        expect(onInput).toHaveBeenCalledTimes(1);
    });

    // An IME composition's first key is not printable (#4479).
    it("moves the caret to the filter when an IME composition starts, without inserting anything", () => {
        const { card, filter } = renderInPane();
        card.focus();

        fireEvent.keyDown(card, { key: "Process", keyCode: 229 });

        expect(document.activeElement).toBe(filter);
        expect(filter.value).toBe("");
    });

    it("keeps Enter, Space and shortcuts on the card", () => {
        const { card, filter, onLaunch } = renderInPane();
        card.focus();

        fireEvent.keyDown(card, { key: "k", metaKey: true });
        fireEvent.keyDown(card, { key: "ArrowDown" });
        expect(filter.value).toBe("");
        expect(document.activeElement).toBe(card);

        fireEvent.keyDown(card, { key: "Enter" });
        fireEvent.keyDown(card, { key: " " });
        expect(onLaunch).toHaveBeenCalledTimes(2);
    });
});

// docs/specs/SPEC_PROVIDER_CARD_TAGS_FREE_LOCAL_2026_10_29.md §3 and §4: the ribbon is what a
// click will do, the chips are what the harness is.
describe("AgentCard tags", () => {
    const renderTags = (provider: string, props: { installed?: boolean; drift?: "behind-pin" | "current"; hasCurrentSession?: boolean } = {}) =>
        render(() => (
            <AgentCard
                agent={makeAgent({ provider, name: provider })}
                launching={false}
                disabled={false}
                installed={props.installed ?? true}
                drift={props.drift}
                hasCurrentSession={props.hasCurrentSession}
                onLaunch={() => {}}
            />
        ));
    // A chip's own words: its tooltip text is also in the page, as hidden text for assistive tech.
    const chipLabels = (container: HTMLElement) =>
        [...container.querySelectorAll(".ui-tag--chip")].map((c) => {
            const own = c.cloneNode(true) as HTMLElement;
            own.querySelectorAll(".ui-tag-sr").forEach((n) => n.remove());
            return own.textContent?.replace(/\s+/g, " ").trim();
        });

    it("shows Free, then Local, for a free provider that can run a local model", () => {
        const { container } = renderTags("opencode");
        expect(chipLabels(container).slice(0, 2)).toEqual(["Free", "Local"]);
    });

    it("gives Free the accent tone and Local the neutral one", () => {
        const { container } = renderTags("opencode");
        expect(container.querySelector(".ui-tag--accent")?.textContent).toContain("Free");
        expect(container.querySelector(".ui-tag--neutral")?.textContent).toContain("Local");
    });

    it("gives Codex Local and no Free, and Claude Code Local and no Free", () => {
        for (const provider of ["codex", "claude"]) {
            cleanup();
            const { container } = renderTags(provider);
            expect(chipLabels(container)).toEqual(["Local"]);
        }
    });

    it("shows no chip for a provider that is neither free nor local", () => {
        for (const provider of ["muse", "kimi", "antigravity-not-here"]) {
            cleanup();
            const { container } = renderTags(provider);
            expect(container.querySelectorAll(".ui-tag--chip")).toHaveLength(0);
        }
        cleanup();
        const { container } = renderTags("muse");
        expect(container.querySelector(".agent-card--has-tags")).toBeNull();
    });

    it("keeps the install ribbon as a ribbon, apart from the chips, with Free and Local beside it", () => {
        const { container } = renderTags("opencode", { installed: false });
        expect(container.querySelector(".ui-tag--ribbon")?.textContent).toContain("Click to install");
        expect(chipLabels(container).slice(0, 2)).toEqual(["Free", "Local"]);
        expect(container.querySelector(".agent-card--has-action")).not.toBeNull();
    });

    it("says Installing… on the ribbon while the install runs", () => {
        render(() => (
            <AgentCard agent={makeAgent({ provider: "opencode" })} launching={false} disabled={false} installed={false} installing onLaunch={() => {}} />
        ));
        expect(screen.getByText("Installing…")).toBeTruthy();
    });

    it("shows the update note as the last chip, after Free and Local", () => {
        const { container } = renderTags("opencode", { drift: "behind-pin" });
        expect(chipLabels(container)).toEqual(["Free", "Local", "Update available"]);
    });

    it("marks a card with a session as having an action, so a narrow card makes room for it", () => {
        const { container } = renderTags("opencode", { hasCurrentSession: true });
        expect(container.querySelector(".agent-card--has-action")).not.toBeNull();
    });

    it("marks a card narrower than 300px, which folds the chips to two and gives its action its own line", () => {
        let fire: ((width: number) => void) | undefined;
        vi.stubGlobal(
            "ResizeObserver",
            class {
                constructor(cb: (entries: { contentRect: { width: number } }[]) => void) {
                    fire = (width) => cb([{ contentRect: { width } }]);
                }
                observe() {}
                disconnect() {}
            }
        );
        try {
            const { container } = renderTags("opencode", { hasCurrentSession: true, drift: "behind-pin" });
            expect(container.querySelector(".agent-card--narrow")).toBeNull();
            expect(chipLabels(container).length).toBe(3);
            fire!(250);
            expect(container.querySelector(".agent-card--narrow")).not.toBeNull();
            expect(chipLabels(container).length).toBeLessThanOrEqual(3);
            fire!(400);
            expect(container.querySelector(".agent-card--narrow")).toBeNull();
        } finally {
            vi.unstubAllGlobals();
        }
    });

    it("keeps the removed hand-rolled ribbon and badge classes gone", () => {
        const { container } = renderTags("opencode", { installed: false });
        expect(container.querySelector(".agent-card-install-ribbon")).toBeNull();
        expect(container.querySelector(".agent-card-upgrade-badge")).toBeNull();
    });

    it("puts the limits, a catch and the day checked in the Free tooltip text", () => {
        const { container } = renderTags("kilo");
        const detail = container.querySelector('[data-testid="agent-access-tag-detail-free"]');
        expect(detail?.textContent).toContain("200 requests an hour");
        expect(detail?.textContent).toContain("checked 2026-10-29");
    });
});
