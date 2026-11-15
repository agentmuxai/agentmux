// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The drops offered on a pane are outcomes: on a set of realistic trees, for every dragged pane over every
 * other pane, no two offered drops leave the same layout, none leaves the layout as it is, and every layout
 * the reducer can make for that pair is offered once.
 */

import { describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { futureLayoutOf, type FutureLayout } from "@/layout/lib/futureLayout";
import { dropActionOf, dropOptionsOf, nearestPane, resetDropOptions, type DropOption, type DropSubject } from "@/layout/lib/dropOptions";
import { GUIDE_GRACE_MS, guideSpots, hideAllGuides, hideGuide, onGuidesShown, showGuide, spotAt, type GuideSpot } from "@/layout/lib/dropGuides";
import { pillStandIn } from "@/layout/lib/paneBodyDrop";
import { walkNodes } from "@/layout/lib/layoutNode";
import { DropDirection, FlexDirection, type LayoutNode } from "@/layout/lib/types";
import type { SignalAtom } from "@/util/util";

const rows = new Map<string, SignalAtom<LayoutState>>();
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: { rpcCall: async () => ({ applied: true, version: Number.MAX_SAFE_INTEGER }) },
}));
vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (_otype: string, oid: string) => oid,
        getMuxObjectAtom: (oid: string) => {
            if (!rows.has(oid)) {
                const [get, set] = createSignal<LayoutState>({ otype: "layout", oid, version: 1, meta: {} } as LayoutState);
                const atom = () => get();
                (atom as any)._set = set;
                rows.set(oid, atom as unknown as SignalAtom<LayoutState>);
            }
            return rows.get(oid);
        },
        getObjectValue: () => undefined,
        setObjectValue: () => {},
    },
    getSettingsKeyAtom: () => createSignal(0.75)[0],
    globalStore: { get: () => undefined, set: () => {} },
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { requestNodeFocus: () => {}, refocusNode: () => {}, claimFocusOnMount: () => {}, ensureSelectionFocused: () => {} },
}));

const leaf = (id: string, size = 10): LayoutNode => ({ id, size, data: { blockId: id } }) as LayoutNode;
// A pane carries the axis it would split along, the other one than its group's, as the tree gives it
// (`addChildAt`, layoutNode.ts): without it every side of a pane reads as "next to it in its group".
const group = (flexDirection: FlexDirection, id: string, size: number, children: LayoutNode[]): LayoutNode => {
    const across = flexDirection === FlexDirection.Row ? FlexDirection.Column : FlexDirection.Row;
    for (const child of children) if (!child.children?.length) child.flexDirection = across;
    return { id, size, flexDirection, children } as LayoutNode;
};
const row = (id: string, size: number, ...children: LayoutNode[]): LayoutNode => group(FlexDirection.Row, id, size, children);
const col = (id: string, size: number, ...children: LayoutNode[]): LayoutNode => group(FlexDirection.Column, id, size, children);

const TREES: Record<string, () => LayoutNode> = {
    "two panes side by side": () => row("r", 10, leaf("a"), leaf("b")),
    "two panes stacked": () => col("r", 10, leaf("a"), leaf("b")),
    "three in a row": () => row("r", 10, leaf("a"), leaf("b"), leaf("c")),
    "a pane beside a stack of two": () => row("r", 10, leaf("a"), col("col", 10, leaf("b"), leaf("c"))),
    "two rows in a column, uneven": () =>
        col("r", 10, row("r1", 6, leaf("a", 7), leaf("b", 13)), row("r2", 14, leaf("c"), leaf("d"))),
    "three columns, one nested": () =>
        row("r", 10, leaf("a", 5), col("col", 12, leaf("b"), row("r2", 10, leaf("c"), leaf("d"))), leaf("e", 8)),
};

function modelOf(tree: LayoutNode): LayoutModel {
    const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
    const m = new LayoutModel(tab);
    m.getBoundingRect = () => ({ top: 0, left: 0, width: 1200, height: 800 });
    m.displayContainerRef.current = { getBoundingClientRect: () => ({ top: 0, left: 0, width: 1200, height: 800 }) } as any;
    m.treeState.rootNode = structuredClone(tree);
    m.updateTree();
    return m;
}

const leafIds = (tree: LayoutNode): string[] => {
    const ids: string[] = [];
    walkNodes(tree, (n) => void (!n.children?.length && ids.push(n.id)));
    return ids;
};

const ALL = [
    DropDirection.Top,
    DropDirection.Right,
    DropDirection.Bottom,
    DropDirection.Left,
    DropDirection.OuterTop,
    DropDirection.OuterRight,
    DropDirection.OuterBottom,
    DropDirection.OuterLeft,
    DropDirection.Center,
];

