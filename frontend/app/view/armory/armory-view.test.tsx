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
 * Accounts, Memories, Skills, MCP Servers, ABF.
 *
 * docs/specs/SPEC_ARMORY_MEMORY_GLOBAL_PERSONAL_RENAME_2026_08_22.md renamed
 * "Memories" to "Global Memory" and "Native Memory" to "Personal Memory",
 * and moved Personal Memory up to sit right below Global Memory (both now
 * adjacent, just below Accounts) — one user-facing "Memory" concept split
 * by scope, abstracting away the two structurally different backing
 * systems. Current rail order: Accounts, Global Memory, Personal Memory,
 * Skills, MCP Servers, ABF. These tests guard the rail contents directly;
 * `ArmorySection`'s type-level rejection of `"identities"` is checked at
 * compile time below (no runtime assertion needed for that part).
 */

import { createSignal } from "solid-js";
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

// blockMeta is a real Solid signal, not a plain object — armory-model.ts's
// sectionAtom/viewName are createMemo-derived from model.blockAtom(), so
// reading a plain (non-reactive) stub only ever satisfies a memo's *first*
// (eager, at-construction) computation. Backing the mock with a genuine
// signal, and having the SetMetaCommand mock write into it, reproduces the
// real write -> WPS push -> blockAtom update round trip closely enough for
// clicking a rail item to actually flip the visible/active section here,
// the same way it does against the real backend.
const [blockMeta, setBlockMeta] = createSignal<Record<string, unknown>>({});
vi.mock("@/app/store/wos", () => ({
    makeORef: (type: string, id: string) => `${type}:${id}`,
    getWaveObjectAtom: () => () => ({ meta: blockMeta() }),
    // global.ts/window-identity.ts evaluate a `tabAtom` createMemo at
    // module-init time that calls WOS.getObjectValue — without this stub
    // the import chain crashes during test setup (same gap browser-model
    // .test.ts's wos mock documents).
    getObjectValue: () => ({}),
}));

const setMetaMock = vi.fn((..._args: unknown[]) => {
    const opts = _args[1] as { oref: string; meta: Record<string, unknown> };
    setBlockMeta((prev) => ({ ...prev, ...opts.meta }));
    return Promise.resolve(undefined);
});
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetMetaCommand: (...args: unknown[]) => setMetaMock(...args),
    },
}));

import { ArmoryView } from "./armory-view";
import { ArmoryViewModel } from "./armory-model";

