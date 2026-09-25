// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneHeaderTabStrip — the unified Pane header. A thin wrapper
 * around `BlockFrame_Header`'s `leadingTabStrip` prop (see that component's
 * own doc comment for why cherry-picking pieces like `EndIcons` was
 * rejected — real features/identity live in parts of BlockFrame_Header this
 * file never touches).
 * Spec: docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md §4.1,
 * and SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md §8
 * (a Pane header always shows its tabs — `pane:tabstrip` was removed).
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PaneHeaderTabStrip } from "./PaneHeaderTabStrip";

// PaneTabStrip's `reserveDragHandle` (always on for a Pane header, §3.6 of
// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md) observes the strip via
// ResizeObserver — not present in jsdom by default. Stubbed, not exercised —
// this file doesn't test overflow behavior itself (PaneTabStrip.test.tsx
// does); this just keeps these tests from crashing on mount.
beforeEach(() => {
    (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
        observe() {}
        unobserve() {}
        disconnect() {}
    };
});

const blockFrameHeaderCalls: any[] = [];
vi.mock("@/app/block/blockframe", () => ({
    BlockFrame_Header: (props: any) => {
        blockFrameHeaderCalls.push(props);
        return <div data-testid="block-frame-header">{props.leadingTabStrip}</div>;
    },
}));
// A settings.json written before `pane:tabstrip` was removed can still carry
// "multi-only". Served here for every key read, so a lone tab rendering a
// pill below proves the stale value is ignored rather than merely absent.
vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: (key: string) => () => (key === "pane:tabstrip" ? "multi-only" : undefined),
}));

afterEach(() => {
    cleanup();
    blockFrameHeaderCalls.length = 0;
});

interface T {
    id: string;
    label: string;
}

function fakeNodeModel(): any {
    return { isFocused: () => false, isMagnified: () => false, toggleMagnify: vi.fn(), onClose: vi.fn() };
}

function renderStrip(tabs: T[], extra: Record<string, unknown> = {}) {
    return render(() => (
        <PaneHeaderTabStrip
            tabs={tabs}
            activeId={tabs[0]?.id ?? null}
            getId={(t: T) => t.id}
            getLabel={(t: T) => t.label}
            onActivate={vi.fn()}
            nodeModel={fakeNodeModel()}
            viewModel={null}
            activeBlockId={() => "b1"}
            {...extra}
        />
    ));
}

describe("PaneHeaderTabStrip", () => {
    it("with 2+ tabs: overrides the iconview with a pill strip (leadingTabStrip set)", () => {
        renderStrip([
            { id: "a", label: "alpha" },
            { id: "b", label: "beta" },
        ]);
        expect(screen.getByText("alpha")).toBeInTheDocument();
        expect(screen.getByText("beta")).toBeInTheDocument();
        expect(blockFrameHeaderCalls[0].leadingTabStrip).toBeTruthy();
    });

    it("with ONE tab: still a pill strip — a Pane header always shows its tabs, even with a stale pane:tabstrip=multi-only", () => {
        const { container } = renderStrip([{ id: "a", label: "alpha" }]);
        expect(blockFrameHeaderCalls[0].leadingTabStrip).toBeTruthy();
        expect(container.querySelectorAll(".pane-tab")).toHaveLength(1);
        expect(screen.getByText("alpha")).toBeInTheDocument();
    });

    it("no longer passes a separate trailing add button — the strip carries its own '+'", () => {
        renderStrip([{ id: "a", label: "alpha" }], { onAdd: vi.fn(), addTitle: "New agent" });
        expect(blockFrameHeaderCalls[0].trailingAddButton).toBeUndefined();
        // Exactly one "+", inside the strip.
        expect(screen.getAllByLabelText("New agent")).toHaveLength(1);
    });

    it("passes connBtnRef/changeConnModalAtom through to BlockFrame_Header unchanged", () => {
        const connBtnRef = { current: null };
        const changeConnModalAtom = (() => false) as any;
        renderStrip([{ id: "a", label: "alpha" }], { connBtnRef, changeConnModalAtom });
        expect(blockFrameHeaderCalls[0].connBtnRef).toBe(connBtnRef);
        expect(blockFrameHeaderCalls[0].changeConnModalAtom).toBe(changeConnModalAtom);
    });
});
