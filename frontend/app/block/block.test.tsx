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
import { createMemo, createSignal, onCleanup } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { NodeModel } from "@/layout/index";
import { registerPaneTab } from "./pane-tab-registry";

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
    viewName = () => `live name of ${this.blockId}`;
    disposed = false;
    constructor(blockId: string, nodeModel: NodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        constructedViewModels.push({ blockId, nodeModel });
    }
    dispose() {
        this.disposed = true;
    }
}
// Mirrors what SysinfoViewModel (and most view models) do in their
// constructor: create a memo over the block's meta, and read the block
// directly while building (sysinfo's loadInitialData -> numPoints()).
const memoVmDisposals: string[] = [];
class TestMemoViewModel {
    viewType = "memo";
    blockId: string;
    viewComponent = null;
    plotType: () => string | undefined;
    constructor(blockId: string, _nodeModel: NodeModel) {
        this.blockId = blockId;
        const blockAtom = blockMetaSignals.get(blockId)![0] as () => { meta?: Record<string, unknown> };
        void blockAtom()?.meta?.["plot"];
        this.plotType = createMemo(() => blockAtom()?.meta?.["plot"] as string | undefined);
        onCleanup(() => memoVmDisposals.push(blockId));
    }
}
// The one visibility signal (Phase 3), per block, driven by the tests.
const visibilitySignals = new Map<string, ReturnType<typeof createSignal<"active" | "dormant" | "windowHidden">>>();
function visibilityOf(id: string) {
    if (!visibilitySignals.has(id)) visibilitySignals.set(id, createSignal<"active" | "dormant" | "windowHidden">("active"));
    return visibilitySignals.get(id)!;
}
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: (id: string) => visibilityOf(id)[0] }));

// Records a native tab's lifecycle hooks (Phase 3b).
const lifecycle: string[] = [];
registerPaneTab({
    apiVersion: 1,
    view: "lifecycle",
    label: "Lifecycle",
    icon: "l",
    create: (ctx) => ({
        component: () => null as any,
        onActivate: () => lifecycle.push(`activate ${ctx.blockId}`),
        onDeactivate: () => lifecycle.push(`deactivate ${ctx.blockId}`),
        focus: () => (lifecycle.push(`focus ${ctx.blockId}`), true),
    }),
});

// A native pane tab (Pane Tab contract Phase 2b): `create(ctx)`, no class.
// Real registry, not mocked — block.tsx looks the manifest up there.
const nativeCreates: { blockId: string; disposed: boolean }[] = [];
registerPaneTab({
    apiVersion: 1,
    view: "native",
    label: "Native",
    icon: "n",
    create: (ctx) => {
        const rec = { blockId: ctx.blockId, disposed: false };
        nativeCreates.push(rec);
        return { component: () => null as any, liveTitle: () => ({ text: `native ${ctx.blockId}` }), dispose: () => (rec.disposed = true) };
    },
});
vi.mock("@/app/block/block-registry", () => ({
    getBlockViewClass: (view: string) =>
        view === "agent" ? TestAgentViewModel : view === "memo" ? TestMemoViewModel : null,
}));

// The ViewModel each preview mount handed its BlockFrame, by blockId.
const previewViewModels = new Map<string, any>();
vi.mock("./blockframe", () => ({
    BlockFrame: (props: { nodeModel: NodeModel; viewModel: any; preview: boolean; children?: any }) => {
        if (props.preview) previewViewModels.set(props.nodeModel.blockId, props.viewModel);
        return (
            <div data-testid={`blockframe-${props.preview ? "preview" : "real"}-${props.nodeModel.blockId}`}>
                {props.children}
            </div>
        );
    },
}));

// The real hook's `settled` is RE-ENTRANT — it calls setSettled(false) on every
// "started" event, including one long after the first cycle settled. A mock
// that returns a constant `true` cannot exercise that, which is exactly how a
// one-way gate silently swallowing the re-cover got through review (P1 on
// #3464). `backfillSettled` is a real signal so a test can flip it.
//
// NOTE the default here is `true`, while the REAL hook initialises `settled` to
// `false` (useSubagentBackfillGate.ts:113) and only flips it when the async
// backfill completes. That divergence is deliberate — most tests in this file
// are about ViewModel/cover concerns and want a pane with nothing outstanding —
// but it is also load-bearing: defaulting to the convenient value is what hid
// the phase-tracking race below (P2 on #3466). Any test that cares about the
// backfill window must set it false BEFORE rendering, as `covers a fast pane
// whose backfill is still outstanding` does.
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
    memoVmDisposals.length = 0;
    previewViewModels.clear();
    nativeCreates.length = 0;
    lifecycle.length = 0;
    visibilitySignals.clear();
    setBackfillSettled(true);
});

