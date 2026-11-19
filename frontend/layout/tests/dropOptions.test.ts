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
import { dropActionOf, dropOptionsOf, nearestPane, resetDropOptions, wideOptionsOf, type DropOption, type DropSubject } from "@/layout/lib/dropOptions";
import { GUIDE_GRACE_MS, guideSpots, hideAllGuides, hideGuide, onGuidesShown, showGuide, spotAt, wideGuideSpots, wideZoneAt, withWideSpots, type GuideSpot } from "@/layout/lib/dropGuides";
import { pillStandIn } from "@/layout/lib/paneBodyDrop";
import { walkNodes } from "@/layout/lib/layoutNode";
import { DropDirection, FlexDirection, LayoutTreeActionType, type LayoutNode } from "@/layout/lib/types";
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
        for (const [width, height] of [[174, 162], [291, 347], [320, 200], [387, 347], [900, 600], [120, 500], [500, 90], [100, 100], [1200, 800], [3800, 2000], [700, 1500]]) {
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

    it("sizes a spot by the pane it is over: larger on a larger pane, never past the ceiling or under the floor, with the gap and the slop growing with it", () => {
        const widthIn = (width: number, height: number) => spotsIn(width, height).spots.find((s) => s.direction === DropDirection.Center)!.width;
        const [small, medium, large] = [widthIn(300, 200), widthIn(600, 400), widthIn(1200, 800)];
        expect(small).toBeLessThan(medium);
        expect(medium).toBeLessThan(large);
        // Pane to pane, the spot is the same share of it, until the ceiling.
        expect(Math.abs(medium / 600 - large / 1200)).toBeLessThan(0.01);
        expect(widthIn(4000, 2400)).toBe(180);
        expect(widthIn(60, 60)).toBe(28);
        // A small pane keeps the size every pane had before, as far as it fits: never smaller than it was.
        // (300 wide: five side by side fit at 53, what they were before; 340 wide has room for the full 56.)
        expect(widthIn(300, 200)).toBe(53);
        expect(widthIn(340, 300)).toBe(56);
        // The gap between two spots grows with the spot, and so does how far past its edge one takes the pointer.
        const gapIn = (width: number, height: number) => {
            const { spots } = spotsIn(width, height);
            const [centre, right] = [DropDirection.Center, DropDirection.Right].map((d) => spots.find((s) => s.direction === d)!);
            return right.x - centre.x - centre.width;
        };
        expect(gapIn(300, 200)).toBe(5);
        expect(gapIn(1200, 800)).toBeGreaterThan(gapIn(300, 200));
        const big = spotsIn(1200, 800).spots.find((s) => s.direction === DropDirection.Center)!;
        const edge = big.x + big.width / 2;
        expect(spotAt([big], { x: edge + 8, y: big.y })).toBe(big);
        const tiny = spotsIn(100, 100).spots.find((s) => s.direction === DropDirection.Center)!;
        expect(spotAt([tiny], { x: tiny.x + tiny.width / 2 + 8, y: tiny.y })).toBeUndefined();
    });

    it("shrinks the spots for one beside the compass, and leaves that one out only where five of the smallest do not fit", () => {
        const directions = (width: number, height: number) => spotsIn(width, height).spots.map((s) => s.direction);
        // Room at the edges: full size, the outer spots at the pane's edges.
        const roomy = spotsIn(900, 600).spots;
        expect(roomy).toHaveLength(ALL_AROUND.length);
        // 15% of the pane's width (the layout is twice as wide as high, so its height allows more).
        expect(roomy[0].width).toBe(135);
        // The edge spot stands in from the pane's edge by a share of the spot (28 px here), not a fixed 12.
        expect(roomy.find((s) => s.direction === DropDirection.OuterRight)!.x).toBeCloseTo(400 + 900 - 28 - 135 / 2, 5);
        // 200 wide: the edges are not clear of the compass, so five across, the outer spot next to the compass.
        const narrow = spotsIn(200, 600).spots;
        expect(narrow).toHaveLength(ALL_AROUND.length);
        expect(narrow[0].width).toBeLessThanOrEqual(56);
        expect(narrow[0].width).toBeGreaterThanOrEqual(28);
        const outer = narrow.find((s) => s.direction === DropDirection.OuterRight)!;
        expect(outer.x - narrow[0].x).toBe(2 * (narrow[0].width + 5));
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

describe("the wide guides: the dragged pane as a band across the whole layout", () => {
    const sized = (tree: LayoutNode, width = 1200, height = 800): LayoutModel => {
        const m = modelOf(tree);
        m.getBoundingRect = () => ({ top: 0, left: 0, width, height });
        m.displayContainerRef.current = { getBoundingClientRect: () => ({ top: 0, left: 0, width, height }) } as any;
        m.updateTree();
        return m;
    };
    const subject = (targetId: string, draggedId: string): DropSubject => ({ targetId, draggedId });
    const nowOf = (m: LayoutModel) => m.leafs().map((l) => ({ id: l.id, rect: m.getNodeRectById(l.id) }));
    const WIDE = [DropDirection.WideTop, DropDirection.WideCenter, DropDirection.WideBottom];

    it("offers a band across the top, the middle and the bottom, each one a layout of its own that fills the layout's width", () => {
        resetDropOptions();
        const m = modelOf(TREES["three in a row"]());
        const options = wideOptionsOf(m, subject("a", "b"));
        expect(options.map((o) => o.direction)).toEqual(WIDE);
        const band = (o: DropOption) => o.future!.rects[o.future!.movedId];
        const [top, middle, bottom] = options.map(band);
        for (const r of [top, middle, bottom]) expect([Math.round(r.left), Math.round(r.width)]).toEqual([0, 1200]);
        expect(Math.round(top.top)).toBe(0);
        expect(Math.round(bottom.top + bottom.height)).toBe(800);
        expect(middle.top).toBeGreaterThan(top.top);
        expect(middle.top + middle.height).toBeLessThan(bottom.top + bottom.height);
        // Everything else keeps its arrangement in the space that is left: the other two stay side by side.
        const others = options[0].future!.leafIds.filter((id) => id !== options[0].future!.movedId).map((id) => options[0].future!.rects[id]);
        expect(others.map((r) => Math.round(r.top))).toEqual([Math.round(top.top + top.height), Math.round(top.top + top.height)]);
    });

    it("is the action the release commits, and the layout it leaves is the one in the spot", () => {
        resetDropOptions();
        const m = modelOf(TREES["three in a row"]());
        const options = wideOptionsOf(m, subject("a", "b"));
        for (const option of options) {
            const action = dropActionOf(m, subject("a", "b"), option.direction)!;
            expect(action.type).toBe(LayoutTreeActionType.MoveToEdge);
            expect(seen(futureLayoutOf(m, action)!)).toBe(seen(option.future!));
        }
    });

    it("offers nothing that leaves the layout as it is: a pane that already is the top row, and a middle with no other pane to part from", () => {
        resetDropOptions();
        const stack = modelOf(TREES["two panes stacked"]());
        // `a` is the top row already (no band across the top, and no middle between nothing), `b` the bottom one.
        expect(wideOptionsOf(stack, subject("b", "a")).map((o) => o.direction)).toEqual([DropDirection.WideBottom]);
        expect(wideOptionsOf(stack, subject("a", "b")).map((o) => o.direction)).toEqual([DropDirection.WideTop]);
        resetDropOptions();
        const pair = modelOf(TREES["two panes side by side"]());
        expect(wideOptionsOf(pair, subject("b", "a")).map((o) => o.direction)).toEqual([DropDirection.WideTop, DropDirection.WideBottom]);
    });

    it("is for a pane of this tree: a tab pill or a pane from another tab lands beside a pane", () => {
        resetDropOptions();
        const m = modelOf(TREES["three in a row"]());
        const foreign = leaf("other");
        expect(wideOptionsOf(m, { targetId: "a", draggedId: "other", foreignNode: foreign })).toEqual([]);
        expect(wideOptionsOf(m, { targetId: "a", draggedId: "pill:x", foreignNode: pillStandIn("x"), pill: true })).toEqual([]);
    });

    it("shows at the top centre and the bottom centre of the layout, and nowhere else", () => {
        const m = sized(TREES["three in a row"]());
        expect(wideZoneAt(m, { x: 600, y: 20 })).toBe("top");
        expect(wideZoneAt(m, { x: 600, y: 780 })).toBe("bottom");
        expect(wideZoneAt(m, { x: 600, y: 400 })).toBeUndefined();
        expect(wideZoneAt(m, { x: 80, y: 20 })).toBeUndefined();
        expect(wideZoneAt(m, { x: 1150, y: 780 })).toBeUndefined();
    });

    it("stands the three spots side by side in the middle of the layout, inside it, none over another, at the edge of its zone", () => {
        const m = sized(TREES["three in a row"]());
        resetDropOptions();
        const options = wideOptionsOf(m, subject("a", "b"));
        for (const zone of ["top", "bottom"] as const) {
            const spots = wideGuideSpots(zone, "a", options, nowOf(m));
            expect(spots.map((s) => s.direction)).toEqual(WIDE);
            expect(spots[1].x).toBeCloseTo(600, 5);
            for (const s of spots) {
                expect(s.x - s.width / 2).toBeGreaterThanOrEqual(0);
                expect(s.x + s.width / 2).toBeLessThanOrEqual(1200);
                expect(s.y - s.height / 2).toBeGreaterThanOrEqual(0);
                expect(s.y + s.height / 2).toBeLessThanOrEqual(800);
            }
            expect(spots[1].x - spots[0].x).toBeGreaterThan(spots[0].width);
            const near = zone === "top" ? spots[0].y < 100 : spots[0].y > 700;
            expect(near).toBe(true);
            // The pointer is on a spot where the spot is, and the zone covers the spots.
            expect(spotAt(spots, { x: spots[2].x, y: spots[2].y })?.direction).toBe(DropDirection.WideBottom);
            expect(wideZoneAt(m, { x: spots[0].x, y: spots[0].y })).toBe(zone);
        }
    });

    describe("beside the hovered pane's own guide", () => {
        /** The pane's guide over `target` and the wide set at the top, and whether any spot of one is over a spot of the other. */
        const ofPane = (tree: LayoutNode, target: string, dragged = "z") => {
            const m = sized(tree);
            resetDropOptions();
            const now = nowOf(m);
            const rect = m.getNodeRectById(target);
            const own = guideSpots(rect, target, dropOptionsOf(m, subject(target, dragged)), now);
            const wide = wideGuideSpots("top", target, wideOptionsOf(m, subject(target, dragged)), now);
            const apart = (a: GuideSpot, b: GuideSpot) =>
                Math.abs(a.x - b.x) >= (a.width + b.width) / 2 + 1 || Math.abs(a.y - b.y) >= (a.height + b.height) / 2 + 1;
            const clear = own.every((s) => wide.every((w) => apart(s, w)));
            return { m, rect, own, wide, clear, now };
        };
        const SIDES = [DropDirection.Top, DropDirection.Right, DropDirection.Bottom, DropDirection.Left, DropDirection.Center];
        const compassOf = (spots: GuideSpot[]) => spots.filter((s) => SIDES.includes(s.direction));
        const stacked = () => col("r", 10, leaf("a", 12), leaf("b", 8), leaf("z"));
        const shortTop = () => col("r", 10, leaf("a", 1), leaf("b", 19), leaf("z"));

        it("leaves a pane's guide where it is when the wide set is not in its way", () => {
            const { own, wide, rect, clear } = ofPane(TREES["three in a row"](), "b", "a");
            expect(wide.length).toBe(3);
            expect(clear).toBe(true);
            // The spot beside it is the middle spot's place, which `a` leaves empty (it already stands left of `b`).
            const top = own.find((s) => s.direction === DropDirection.Top)!;
            expect(top.x).toBe(rect.left + rect.width / 2);
        });

        it("moves it down, clear of the wide set, for a pane at the top that it would have been under", () => {
            const { clear, own, rect } = ofPane(stacked(), "a");
            expect(clear).toBe(true);
            // Down from where the middle of the pane puts it, and still in the pane.
            const centre = compassOf(own).find((s) => s.direction === DropDirection.Center)!;
            expect(centre.y).toBeGreaterThan(rect.top + rect.height / 2);
            for (const s of compassOf(own)) expect(s.y + s.height / 2).toBeLessThanOrEqual(rect.top + rect.height);
            // Every spot of the pane's guide is still where a pointer takes it.
            for (const s of own) expect(spotAt(own, { x: s.x, y: s.y })?.direction).toBe(s.direction);
        });

        it("holds it beside the wide set for a pane too short to hold it below", () => {
            const { clear, own, wide, rect } = ofPane(shortTop(), "a");
            expect(clear).toBe(true);
            const centre = compassOf(own).find((s) => s.direction === DropDirection.Center)!;
            // Level with where it was, to the side of the three wide spots, in the pane's width.
            expect(centre.y).toBeCloseTo(rect.top + rect.height / 2, 5);
            expect(centre.x + centre.width / 2).toBeLessThan(wide[0].x - wide[0].width / 2);
            expect(centre.x - centre.width / 2).toBeGreaterThanOrEqual(rect.left);
        });

        it("does the same at the bottom of the layout, mirrored", () => {
            const m = sized(col("r", 10, leaf("z"), leaf("b", 8), leaf("a", 12)));
            resetDropOptions();
            const now = nowOf(m);
            const rect = m.getNodeRectById("a");
            const own = guideSpots(rect, "a", dropOptionsOf(m, subject("a", "z")), now);
            const wide = wideGuideSpots("bottom", "a", wideOptionsOf(m, subject("a", "z")), now);
            const centre = own.find((s) => s.direction === DropDirection.Center)!;
            expect(centre.y).toBeLessThan(rect.top + rect.height / 2);
            expect(own.every((s) => wide.every((w) => Math.abs(s.x - w.x) >= (s.width + w.width) / 2 + 1 || Math.abs(s.y - w.y) >= (s.height + w.height) / 2 + 1))).toBe(true);
        });

        it("drops only the spots that are under the wide set, for a pane too small for any other place", () => {
            const m = sized(col("r", 10, row("r1", 1, leaf("a", 4), leaf("b", 2), leaf("c", 4)), leaf("d", 19)));
            resetDropOptions();
            const now = nowOf(m);
            const own = guideSpots(m.getNodeRectById("b"), "b", dropOptionsOf(m, subject("b", "d")), now);
            const wide = wideGuideSpots("top", "b", wideOptionsOf(m, subject("b", "d")), now);
            const mixed = withWideSpots(own, wide);
            expect(mixed.slice(0, wide.length)).toEqual(wide);
            expect(mixed.length).toBeLessThan(wide.length + own.length);
            // None of the pane's that stay is over a wide one, and the same arrays give the same one back.
            for (const s of mixed.slice(wide.length)) expect(own).toContain(s);
            expect(withWideSpots(own, wide)).toBe(mixed);
        });
    });

    it("sizes the spots by the layout, within the floor and the ceiling, and leaves a layout too small for them alone", () => {
        resetDropOptions();
        const widthIn = (width: number, height: number): number | undefined => {
            const m = sized(TREES["three in a row"](), width, height);
            resetDropOptions();
            return wideGuideSpots("top", "a", wideOptionsOf(m, subject("a", "b")), nowOf(m))[0]?.width;
        };
        const [small, medium, large] = [widthIn(900, 600), widthIn(1500, 1000), widthIn(2100, 1400)];
        expect(small!).toBeLessThan(medium!);
        expect(medium!).toBeLessThan(large!);
        expect(widthIn(4000, 2400)).toBe(180);
        // A layout the three spots and their zones would cover most of has none.
        expect(widthIn(200, 120)).toBeUndefined();
        expect(wideZoneAt(sized(TREES["three in a row"](), 200, 120), { x: 100, y: 5 })).toBeUndefined();
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