/** A layout as the user sees it: where each block is. */
const seen = (f: FutureLayout): string =>
    f.leafIds
        .map((id) => `${f.leafBlockIds[id]}:${["left", "top", "width", "height"].map((k) => Math.round((f.rects[id] as any)[k])).join(",")}`)
        .sort()
        .join(";");

const standing = (m: LayoutModel): string =>
    m
        .leafs()
        .map((l) => `${l.data?.blockId}:${["left", "top", "width", "height"].map((k) => Math.round((m.getNodeRectById(l.id) as any)[k])).join(",")}`)
        .sort()
        .join(";");

describe("dropOptionsOf: a pane of the tree", () => {
    for (const [name, make] of Object.entries(TREES)) {
        it(`offers each layout once, and never the layout as it is: ${name}`, () => {
            const tree = make();
            const table: string[] = [];
            for (const dragged of leafIds(tree)) {
                for (const target of leafIds(tree)) {
                    if (dragged === target) continue;
                    resetDropOptions();
                    const m = modelOf(tree);
                    const subject: DropSubject = { targetId: target, draggedId: dragged };
                    const options = dropOptionsOf(m, subject);
                    const offered = options.map((o) => seen(o.future!));
                    // No two alike, none the layout as it stands.
                    expect(new Set(offered).size).toBe(offered.length);
                    expect(offered).not.toContain(standing(m));
                    // Every layout the reducer can make for the pair is offered.
                    const reachable = new Set<string>();
                    for (const direction of ALL) {
                        const action = dropActionOf(m, subject, direction);
                        const future = action && futureLayoutOf(m, action);
                        if (future && seen(future) !== standing(m)) reachable.add(seen(future));
                    }
                    expect(new Set(offered)).toEqual(reachable);
                    // The same array for the same pane of the same layout: the spots keep their elements.
                    expect(dropOptionsOf(m, subject)).toBe(options);
                    table.push(`${dragged} over ${target}: ${options.map((o) => DropDirection[o.direction]).join(" ")}`);
                }
            }
            console.log(`${name}\n  ${table.join("\n  ")}`);
        });
    }
});

describe("dropOptionsOf: a tab pill", () => {
    it("splits the hovered pane on each side, and joins in the middle; over its own pane it does not join", () => {
        const m = modelOf(TREES["three in a row"]());
        const stand = pillStandIn("x");
        const other = dropOptionsOf(m, { targetId: "b", draggedId: stand.id, foreignNode: stand, pill: true, center: "join" });
        expect(other.map((o) => DropDirection[o.direction])).toEqual(["Top", "Right", "Bottom", "Left", "Center"]);
        expect(other.find((o) => o.direction === DropDirection.Center)?.join).toBe(true);
        const own = dropOptionsOf(m, { targetId: "b", draggedId: stand.id, foreignNode: stand, pill: true, center: "none" });
        expect(own.map((o) => DropDirection[o.direction])).toEqual(["Top", "Right", "Bottom", "Left"]);
    });
});

