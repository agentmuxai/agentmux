// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression coverage: a drag-preview thumbnail (`tabcontent.tsx`'s
 * `renderPreview`, `<Block preview>` with the RAW leaf nodeModel) and the
 * real, non-preview mount of the SAME blockId used to share one
 * ViewModel instance via `block-component-registry.ts`'s blockId-only-keyed
 * "last writer wins" adoption. Reproduced live: the preview mount's effect
 * ran first, so the real mount adopted a ViewModel whose cached `nodeModel`
 * was the preview's own raw leaf model — permanently missing
 * `paneChromeHoisted`, so `noHeader()` stayed stuck `false` and the real
 * pane showed a double header forever. Fixed by never letting a preview
 * mount touch the shared registry at all — it always gets its own private
 * ViewModel. This test asserts that invariant directly: whichever mount's
 * effect runs first, the REAL mount's ViewModel always ends up holding the
 * REAL mount's own nodeModel, never the preview's.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NodeModel } from "@/layout/index";

const blockMetaSignals = new Map<string, ReturnType<typeof createSignal<{ meta?: { view?: string } }>>>();
function setBlockView(blockId: string, view: string | undefined) {
    if (!blockMetaSignals.has(blockId)) {
        blockMetaSignals.set(blockId, createSignal<{ meta?: { view?: string } }>({ meta: { view } }));
    } else {
        blockMetaSignals.get(blockId)![1]({ meta: { view } });
    }
}

vi.mock("@/store/mos", () => ({
    getMuxObjectAtom: (oid: string) => {
        if (!blockMetaSignals.has(oid)) {
            blockMetaSignals.set(oid, createSignal<{ meta?: { view?: string } }>({ meta: {} }));
        }
        return blockMetaSignals.get(oid)![0];
    },
    makeORef: (_otype: string, oid: string) => oid,
    useMuxObjectValue: (oid: string) => {
        if (!blockMetaSignals.has(oid)) {
            blockMetaSignals.set(oid, createSignal<{ meta?: { view?: string } }>({ meta: {} }));
        }
        return [blockMetaSignals.get(oid)![0], () => {}];
    },
}));

const registry = new Map<string, unknown>();
vi.mock("@/store/global", () => ({
    atoms: { prefersReducedMotionAtom: () => false },
    counterInc: () => {},
    getBlockComponentModel: (blockId: string) => registry.get(blockId),
    registerBlockComponentModel: (blockId: string, bcm: unknown) => {
        registry.set(blockId, bcm);
    },
    unregisterBlockComponentModel: (blockId: string, owner?: unknown) => {
        if (owner !== undefined && registry.get(blockId) !== owner) return;
        registry.delete(blockId);
    },
}));

// Records every ViewModel this test's fake view class constructs, so a test
// can inspect which nodeModel instance ended up cached on which mount's vm —
// the exact thing the bug got wrong.
const constructedViewModels: { blockId: string; nodeModel: NodeModel }[] = [];
class TestAgentViewModel {
    viewType = "agent";
    blockId: string;
    nodeModel: NodeModel;
    viewComponent = null;
    noHeader = () => (this.nodeModel as any)?.paneChromeHoisted?.() === true;
    constructor(blockId: string, nodeModel: NodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        constructedViewModels.push({ blockId, nodeModel });
    }
}
vi.mock("@/app/block/block-registry", () => ({
    getBlockViewClass: (view: string) => (view === "agent" ? TestAgentViewModel : null),
}));

vi.mock("./blockframe", () => ({
    BlockFrame: (props: { nodeModel: NodeModel; viewModel: any; preview: boolean; children?: any }) => (
        <div data-testid={`blockframe-${props.preview ? "preview" : "real"}-${props.nodeModel.blockId}`}>
            {props.children}
        </div>
    ),
}));

// The real hook's `settled` is RE-ENTRANT — it calls setSettled(false) on every
// "started" event, including one long after the first cycle settled. A mock
// that returns a constant `true` cannot exercise that, which is exactly how a
// one-way gate silently swallowing the re-cover got through review (P1 on
// #3464). `backfillSettled` is a real signal so a test can flip it.
const [backfillSettled, setBackfillSettled] = createSignal(true);
vi.mock("@/app/view/agent/hooks/useSubagentBackfillGate", () => ({
    useSubagentBackfillGate: () => () => backfillSettled(),
}));

vi.mock("@/layout/index", () => ({
    useDebouncedNodeInnerRect: () => () => undefined,
}));

vi.mock("@/app/platform/ipc", () => ({
    invokeCommand: () => Promise.resolve(),
}));

vi.mock("@/util/focusutil", () => ({
    focusedBlockId: () => null,
}));

async function loadBlock() {
    const mod = await import("./block");
    return mod.Block;
}

function makeNodeModel(overrides: Partial<NodeModel> & { blockId: string }): NodeModel {
    return {
        isFocused: () => false,
        disablePointerEvents: () => false,
        focusNode: () => {},
        ...overrides,
    } as unknown as NodeModel;
}

afterEach(() => {
    cleanup();
    blockMetaSignals.clear();
    registry.clear();
    constructedViewModels.length = 0;
    setBackfillSettled(true);
});