/**
 * block.tsx builds a ViewModel inside a reactive effect. A constructor's memos
 * used to belong to that effect run, and its reads subscribed the effect to the
 * block — so the first meta change re-ran the effect, disposed the run, and
 * with it every memo of the (cached, reused) ViewModel. Sysinfo's plot type
 * changed once, then froze (REPORT_SYSINFO_PLOT_TYPE_AND_BROWSER_PREVIEW_VM_2026_09_25.md §3).
 */
describe("Block — a ViewModel's own reactive state outlives meta changes", () => {
    function setMeta(blockId: string, meta: Record<string, unknown>) {
        const sig = blockMetaSignals.get(blockId);
        if (sig) sig[1]({ meta } as any);
        else blockMetaSignals.set(blockId, createSignal<{ meta?: any }>({ meta }));
    }

    it("a constructor memo keeps tracking the block across repeated meta changes", async () => {
        setMeta("m1", { view: "memo", plot: "CPU" });
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "m1" })} preview={false} />);
        const vm = (registry.get("m1") as { viewModel: TestMemoViewModel }).viewModel;
        expect(vm.plotType()).toBe("CPU");

        setMeta("m1", { view: "memo", plot: "CPU + Mem" });
        expect(vm.plotType()).toBe("CPU + Mem");
        setMeta("m1", { view: "memo", plot: "Mem" });
        expect(vm.plotType()).toBe("Mem");
        expect(memoVmDisposals).toEqual([]);
    });

    it("disposes the ViewModel's own reactive state when the mount that created it unmounts", async () => {
        setMeta("m2", { view: "memo", plot: "CPU" });
        const Block = await loadBlock();
        const { unmount } = render(() => <Block nodeModel={makeNodeModel({ blockId: "m2" })} preview={false} />);
        expect(memoVmDisposals).toEqual([]);
        unmount();
        expect(memoVmDisposals).toEqual(["m2"]);
    });
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

        // Only the real mount constructed a ViewModel — a preview never builds
        // a live instance (see "a preview never builds a live ViewModel").
        expect(constructedViewModels).toHaveLength(1);
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
 * Every tile keeps a drag-preview `<Block preview>` mounted (TileLayout.core.tsx's
 * `previewElement`). It used to construct its own ViewModel of the view's class,
 * whose constructor can have global side effects: BrowserViewModel registers
 * itself in the block-id-keyed browser-pane store (replacing the real pane's
 * projections) and its dispose unregisters that slot — a split left the real
 * browser pane black
 * (REPORT_SYSINFO_PLOT_TYPE_AND_BROWSER_PREVIEW_VM_2026_09_25.md §2).
 */
describe("Block — a preview never builds a live ViewModel", () => {
    it("constructs nothing for a preview mount on its own", async () => {
        setBlockView("p1", "agent");
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "p1" })} preview={true} />);
        expect(constructedViewModels).toHaveLength(0);
        expect(previewViewModels.get("p1")).toBeDefined();
    });

    it("unmounting a preview leaves the real ViewModel alive and registered", async () => {
        setBlockView("p2", "agent");
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "p2" })} preview={false} />);
        const preview = render(() => <Block nodeModel={makeNodeModel({ blockId: "p2" })} preview={true} />);
        preview.unmount();

        expect(constructedViewModels).toHaveLength(1);
        const registered = registry.get("p2") as { viewModel: TestAgentViewModel };
        expect(registered.viewModel.disposed).toBe(false);
    });

    it("the preview's header shows the live ViewModel's name", async () => {
        setBlockView("p3", "agent");
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "p3" })} preview={false} />);
        render(() => <Block nodeModel={makeNodeModel({ blockId: "p3" })} preview={true} />);
        expect(previewViewModels.get("p3").viewName()).toBe("live name of p3");
    });
});

/**
 * Pane Tab contract Phase 3b: the host fires onActivate/onDeactivate from the
 * one visibility signal — on BOTH paths (a remount tab activates by mounting,
 * a kept-alive one by leaving dormancy) — and a tab that becomes active in a
 * focused pane gets focus, whatever tab type the pane started with.
 */
