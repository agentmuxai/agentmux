// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the Armory rail (`ArmoryView`).
 *
 * Armory Phase 5
 * (docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md
 * §1/§2/§3) removed the "Identities" tab (folded into the agent-pane's own
 * Identity tab — see agent-identity-links-panel.test.tsx), renamed "Memory"
 * to "Memories", and reordered the rail to
 * Accounts, Memories, Skills, MCP Servers, Bundles. These tests guard the
 * rail contents directly; `ArmorySection`'s type-level rejection of
 * `"identities"` is checked at compile time below (no runtime assertion
 * needed for that part).
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/view/accounts/accounts-manager", () => ({
    AccountsManager: () => <div data-testid="accounts-manager" />,
}));
vi.mock("@/app/view/brain/global-brain-manager", () => ({
    GlobalBrainManager: () => <div data-testid="brain-manager" />,
}));
vi.mock("@/app/view/memory/memory-manager", () => ({
    MemoryManager: () => <div data-testid="memory-manager" />,
}));
vi.mock("@/app/view/mcp/mcp-manager", () => ({
    McpManager: () => <div data-testid="mcp-manager" />,
}));
vi.mock("@/app/view/skill/skill-manager", () => ({
    SkillManager: () => <div data-testid="skill-manager" />,
}));

import { ArmoryView } from "./armory-view";
import { ArmoryViewModel } from "./armory-model";

describe("ArmoryView rail", () => {
    afterEach(() => {
        cleanup();
    });

    function renderArmory() {
        const model = new ArmoryViewModel("test-block", null as any);
        return render(() => (
            <ArmoryView
                blockId="test-block"
                model={model}
                blockRef={{ current: null }}
                contentRef={{ current: null }}
            />
        ));
    }

    it("has no 'Identities' rail entry", () => {
        renderArmory();
        expect(screen.queryByText("Identities")).not.toBeInTheDocument();
        expect(document.querySelector('[class*="fa-id-card"]')).not.toBeInTheDocument();
    });

    it("does not mount the removed identity pane container/class", () => {
        // The `bundle-manager-pane--identity` modifier class (and its mount
        // div) were removed along with the Identities tab. If
        // armory-view.tsx still imported the deleted agent-identities-panel.tsx
        // module, this test file would fail to even import ArmoryView above.
        const { container } = renderArmory();
        expect(container.querySelector(".bundle-manager-pane--identity")).not.toBeInTheDocument();
    });

    it("labels the brain tab 'Memories' (renamed from 'Memory')", () => {
        renderArmory();
        expect(screen.getAllByText("Memories").length).toBeGreaterThan(0);
        expect(screen.queryByText("Memory")).not.toBeInTheDocument();
    });

    it("orders the rail as Accounts, Memories, Skills, MCP Servers, Bundles", () => {
        renderArmory();
        const rail = screen.getByLabelText("Armory section", { selector: "nav.bundle-manager-rail" });
        const labels = Array.from(rail.querySelectorAll("button span")).map((el) => el.textContent);
        expect(labels).toEqual(["Accounts", "Memories", "Skills", "MCP Servers", "Bundles"]);
    });
});

// ── Compile-time check: ArmorySection no longer accepts "identities" ──────
//
// This block exercises no runtime behavior; it only needs to type-check.
// If `ArmorySection` regains an `"identities"` member, the `@ts-expect-error`
// below starts failing to error, which `tsc --noEmit` reports as a compile
// error (an unused `@ts-expect-error` directive is itself a type error).
import type { ArmorySection } from "./armory-model";

function assertArmorySectionRejectsIdentities() {
    // @ts-expect-error — "identities" was removed from ArmorySection (§1).
    const bad: ArmorySection = "identities";
    void bad;
}
void assertArmorySectionRejectsIdentities;