describe("Block — preview/real ViewModel isolation", () => {
    it("the real mount's ViewModel holds the real mount's own nodeModel, even when the preview mount constructs first", async () => {
        setBlockView("b1", "agent");
        const Block = await loadBlock();

        const previewNodeModel = makeNodeModel({ blockId: "b1" }); // raw leaf model — no paneChromeHoisted at all
        const realNodeModel = makeNodeModel({ blockId: "b1", paneChromeHoisted: () => true } as any);

        // Preview mounts FIRST — the exact ordering that reproduced the bug.
        render(() => <Block nodeModel={previewNodeModel} preview={true} />);
        render(() => <Block nodeModel={realNodeModel} preview={false} />);

        // Two SEPARATE ViewModels were constructed — preview never adopted
        // into (or was adopted from) the shared registry.
        expect(constructedViewModels).toHaveLength(2);
        const realVm = constructedViewModels.find((v) => v.nodeModel === realNodeModel);
        expect(realVm).toBeDefined();
        expect((realVm!.nodeModel as any).paneChromeHoisted?.()).toBe(true);

        // The registry holds only the REAL mount's registration.
        const registered = registry.get("b1") as { viewModel: TestAgentViewModel } | undefined;
        expect(registered?.viewModel.nodeModel).toBe(realNodeModel);
    });

    it("the real mount's ViewModel holds the real mount's own nodeModel, even when the preview mount constructs second", async () => {
        setBlockView("b1", "agent");
        const Block = await loadBlock();

        const previewNodeModel = makeNodeModel({ blockId: "b1" });
        const realNodeModel = makeNodeModel({ blockId: "b1", paneChromeHoisted: () => true } as any);

        // Real mounts FIRST this time — the ordering that happened to work
        // by luck before this fix; must still work after it.
        render(() => <Block nodeModel={realNodeModel} preview={false} />);
        render(() => <Block nodeModel={previewNodeModel} preview={true} />);

        const registered = registry.get("b1") as { viewModel: TestAgentViewModel } | undefined;
        expect(registered?.viewModel.nodeModel).toBe(realNodeModel);
        expect(registered?.viewModel.noHeader()).toBe(true);
    });
});

/**
 * Phase 3 of SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md.
 *
 * The headline acceptance criterion is a COUNT: at no point during a pane mount
 * is more than one loading indicator in the DOM. `spin:2` was measured before
 * this, with block.tsx, agent-view.tsx and AgentPicker.tsx each running their
 * own visible/fading/unmount state machine for the same pane.
 */
describe("Block — one loading cover, stated coverage", () => {
    const covers = () => document.querySelectorAll(".agent-pane-loading-overlay");

    it("covers the pane exactly once while its view is still unresolved", async () => {
        // No view type → getBlockViewClass returns null → viewModel() is null →
        // ready() is false. This is the pane-still-assembling state.
        setBlockView("b-assembling", undefined);
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "b-assembling" })} preview={false} />);

        expect(covers().length).toBe(1);
        expect(covers()[0].classList.contains("is-fading")).toBe(false);
    });

    it("sits in flow while there is no mounted content to overlay", async () => {
        setBlockView("b-inflow", undefined);
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "b-inflow" })} preview={false} />);

        // `.block` is rendered by BlockFrame, which does not exist until
        // ready() — so there is no positioned ancestor for `inset: 0` to
        // resolve against, and an absolute cover would collapse or escape the
        // pane depending on the render route.
        expect(covers()[0].classList.contains("is-in-flow")).toBe(true);
    });

    /**
     * The warm-cache path, and the one a naive port gets wrong: if the gates
     * complete during setup, the cover never painted, so fading it out would
     * flash an opaque panel over content that was never hidden. The code this
     * replaced special-cased exactly this ("reflect it directly, no fade").
     */
    it("never paints a cover for a block that is already ready on first flush", async () => {
        setBlockView("b-warm", "agent"); // resolves synchronously → ready() true
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "b-warm" })} preview={false} />);

        expect(covers().length).toBe(0);
    });
});

/**
 * The re-entrancy the one-way controller cannot express (reagent P1 on #3464).
 *
 * `useSubagentBackfillGate`'s `settled` flips back to false on EVERY "started"
 * event, including one arriving long after the first cycle settled — that
 * re-arm is the hook's round-7 fix, not an edge case. The pre-consolidation
 * code re-showed its spinner for it ("Not ready anymore ... show the spinner
 * again immediately"). Routing it through `PaneReadiness.gate()` type-checks
 * and reads correctly, and silently drops every re-cover after the first,
 * because gate release is one-shot and re-registering after reveal is a
 * documented no-op. So it drives its own cycle instead, rendered by the same
 * cover.
 */
describe("Block — a later backfill cycle re-covers the pane", () => {
    const covers = () => document.querySelectorAll(".agent-pane-loading-overlay");

    it("re-covers live content when backfill re-arms, and clears when it settles", async () => {
        setBlockView("b-rearm", "agent"); // ready() true → assembly cover never paints
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "b-rearm" })} preview={false} />);
        expect(covers().length).toBe(0);

        // A fresh "started" arrives well after the pane went live.
        setBackfillSettled(false);
        expect(covers().length).toBe(1);
        expect(covers()[0].classList.contains("is-fading")).toBe(false); // opaque, no fade in
        // Content stayed mounted underneath — this covers, it does not unmount.
        expect(document.querySelector('[data-testid="blockframe-real-b-rearm"]')).not.toBeNull();

        // ...and settles again, starting the fade rather than a hard cut.
        setBackfillSettled(true);
        expect(covers().length).toBe(1);
        expect(covers()[0].classList.contains("is-fading")).toBe(true);
    });

    it("overlays rather than displacing content on a re-cover", async () => {
        setBlockView("b-rearm2", "agent");
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "b-rearm2" })} preview={false} />);

        setBackfillSettled(false);
        // ready() is true, so there IS mounted content to sit on top of.
        expect(covers()[0].classList.contains("is-in-flow")).toBe(false);
    });
});
