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

vi.mock("@/app/view/agent/hooks/useSubagentBackfillGate", () => ({
    useSubagentBackfillGate: () => () => true,
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