describe("Block — the host fires a tab's activation hooks", () => {
    it("activates on mount, deactivates when it goes dormant, activates again when it comes back", async () => {
        setBlockView("l1", "lifecycle");
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "l1" })} preview={false} />);
        expect(lifecycle).toEqual(["activate l1"]);

        visibilityOf("l1")[1]("dormant");
        visibilityOf("l1")[1]("windowHidden");
        visibilityOf("l1")[1]("active");
        expect(lifecycle).toEqual(["activate l1", "deactivate l1", "activate l1"]);
    });

    it("gives the newly active tab focus only when its pane is focused", async () => {
        setBlockView("l2", "lifecycle");
        const [paneFocused, setPaneFocused] = createSignal(false);
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "l2", isFocused: paneFocused })} preview={false} />);
        visibilityOf("l2")[1]("dormant");
        setPaneFocused(true);
        visibilityOf("l2")[1]("active");
        expect(lifecycle).toEqual(["activate l2", "deactivate l2", "activate l2", "focus l2"]);
    });

    it("deactivates an active tab when it unmounts, and never fires for a preview", async () => {
        setBlockView("l3", "lifecycle");
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "l3" })} preview={true} />);
        const real = render(() => <Block nodeModel={makeNodeModel({ blockId: "l3" })} preview={false} />);
        real.unmount();
        expect(lifecycle).toEqual(["activate l3", "deactivate l3"]);
    });
});

describe("Block — a native pane tab (create(ctx))", () => {
    it("creates the instance once, registers it as the pane's ViewModel, and disposes it on unmount", async () => {
        setBlockView("n1", "native");
        const Block = await loadBlock();
        const { unmount } = render(() => <Block nodeModel={makeNodeModel({ blockId: "n1" })} preview={false} />);

        expect(nativeCreates).toEqual([{ blockId: "n1", disposed: false }]);
        const vm = (registry.get("n1") as { viewModel: ViewModel }).viewModel;
        expect(vm.viewType).toBe("native");
        expect(vm.viewName?.()).toBe("native n1");

        unmount();
        expect(nativeCreates).toEqual([{ blockId: "n1", disposed: true }]);
    });

    it("a preview of a native tab never creates an instance", async () => {
        setBlockView("n2", "native");
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "n2" })} preview={true} />);
        expect(nativeCreates).toHaveLength(0);
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

/**
 * The realistic ordering for a fast-resolving pane with a slow backfill, and
 * the one the convenient mock default hid (reagent P1/P2 on #3466).
 *
 * The real hook initialises `settled` to FALSE and leaves it there until the
 * async backfill completes. So `ready()` — and with it the whole controller —
 * can reach `live` while the backfill signal has never changed value once. An
 * effect that reads the phase through `untrack` never re-runs to notice, the
 * cover never appears, and the pane reveals mid-backfill: exactly the Activity
 * Dock flicker rounds 2-8 of that hook exist to prevent.
 */
describe("Block — backfill outstanding from the very start", () => {
    const covers = () => document.querySelectorAll(".agent-pane-loading-overlay");

    it("covers a fast pane whose backfill is still outstanding", async () => {
        setBackfillSettled(false); // never changes value during this test
        setBlockView("b-slowfill", "agent"); // ready() true immediately
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "b-slowfill" })} preview={false} />);

        // The pane is live, but the backfill has not settled — it must still be
        // covered, over already-mounted content.
        expect(covers().length).toBe(1);
        expect(covers()[0].classList.contains("is-fading")).toBe(false);
        expect(covers()[0].classList.contains("is-in-flow")).toBe(false);
        expect(document.querySelector('[data-testid="blockframe-real-b-slowfill"]')).not.toBeNull();
    });

    it("starts fading only once that first backfill finally settles", async () => {
        setBackfillSettled(false);
        setBlockView("b-slowfill2", "agent");
        const Block = await loadBlock();
        render(() => <Block nodeModel={makeNodeModel({ blockId: "b-slowfill2" })} preview={false} />);
        expect(covers()[0].classList.contains("is-fading")).toBe(false);

        setBackfillSettled(true);
        expect(covers().length).toBe(1);
        expect(covers()[0].classList.contains("is-fading")).toBe(true);
    });
});
