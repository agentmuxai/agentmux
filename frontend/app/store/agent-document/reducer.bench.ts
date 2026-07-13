// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Benchmarks for the agent-document reducer's `StreamFlush` — task #40,
 * the empirical follow-up to the task #39 fix.
 *
 * Manual-only: NOT wired into `package.json`/CI. Run on demand with:
 *
 *   npx vitest bench frontend/app/store/agent-document/reducer.bench.ts
 *
 * Two scenarios, because `StreamFlush` has two different cost profiles
 * and the task #39 fix only removed ONE O(n) pass from each — see the
 * "what this does and doesn't prove" note below before reading numbers.
 *
 *  - "append" — `newNodes` contains a genuinely new id (the first chunk
 *    of a new markdown/tool block). Before the fix: the reducer rebuilt
 *    an id->index `Map` from scratch by scanning ALL of `state.nodes`
 *    (`for (let i=0;i<state.nodes.length;i++) indexById.set(...)`) on
 *    EVERY flush, unconditionally — one O(n) pass. After the fix: it
 *    reuses `state.nodeIndexById` and only clones it
 *    (`new Map(state.nodeIndexById)`) when an append actually happens —
 *    still O(n) for the clone itself (a `Map` has no cheaper persistent
 *    "add one entry" operation than a plain array does), so this
 *    scenario is NOT expected to go flat. What it proves is the clone
 *    (one pass, done by `Map`'s own copy constructor) is cheaper than
 *    the old scan-and-insert loop (also one pass, but each iteration
 *    pays for both a property read AND a hash-insert).
 *
 *  - "update-only" — `updatedNodes` targets an id that's ALREADY in
 *    `state.nodes` (the dominant real-world case: every token AFTER the
 *    first one in a streaming markdown block is a content-only update
 *    to the SAME node, not a new node). Before the fix: same
 *    unconditional O(n) map rebuild as above, PLUS the node-array copy.
 *    After the fix: `nodeIndexById` is reused as-is (zero cost, not
 *    even a clone) — the map rebuild is gone entirely for this case.
 *
 * What this does NOT prove: that `StreamFlush` overall is O(1)/O(k).
 * It isn't, in EITHER scenario. `agent-document-store.ts` exposes
 * `nodes` as a plain-array Solid *signal* (not a `createStore`), so a
 * brand-new `DocumentNode[]` reference is structurally required on
 * every change for the signal to fire — `ensureClone()`'s
 * `state.nodes.slice()` is an O(n) full-array copy on every
 * nodes-changing flush, append or update, before AND after this fix.
 * That's the residual documented in PR #2127 / task #39: removing it
 * too would mean switching to a store/`produce`-based document model,
 * out of that task's scope. Expect BOTH scenarios below to still show
 * cost growing with document size — the fix's win is a reduced constant
 * factor (no more a SECOND O(n) pass stacked on top of the array copy),
 * most pronounced in "update-only" where that second pass is now zero
 * instead of one-map-insert-per-existing-node.
 */

import { bench, describe } from "vitest";
import type { DocumentNode } from "../../view/agent/types";
import { update } from "./reducer";
import { initialState } from "./types";

const DOC_SIZES = [100, 1_000, 10_000] as const;

const md = (id: string, content = id): DocumentNode => ({
    type: "markdown",
    id,
    content,
    timestamp: 0,
});

/** Build a base state with `size` existing markdown nodes, via the
 *  reducer itself (one real `StreamFlush`, not a hand-rolled object) so
 *  `nodeIndexById` is populated exactly the way production traffic
 *  populates it. This setup runs ONCE per size, outside the timed
 *  `bench()` loop below. */
function seedState(size: number) {
    const nodes = Array.from({ length: size }, (_, i) => md(`existing-${i}`));
    return update(initialState(), { type: "StreamFlush", newNodes: nodes, updatedNodes: [] })
        .state;
}

describe("agent-document reducer update(): StreamFlush append (new node)", () => {
    for (const size of DOC_SIZES) {
        // `update()` is pure — `base` is never mutated or reassigned, so
        // every iteration appends onto the SAME fixed-size document
        // instead of letting the document grow across the bench run.
        const base = seedState(size);
        let counter = 0;
        bench(`append 1 new node onto a ${size.toLocaleString()}-node document`, () => {
            counter++;
            update(base, {
                type: "StreamFlush",
                newNodes: [md(`new-${size}-${counter}`)],
                updatedNodes: [],
            });
        });
    }
});

describe("agent-document reducer update(): StreamFlush update-only (dominant streaming case)", () => {
    for (const size of DOC_SIZES) {
        const base = seedState(size);
        // Target the LAST existing node each time — same node every
        // call, mirroring "more tokens land on the in-progress tail
        // block." A fresh `content` string per call avoids accidentally
        // measuring any content-equality fast path.
        const targetId = `existing-${size - 1}`;
        let counter = 0;
        bench(`update 1 existing node in a ${size.toLocaleString()}-node document`, () => {
            counter++;
            update(base, {
                type: "StreamFlush",
                newNodes: [],
                updatedNodes: [md(targetId, `updated content ${counter}`)],
            });
        });
    }
});