describe("guideSpots", () => {
    it("puts each drop's spot inside the hovered pane, apart from the others, and reads the pointer on it", () => {
        const m = modelOf(TREES["three columns, one nested"]());
        const now = m.leafs().map((l) => ({ id: l.id, rect: m.getNodeRectById(l.id) }));
        for (const target of ["a", "b", "e"]) {
            const rect = m.getNodeRectById(target);
            const options = dropOptionsOf(m, { targetId: target, draggedId: "c" });
            const spots = guideSpots(rect, target, options, now);
            expect(spots.map((s) => s.direction)).toEqual(options.map((o) => o.direction));
            for (const spot of spots) {
                expect(spotAt(spots, { x: spot.x, y: spot.y })?.direction).toBe(spot.direction);
                expect(spot.panes.filter((p) => p.moved)).toHaveLength(1);
            }
            for (const a of spots) {
                for (const b of spots) {
                    if (a === b) continue;
                    const apart = Math.abs(a.x - b.x) >= a.width || Math.abs(a.y - b.y) >= a.height;
                    expect(apart).toBe(true);
                }
            }
        }
    });

    // A pane with a drop on each side and one beside its whole group on each side: the most spots across.
    const ALL_AROUND: DropOption[] = [
        DropDirection.Top,
        DropDirection.Right,
        DropDirection.Bottom,
        DropDirection.Left,
        DropDirection.Center,
        DropDirection.OuterTop,
        DropDirection.OuterRight,
        DropDirection.OuterBottom,
        DropDirection.OuterLeft,
    ].map((direction) => ({ direction, join: true as const }));
    const paneAt = (width: number, height: number) => ({ left: 400, top: 300, width, height });
    const spotsIn = (width: number, height: number) => {
        const rect = paneAt(width, height);
        // A layout twice as wide as it is high, the hovered pane one of its panes.
        const now = [{ id: "t", rect }, { id: "rest", rect: { left: 0, top: 0, width: 1600, height: 800 } }];
        return { rect, spots: guideSpots(rect, "t", ALL_AROUND, now) };
    };
    const isInside = (spot: GuideSpot, rect: Dimensions): boolean =>
        spot.x - spot.width / 2 >= rect.left &&
        spot.x + spot.width / 2 <= rect.left + rect.width &&
        spot.y - spot.height / 2 >= rect.top &&
        spot.y + spot.height / 2 <= rect.top + rect.height;

    it("keeps every spot inside the pane at any size, none over another: the part over the next pane can not be chosen", () => {
        // 174 by 162 and 291 wide are the panes the macOS look check measured edge spots outside of.
        for (const [width, height] of [[174, 162], [291, 347], [320, 200], [387, 347], [900, 600], [120, 500], [500, 90], [100, 100]]) {
            const { rect, spots } = spotsIn(width, height);
            for (const spot of spots) expect(isInside(spot, rect), `${DropDirection[spot.direction]} in ${width}x${height}`).toBe(true);
            for (const a of spots) {
                for (const b of spots) {
                    if (a === b) continue;
                    expect(Math.abs(a.x - b.x) >= a.width || Math.abs(a.y - b.y) >= a.height, `${width}x${height}`).toBe(true);
                }
            }
        }
    });

    it("shrinks the spots for one beside the compass, and leaves that one out only where five of the smallest do not fit", () => {
        const directions = (width: number, height: number) => spotsIn(width, height).spots.map((s) => s.direction);
        // Room at the edges: full size, the outer spots at the pane's edges.
        const roomy = spotsIn(900, 600).spots;
        expect(roomy).toHaveLength(ALL_AROUND.length);
        expect(roomy[0].width).toBe(56);
        expect(roomy.find((s) => s.direction === DropDirection.OuterRight)!.x).toBe(400 + 900 - 12 - 28);
        // 291 wide: five across, a little smaller, the outer spot next to the compass.
        const narrow = spotsIn(291, 600).spots;
        expect(narrow).toHaveLength(ALL_AROUND.length);
        expect(narrow[0].width).toBeLessThan(56);
        expect(narrow[0].width).toBeGreaterThan(28);
        // Too narrow for five of the smallest: no spot beside the compass across, the ones above and below stay.
        expect(directions(120, 500)).not.toContain(DropDirection.OuterLeft);
        expect(directions(120, 500)).not.toContain(DropDirection.OuterRight);
        expect(directions(120, 500)).toContain(DropDirection.OuterTop);
        expect(directions(120, 500)).toContain(DropDirection.Left);
        // And too low for five: none above and below.
        expect(directions(500, 80)).not.toContain(DropDirection.OuterTop);
        expect(directions(500, 80)).toContain(DropDirection.OuterRight);
    });
});

describe("the guides of a drag, for the picture the cursor carries", () => {
    it("says when the first guide of any layout shows, and when the last one is gone, after its grace", () => {
        vi.useFakeTimers();
        const said: boolean[] = [];
        onGuidesShown((shown) => said.push(shown));
        const [one, two] = [{} as LayoutModel, {} as LayoutModel];
        const guide = { targetId: "a", spots: [] };
        showGuide(one, guide);
        showGuide(one, { ...guide, targetId: "b" });
        showGuide(two, guide);
        expect(said).toEqual([true]);
        hideGuide(one);
        expect(said).toEqual([true]);
        // Within the grace the guide is still up, and a sample that shows it again keeps it.
        hideGuide(two, GUIDE_GRACE_MS);
        showGuide(two, guide);
        vi.advanceTimersByTime(GUIDE_GRACE_MS);
        expect(said).toEqual([true]);
        hideGuide(two, GUIDE_GRACE_MS);
        expect(said).toEqual([true]);
        vi.advanceTimersByTime(GUIDE_GRACE_MS);
        expect(said).toEqual([true, false]);
        // A layout with no guide up has nothing to say.
        hideGuide(one);
        hideAllGuides();
        expect(said).toEqual([true, false]);
        showGuide(one, guide);
        hideAllGuides();
        expect(said).toEqual([true, false, true, false]);
        onGuidesShown(() => {});
        vi.useRealTimers();
    });
});

describe("nearestPane", () => {
    it("takes the pane whose rect is nearest, not the one whose middle is", () => {
        // A wide pane beside a narrow one: a point under the wide pane, by its far edge, is nearer the narrow
        // pane's middle, and is still the wide pane's.
        const m = modelOf(row("r", 10, leaf("wide", 30), leaf("narrow", 2)));
        const wide = m.getNodeRectById("wide");
        const below = { x: wide.left + wide.width - 5, y: 850 };
        const near = nearestPane(m, below);
        expect(near?.nodeId).toBe("wide");
        expect(near?.offset).toEqual({ x: below.x, y: wide.top + wide.height });
        expect(nearestPane(m, below, "wide")?.nodeId).toBe("narrow");
    });
});