describe("ArmoryView rail", () => {
    afterEach(() => {
        cleanup();
        setBlockMeta({});
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

    it("labels the brain tab 'Global Memory' (renamed from 'Memories')", () => {
        renderArmory();
        expect(screen.getAllByText("Global Memory").length).toBeGreaterThan(0);
        expect(screen.queryByText("Memories")).not.toBeInTheDocument();
        expect(screen.queryByText("Memory")).not.toBeInTheDocument();
    });

    it("labels the native-memory tab 'Personal Memory' (renamed from 'Native Memory')", () => {
        renderArmory();
        expect(screen.getAllByText("Personal Memory").length).toBeGreaterThan(0);
        expect(screen.queryByText("Native Memory")).not.toBeInTheDocument();
    });

    it("orders the rail as Accounts, Global Memory, Personal Memory, Skills, MCP Servers, ABF", () => {
        renderArmory();
        const rail = screen.getByLabelText("Armory section", { selector: "nav.bundle-manager-rail" });
        const labels = Array.from(rail.querySelectorAll("button span")).map((el) => el.textContent);
        expect(labels).toEqual(["Accounts", "Global Memory", "Personal Memory", "Skills", "MCP Servers", "ABF"]);
    });
});

describe("ArmoryView pane title", () => {
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
        setBlockMeta({});
    });

    function renderArmory() {
        const model = new ArmoryViewModel("test-block", null as any);
        const result = render(() => (
            <ArmoryView
                blockId="test-block"
                model={model}
                blockRef={{ current: null }}
                contentRef={{ current: null }}
            />
        ));
        return { ...result, model };
    }

    it("defaults viewName() to 'Accounts' with no armory:section meta", () => {
        const { model } = renderArmory();
        expect(model.viewName()).toBe("Accounts");
    });

    it("clicking a rail item writes armory:section via SetMetaCommand and updates viewName()", () => {
        const { model } = renderArmory();
        const rail = screen.getByLabelText("Armory section", { selector: "nav.bundle-manager-rail" });
        const skillsButton = Array.from(rail.querySelectorAll("button")).find(
            (b) => b.textContent?.includes("Skills"),
        ) as HTMLButtonElement;
        skillsButton.click();
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "armory:section": "skills" } },
        );
        expect(model.viewName()).toBe("Skills");
        const skillsPane = screen.getByTestId("skill-manager").closest(".bundle-manager-pane");
        expect(skillsPane?.classList.contains("is-hidden")).toBe(false);
    });

    it("clicking a tab-bar item writes armory:section via SetMetaCommand and updates viewName()", () => {
        const { model } = renderArmory();
        const tabBar = screen.getByLabelText("Armory section", { selector: "nav.bundle-manager-tab-bar" });
        const mcpButton = Array.from(tabBar.querySelectorAll("button")).find(
            (b) => b.textContent?.includes("MCP Servers"),
        ) as HTMLButtonElement;
        mcpButton.click();
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "armory:section": "mcp" } },
        );
        expect(model.viewName()).toBe("MCP Servers");
    });

    // SPEC_RESPONSIVE_TAB_BAR_TOP_POSITION_2026_08_24.md
    it("renders the tab-bar before the content section, so it sits at the top of the pane", () => {
        renderArmory();
        const tabBar = screen.getByLabelText("Armory section", { selector: "nav.bundle-manager-tab-bar" });
        const section = document.querySelector(".bundle-manager-section");
        expect(tabBar.compareDocumentPosition(section as Node) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it("highlights only the ABF entry in both the rail and the tab-bar", () => {
        renderArmory();
        const rail = screen.getByLabelText("Armory section", { selector: "nav.bundle-manager-rail" });
        const tabBar = screen.getByLabelText("Armory section", { selector: "nav.bundle-manager-tab-bar" });
        for (const nav of [rail, tabBar]) {
            const highlighted = Array.from(nav.querySelectorAll("button.is-abf-highlight"));
            expect(highlighted).toHaveLength(1);
            expect(highlighted[0].textContent).toContain("ABF");
        }
    });

    it("viewName() reflects a pre-seeded armory:section meta value", () => {
        setBlockMeta({ "armory:section": "bundles" });
        const model = new ArmoryViewModel("test-block", null as any);
        expect(model.viewName()).toBe("ABF");
    });

    it("falls back to 'Accounts' for an invalid armory:section meta value", () => {
        setBlockMeta({ "armory:section": "not-a-real-section" });
        const model = new ArmoryViewModel("test-block", null as any);
        expect(model.viewName()).toBe("Accounts");
    });
});

describe("ArmoryView zoom", () => {
    afterEach(() => {
        cleanup();
        setMetaMock.mockClear();
        setBlockMeta({});
    });

    function renderArmory() {
        const model = new ArmoryViewModel("test-block", null as any);
        const result = render(() => (
            <ArmoryView
                blockId="test-block"
                model={model}
                blockRef={{ current: null }}
                contentRef={{ current: null }}
            />
        ));
        return { ...result, model };
    }

    it("applies model.zoomAtom() as a CSS zoom style on the root", () => {
        const { container } = renderArmory();
        const view = container.querySelector(".armory-view") as HTMLElement;
        expect(view.style.zoom).toBe("1");
    });

    it("Ctrl+Wheel down writes a decreased term:zoom via SetMetaCommand", () => {
        const { container } = renderArmory();
        const view = container.querySelector(".armory-view") as HTMLElement;
        view.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: 100, bubbles: true, cancelable: true }));
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "term:zoom": 0.9 } },
        );
    });

    it("Ctrl+Wheel up writes an increased term:zoom via SetMetaCommand", () => {
        const { container } = renderArmory();
        const view = container.querySelector(".armory-view") as HTMLElement;
        view.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "term:zoom": 1.1 } },
        );
    });

    it("plain wheel (no Ctrl) does not trigger a zoom RPC call", () => {
        const { container } = renderArmory();
        const view = container.querySelector(".armory-view") as HTMLElement;
        view.dispatchEvent(new WheelEvent("wheel", { ctrlKey: false, deltaY: 100, bubbles: true, cancelable: true }));
        expect(setMetaMock).not.toHaveBeenCalled();
    });

    it("returning to 1.0 clears the metadata key (writes null)", () => {
        const model = new ArmoryViewModel("test-block", null as any);
        // model.zoomAtom is what the wheel handler and render path both read;
        // overriding it directly (rather than the underlying blockAtom signal
        // it's derived from) sidesteps reactive-system timing entirely.
        (model as any).zoomAtom = () => 0.9;
        const { container } = render(() => (
            <ArmoryView blockId="test-block" model={model} blockRef={{ current: null }} contentRef={{ current: null }} />
        ));
        const view = container.querySelector(".armory-view") as HTMLElement;
        view.dispatchEvent(new WheelEvent("wheel", { ctrlKey: true, deltaY: -100, bubbles: true, cancelable: true }));
        expect(setMetaMock).toHaveBeenCalledWith(
            undefined,
            { oref: "block:test-block", meta: { "term:zoom": null } },
        );
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
