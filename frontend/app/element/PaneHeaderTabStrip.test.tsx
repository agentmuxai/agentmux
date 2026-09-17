// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for PaneHeaderTabStrip — the unified Pane header. A thin wrapper
 * around `BlockFrame_Header`'s `leadingTabStrip` prop (see that component's
 * own doc comment for why cherry-picking pieces like `EndIcons` was
 * rejected — real features, e.g. term's ConnectionButton, live in parts of
 * BlockFrame_Header this file never touches).
 * Spec: docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md §4.1.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PaneHeaderTabStrip } from "./PaneHeaderTabStrip";

const blockFrameHeaderCalls: any[] = [];
vi.mock("@/app/block/blockframe", () => ({
    BlockFrame_Header: (props: any) => {
        blockFrameHeaderCalls.push(props);
        return <div data-testid="block-frame-header">{props.leadingTabStrip}</div>;
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
const TABS: T[] = [{ id: "a", label: "alpha" }];

function fakeNodeModel(): any {
    return { isFocused: () => false, isMagnified: () => false, toggleMagnify: vi.fn(), onClose: vi.fn() };
}

describe("PaneHeaderTabStrip", () => {
    it("renders BlockFrame_Header with a leadingTabStrip containing the tab pills", () => {
        render(() => (
            <PaneHeaderTabStrip
                tabs={TABS}
                activeId="a"
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                nodeModel={fakeNodeModel()}
                viewModel={null}
                activeBlockId={() => "b1"}
            />
        ));
        expect(screen.getByTestId("block-frame-header")).toBeInTheDocument();
        expect(screen.getByText("alpha")).toBeInTheDocument();
        expect(blockFrameHeaderCalls[0].viewModel).toBeNull();
    });

    it("passes connBtnRef/changeConnModalAtom through to BlockFrame_Header unchanged", () => {
        const connBtnRef = { current: null };
        const changeConnModalAtom = (() => false) as any;
        render(() => (
            <PaneHeaderTabStrip
                tabs={TABS}
                activeId="a"
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

    it("shows emptyLabel as plain text when tabs is empty and onAdd is omitted", () => {
        render(() => (
            <PaneHeaderTabStrip
                tabs={[]}
                activeId={null}
                getId={(t: T) => t.id}
                getLabel={(t: T) => t.label}
                onActivate={vi.fn()}
                nodeModel={fakeNodeModel()}
                viewModel={null}
                activeBlockId={() => "b1"}
                emptyLabel="Agent"
            />
        ));
        expect(screen.getByText("Agent")).toBeInTheDocument();
    });
});
