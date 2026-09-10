// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for the leaf-level chrome-hoisting router
 * (SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md).
 *
 * The acceptance criterion that actually proves the fix, not just "looks
 * right": mount, capture the hoisted chrome's own DOM element reference,
 * drive a switch (the active member changes), and assert `===` on that
 * reference afterward — while the switch-scoped content DOES get a fresh
 * element, proving the inner remount boundary is doing the work the outer
 * one used to.
 */

import { cleanup, render, screen } from "@solidjs/testing-library";
import { createEffect, createSignal, onCleanup } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NodeModel } from "@/layout/index";

// Module-level, reset in afterEach — read by the mocked Block below (via a
// live binding, not a snapshot: the factory body runs lazily on every
// mount, so it always sees whatever this currently points at) and by
// tests, to count how many times renderPaneChrome actually fires.
let renderPaneChromeCallCount = { count: 0 };

// Hoisted mocks — factories run before imports, so the real Block/
// resolveEffectiveViewType and the real WOS store are replaced with
// controllable test doubles before pane-leaf-chrome.tsx (which imports
// both) is ever loaded.
//
// This fake Block deliberately reproduces block.tsx's REAL
// setActiveViewModel timing, not just its end state — ReAgent P1 on this
// PR: an earlier version of this mock never called setActiveViewModel at
// all, so activeViewModel() was driven directly by the test and never
// exercised the null blip a real switch produces (block.tsx's onCleanup
// clears the OLD member's vm SYNCHRONOUSLY, during the <Key>'s own
// reconciliation; the createEffect that sets the NEW member's vm is
// DEFERRED to the next effects flush) — the exact gap that let a
// re-invocation bug ship undetected. Matching that timing here is what
// makes the "does not re-invoke renderPaneChrome" test below meaningful.
vi.mock("@/app/block/block", () => ({
    Block: (props: { nodeModel: NodeModel; preview: boolean }) => {
        const owner = {};
        createEffect(() => {
            (props.nodeModel as any).setActiveViewModel?.(
                {
                    viewType: "agent",
                    viewComponent: null,
                    renderPaneChrome: (_nodeModel: NodeModel, content: any) => {
                        renderPaneChromeCallCount.count++;
                        return (
                            <div data-testid="chrome-root">
                                <div data-testid="chrome-header">chrome</div>
                                {content}
                            </div>
                        );
                    },
                } as unknown as ViewModel,
                owner,
            );
        });
        onCleanup(() => {
            (props.nodeModel as any).setActiveViewModel?.(null, owner);
        });
        return <div data-testid={`block-${props.nodeModel.blockId}`}>{props.nodeModel.blockId}</div>;
    },
    // Identity — the real migration/rename redirect logic (forge->agent
    // etc.) is exercised by block.tsx's own consumer, makeViewModel; here
    // the test drives the effective view type directly via block meta.
    resolveEffectiveViewType: (v: string) => v,
}));

const blockMetaSignals = new Map<string, ReturnType<typeof createSignal<{ meta?: { view?: string } }>>>();
function setBlockView(blockId: string, view: string | undefined) {
    if (!blockMetaSignals.has(blockId)) {
        blockMetaSignals.set(blockId, createSignal<{ meta?: { view?: string } }>({ meta: { view } }));
    } else {
        blockMetaSignals.get(blockId)![1]({ meta: { view } });
    }
}

vi.mock("@/app/store/global", () => ({
    WOS: {
        makeORef: (_otype: string, oid: string) => oid,
        getWaveObjectAtom: (oid: string) => {
            if (!blockMetaSignals.has(oid)) {
                blockMetaSignals.set(oid, createSignal<{ meta?: { view?: string } }>({ meta: {} }));
            }
            return blockMetaSignals.get(oid)![0];
        },
    },
}));

async function loadPaneLeafChrome() {
    const mod = await import("./pane-leaf-chrome");
    return mod.PaneLeafChrome;
}

