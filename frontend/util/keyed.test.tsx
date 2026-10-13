// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// docs/specs/SPEC_KEYED_SLOT_RETENTION_FIX_2026_10_12.md §3.

import { render } from "@solidjs/testing-library";
import v8 from "node:v8";
import { createMemo, createRoot, createSignal, onCleanup } from "solid-js";
import { describe, expect, it } from "vitest";
import { Key, keyArray } from "./keyed";

/** One per mapped item, so live ones can be counted after a full GC. */
class Probe {
    payload = new Array(1000).fill(0);
    constructor(readonly name: string) {}
}

/** Live Probe instances; `v8.queryObjects` runs a full GC first. */
function liveProbes(): number {
    return v8.queryObjects(Probe, { format: "count" });
}

describe("keyArray: a slot does not keep the rows of the pass it was born in", () => {
    it("items that left the list are collectable while a slot born with them lives", () => {
        const [list, setList] = createSignal<string[]>([]);
        let dispose!: () => void;
        let read!: () => Probe[];
        createRoot((d) => {
            dispose = d;
            const mapped = createMemo(
                keyArray(
                    list,
                    (k) => k,
                    (item) => new Probe(item())
                )
            );
            read = () => mapped();
        });
        // Pass 1 creates a, b, c together; pass 2 keeps only c and adds d.
        // c's slot was born in pass 1, whose result array held a and b.
        setList(["a", "b", "c"]);
        expect(read().map((p) => p.name)).toEqual(["a", "b", "c"]);
        setList(["c", "d"]);
        expect(read().map((p) => p.name)).toEqual(["c", "d"]);

        expect(liveProbes()).toBe(2);
        dispose();
    });

    it("the same holds through many passes", () => {
        const [list, setList] = createSignal<number[]>([]);
        let dispose!: () => void;
        let read!: () => Probe[];
        createRoot((d) => {
            dispose = d;
            const mapped = createMemo(
                keyArray(
                    list,
                    (k) => k,
                    (item) => new Probe(String(item()))
                )
            );
            read = () => mapped();
        });
        // A window of 10 sliding by one each pass, as the agent document's
        // rows do while a turn streams.
        for (let start = 0; start < 200; start++) {
            setList(Array.from({ length: 10 }, (_, i) => start + i));
            read();
        }
        expect(liveProbes()).toBe(10);
        dispose();
    });
});

describe("keyArray: behaviour", () => {
    it("keeps an item's mapped output while its key stays, and disposes removed keys", () => {
        const [list, setList] = createSignal([
            { id: 1, v: "a" },
            { id: 2, v: "b" },
        ]);
        const cleaned: number[] = [];
        let made = 0;
        createRoot((dispose) => {
            const mapped = createMemo(
                keyArray(
                    list,
                    (x) => x.id,
                    (item) => {
                        made++;
                        const id = item().id;
                        onCleanup(() => cleaned.push(id));
                        return { id, value: () => item().v };
                    }
                )
            );
            const first = mapped();
            setList([
                { id: 2, v: "B" },
                { id: 3, v: "c" },
            ]);
            const second = mapped();
            expect(second[0]).toBe(first[1]); // same key → same output
            expect(second[0].value()).toBe("B"); // with the new item
            expect(made).toBe(3);
            expect(cleaned).toEqual([1]);
            dispose();
        });
        expect(cleaned.sort()).toEqual([1, 2, 3]);
    });

    it("updates the index signal when items move", () => {
        const [list, setList] = createSignal(["x", "y", "z"]);
        createRoot((dispose) => {
            const mapped = createMemo(
                keyArray(
                    list,
                    (k) => k,
                    (item, index) => ({ item, index })
                )
            );
            const z = mapped()[2];
            expect(z.index()).toBe(2);
            setList(["z", "x", "y"]);
            expect(mapped()[0]).toBe(z);
            expect(z.index()).toBe(0);
            dispose();
        });
    });

    it("shows the fallback for an empty list, and drops it when items arrive", () => {
        const [list, setList] = createSignal<string[]>([]);
        createRoot((dispose) => {
            const mapped = createMemo(
                keyArray(
                    list,
                    (k) => k,
                    (item) => `item:${item()}`,
                    { fallback: () => "empty" }
                )
            );
            expect(mapped()).toEqual(["empty"]);
            setList(["a"]);
            expect(mapped()).toEqual(["item:a"]);
            setList([]);
            expect(mapped()).toEqual(["empty"]);
            dispose();
        });
    });
});

describe("<Key>", () => {
    it("renders keyed children, keeps their DOM across moves, and supports `by` as a property name", () => {
        const [list, setList] = createSignal([{ id: "a" }, { id: "b" }]);
        const { container } = render(() => (
            <div>
                <Key each={list()} by="id">
                    {(item) => <span data-id={item().id}>{item().id}</span>}
                </Key>
            </div>
        ));
        const b = container.querySelector('[data-id="b"]');
        setList([{ id: "b" }, { id: "c" }]);
        expect([...container.querySelectorAll("span")].map((s) => s.textContent)).toEqual(["b", "c"]);
        expect(container.querySelector('[data-id="b"]')).toBe(b);
    });

    it("renders the fallback when the list is empty", () => {
        const { container } = render(() => (
            <div>
                <Key each={[] as string[]} by={(v) => v} fallback={<em>none</em>}>
                    {(item) => <span>{item()}</span>}
                </Key>
            </div>
        ));
        expect(container.querySelector("em")?.textContent).toBe("none");
    });
});
