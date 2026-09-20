// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneHeaderTabStrip — the unified Pane header. A thin wrapper
 * around `BlockFrame_Header`'s `leadingTabStrip`/`trailingAddButton` props
 * (see that component's own doc comment for why cherry-picking pieces like
 * `EndIcons`, or unconditionally overriding the iconview even for a lone
 * tab, were both rejected — real features/identity live in parts of
 * BlockFrame_Header this file never touches).
 * Spec: docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md §4.1.
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
        return (
            <div data-testid="block-frame-header">
                {props.leadingTabStrip}
                {!props.leadingTabStrip && props.trailingAddButton}
            </div>
        );
    },
}));
vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: () => () => undefined,
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

describe("PaneHeaderTabStrip", () => {
    it("with 2+ tabs: overrides the iconview with a pill strip (leadingTabStrip set)", () => {
        render(() => (
            <PaneHeaderTabStrip
                tabs={[{ id: "a", label: "alpha" }, { id: "b", label: "beta" }] as T[]}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                nodeModel={fakeNodeModel()}
                viewModel={null}
                activeBlockId={() => "b1"}
            />
        ));
        expect(screen.getByText("alpha")).toBeInTheDocument();
        expect(screen.getByText("beta")).toBeInTheDocument();
        expect(blockFrameHeaderCalls[0].leadingTabStrip).toBeTruthy();
    });

    it("with 0 tabs and no onAdd (fresh/unlaunched pane): does NOT override the iconview, and shows no add button", () => {
        render(() => (
            <PaneHeaderTabStrip
                tabs={[] as T[]}
                activeId={null}
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                nodeModel={fakeNodeModel()}
                viewModel={null}
                activeBlockId={() => "b1"}
            />
        ));
        // The real BlockFrame_Header iconview would render here in
        // production — this mock just proves leadingTabStrip was NOT set,
        // i.e. PaneHeaderTabStrip didn't try to substitute anything for it.
        expect(blockFrameHeaderCalls[0].leadingTabStrip).toBeUndefined();
    });

    // Regression for ReAgent P1 on PR #3309 (round 2): with no tabs to show,
    // the real iconview must stay (an earlier version replaced it with a
    // hardcoded literal), with the "+" appended separately. PaneChrome
    // always passes at least one tab now, but the component still has to
    // handle an empty list correctly.
    it("with 0 tabs and onAdd set: does NOT override the iconview, but DOES show the add button", () => {
        const onAdd = vi.fn();
        render(() => (
            <PaneHeaderTabStrip
                tabs={[] as T[]}
                activeId={null}
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                nodeModel={fakeNodeModel()}
                viewModel={null}
                activeBlockId={() => "b1"}
                onAdd={onAdd}
                addTitle="New agent"
            />
        ));
        expect(blockFrameHeaderCalls[0].leadingTabStrip).toBeUndefined();
        expect(screen.getByLabelText("New agent")).toBeInTheDocument();
    });

    it("passes connBtnRef/changeConnModalAtom through to BlockFrame_Header unchanged", () => {
        const connBtnRef = { current: null };
        const changeConnModalAtom = (() => false) as any;
        render(() => (
            <PaneHeaderTabStrip
                tabs={[] as T[]}
                activeId={null}
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                nodeModel={fakeNodeModel()}
                viewModel={null}
                activeBlockId={() => "b1"}
                connBtnRef={connBtnRef}
                changeConnModalAtom={changeConnModalAtom}
            />
        ));
        expect(blockFrameHeaderCalls[0].connBtnRef).toBe(connBtnRef);
        expect(blockFrameHeaderCalls[0].changeConnModalAtom).toBe(changeConnModalAtom);
    });
});