/** Minimal fake NodeModel — only the fields PaneLeafChrome itself reads.
 *  `activeViewModel` is a plain, externally-set accessor here — fine for
 *  tests that don't drive a real Block mount/unmount switch. */
function makeFakeNodeModel(overrides?: {
    activeBlockId?: () => string;
    hasEverBeenMultiMember?: () => boolean;
    activeViewModel?: () => ViewModel | null;
}): NodeModel {
    return {
        blockId: "b1",
        nodeId: "node-1",
        activeBlockId: overrides?.activeBlockId ?? (() => "b1"),
        hasEverBeenMultiMember: overrides?.hasEverBeenMultiMember ?? (() => false),
        activeViewModel: overrides?.activeViewModel ?? (() => null),
    } as unknown as NodeModel;
}

function fakeChromeViewModel(testId: string): ViewModel {
    return {
        viewType: "agent",
        viewComponent: null as any,
        renderPaneChrome: (_nodeModel: NodeModel, content: any) => (
            <div data-testid={testId}>
                <div data-testid="chrome-header">chrome</div>
                {content}
            </div>
        ),
    } as unknown as ViewModel;
}

/** A NodeModel with a REAL, owner-checked activeViewModel/setActiveViewModel
 *  signal pair — mirroring layoutNodeModels.ts's actual implementation —
 *  so the mocked Block above (which drives it via createEffect/onCleanup,
 *  exactly like real block.tsx) produces the real null-blip timing on
 *  every switch. */
function makeRealisticNodeModel(overrides?: { activeBlockId?: () => string }): NodeModel {
    let owner: object | null = null;
    const [activeViewModelSig, setActiveViewModelSig] = createSignal<ViewModel | null>(null);
    const setActiveViewModel = (vm: ViewModel | null, o: object) => {
        if (vm === null) {
            if (owner !== o) return;
            owner = null;
            setActiveViewModelSig(null);
            return;
        }
        owner = o;
        setActiveViewModelSig(() => vm);
    };
    return {
        blockId: "b1",
        nodeId: "node-1",
        activeBlockId: overrides?.activeBlockId ?? (() => "b1"),
        hasEverBeenMultiMember: () => true,
        activeViewModel: activeViewModelSig,
        setActiveViewModel,
    } as unknown as NodeModel;
}

afterEach(() => {
    cleanup();
    blockMetaSignals.clear();
    renderPaneChromeCallCount = { count: 0 };
});

describe("PaneLeafChrome — passthrough (not hoisted)", () => {
    it("renders the block directly, with no chrome wrapper, when the leaf has never been multi-member", async () => {
        setBlockView("b1", "agent");
        const nodeModel = makeFakeNodeModel({ hasEverBeenMultiMember: () => false });
        const PaneLeafChrome = await loadPaneLeafChrome();

        render(() => <PaneLeafChrome nodeModel={nodeModel} />);

        expect(screen.getByTestId("block-b1")).toBeInTheDocument();
        expect(screen.queryByTestId("chrome-header")).toBeNull();
    });

    it("renders the block directly when hasEverBeenMultiMember is true but the effective view type isn't agent", async () => {
        setBlockView("b1", "term");
        const nodeModel = makeFakeNodeModel({
            hasEverBeenMultiMember: () => true,
            activeViewModel: () => fakeChromeViewModel("chrome-root"),
        });
        const PaneLeafChrome = await loadPaneLeafChrome();

        render(() => <PaneLeafChrome nodeModel={nodeModel} />);

        expect(screen.getByTestId("block-b1")).toBeInTheDocument();
        expect(screen.queryByTestId("chrome-header")).toBeNull();
    });
});

describe("PaneLeafChrome — hoisted", () => {
    it("wraps the block in the active ViewModel's renderPaneChrome once multi-member and view type is agent", async () => {
        setBlockView("b1", "agent");
        const nodeModel = makeFakeNodeModel({
            hasEverBeenMultiMember: () => true,
            activeViewModel: () => fakeChromeViewModel("chrome-root"),
        });
        const PaneLeafChrome = await loadPaneLeafChrome();

        render(() => <PaneLeafChrome nodeModel={nodeModel} />);

        expect(screen.getByTestId("chrome-root")).toBeInTheDocument();
        expect(screen.getByTestId("chrome-header")).toBeInTheDocument();
        // Content still renders, now NESTED inside chrome.
        expect(screen.getByTestId("chrome-root").contains(screen.getByTestId("block-b1"))).toBe(true);
    });

    // The acceptance criterion: chrome's own DOM node survives a switch
    // (same reference, not just equal-looking markup), while the
    // switch-scoped block content gets a genuinely fresh element — proving
    // the inner remount boundary is what updates displayed content now,
    // not a full teardown of the chrome around it. Uses the REALISTIC
    // node model (real activeViewModel signal, driven by the mocked
    // Block's own createEffect/onCleanup) so this actually exercises the
    // null-blip timing a real switch produces.
    it("keeps the SAME chrome DOM node across a switch, while the block content remounts", async () => {
        setBlockView("b1", "agent");
        setBlockView("b2", "agent");
        const [activeBlockId, setActiveBlockId] = createSignal("b1");
        const nodeModel = makeRealisticNodeModel({ activeBlockId });
        const PaneLeafChrome = await loadPaneLeafChrome();

        render(() => <PaneLeafChrome nodeModel={nodeModel} />);

        const chromeBefore = screen.getByTestId("chrome-root");
        const headerBefore = screen.getByTestId("chrome-header");
        const blockBefore = screen.getByTestId("block-b1");

        setActiveBlockId("b2");

        const chromeAfter = screen.getByTestId("chrome-root");
        const headerAfter = screen.getByTestId("chrome-header");
        expect(chromeAfter).toBe(chromeBefore); // same DOM node — chrome did NOT remount
        expect(headerAfter).toBe(headerBefore);

        // The old member's block element is gone; a fresh one for the new
        // active member takes its place — this IS the mechanism that
        // updates displayed content now.
        expect(screen.queryByTestId("block-b1")).toBeNull();
        const blockAfter = screen.getByTestId("block-b2");
        expect(blockAfter).not.toBe(blockBefore);
    });

    // ReAgent P1 on this PR, the direct regression test for the exact bug
    // found: NodeModel.activeViewModel() genuinely blips through `null` on
    // EVERY switch (see the mocked Block's own comment above for why), and
    // a naive `<Show when={nodeModel.activeViewModel()}>` callback form
    // would treat that as a fresh falsy->truthy edge and re-invoke
    // renderPaneChrome — mounting a BRAND NEW AgentPaneChrome each time.
    // This asserts the call count directly, the strongest form of the
    // claim: not just "the DOM node happens to look the same" but "chrome
    // was genuinely constructed exactly once," across three switches.
    it("invokes renderPaneChrome exactly once per leaf, never again on later switches", async () => {
        setBlockView("b1", "agent");
        setBlockView("b2", "agent");
        setBlockView("b3", "agent");
        const [activeBlockId, setActiveBlockId] = createSignal("b1");
        const nodeModel = makeRealisticNodeModel({ activeBlockId });
        const PaneLeafChrome = await loadPaneLeafChrome();

        render(() => <PaneLeafChrome nodeModel={nodeModel} />);
        expect(renderPaneChromeCallCount.count).toBe(1);

        setActiveBlockId("b2");
        expect(renderPaneChromeCallCount.count).toBe(1); // NOT 2

        setActiveBlockId("b3");
        expect(renderPaneChromeCallCount.count).toBe(1); // NOT 3

        setActiveBlockId("b1");
        expect(renderPaneChromeCallCount.count).toBe(1); // switching back doesn't re-invoke either
    });
});
