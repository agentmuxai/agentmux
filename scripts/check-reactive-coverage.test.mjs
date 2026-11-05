// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-reactive-coverage.mjs.

import { describe, expect, it } from "vitest";
import { compare, formatBaseline, isStale, isViewPath, lowered, parseBaseline } from "./check-reactive-coverage.mjs";

const FETCHES = `onMount(async () => { await RpcApi.ListThingsCommand(TabRpcClient); });`;

describe("isViewPath", () => {
    it("takes view source, not tests or other folders", () => {
        expect(isViewPath("frontend/app/view/settings/sections/hidden-agents.tsx")).toBe(true);
        expect(isViewPath("frontend/app/view/mcp/mcp-observed.ts")).toBe(true);
        expect(isViewPath("frontend/app/view/settings/sections/hidden-agents.test.tsx")).toBe(false);
        expect(isViewPath("frontend/app/store/domain-store.ts")).toBe(false);
        expect(isViewPath("frontend/app/view/readme.md")).toBe(false);
    });
});

describe("isStale", () => {
    it("flags a fetch on mount with nothing live", () => {
        expect(isStale(FETCHES)).toBe(true);
        expect(isStale(`const [x] = createResource(() => RpcApi.GetXCommand(TabRpcClient));`)).toBe(true);
    });

    it("ignores a file that does not fetch over RPC on mount", () => {
        expect(isStale(`onMount(() => focus());`)).toBe(false);
        expect(isStale(`function load() { return RpcApi.GetXCommand(TabRpcClient); }`)).toBe(false);
    });

    it("accepts each kind of live source", () => {
        for (const live of [
            `onMuxEvent(WpsEvent.ThingsChanged, reload);`,
            `pollWhileVisible(reload, 5000);`,
            `setInterval(reload, 5000);`,
            `store.subscribe(reload);`,
            `const things = createDomainStore({});`,
            `thingsStore.follow();`,
        ]) {
            expect(isStale(`${FETCHES}\n${live}`), live).toBe(false);
        }
    });

    it("accepts a one-shot marker that gives a reason, and not one that gives none", () => {
        expect(isStale(`// reactive-coverage: one-shot options load when the form opens\n${FETCHES}`)).toBe(false);
        expect(isStale(`// reactive-coverage: one-shot\n${FETCHES}`)).toBe(true);
    });
});

describe("baseline", () => {
    it("round-trips through format and parse, sorted", () => {
        const text = formatBaseline(new Set(["b.tsx", "a.tsx"]));
        expect([...parseBaseline(text)]).toEqual(["a.tsx", "b.tsx"]);
        expect(text.indexOf("a.tsx")).toBeLessThan(text.indexOf("b.tsx"));
    });

    it("ignores comments and blank lines", () => {
        expect([...parseBaseline("# note\n\nx.tsx\n  y.tsx  \n")]).toEqual(["x.tsx", "y.tsx"]);
    });
});

describe("compare and lowered", () => {
    it("reports a new stale file and a fixed one", () => {
        const stale = new Set(["a.tsx", "new.tsx"]);
        const baseline = new Set(["a.tsx", "fixed.tsx"]);
        expect(compare(stale, baseline)).toEqual({ added: ["new.tsx"], fixed: ["fixed.tsx"] });
    });

    it("is clean when they agree", () => {
        expect(compare(new Set(["a.tsx"]), new Set(["a.tsx"]))).toEqual({ added: [], fixed: [] });
    });

    it("--update drops the fixed and never adds", () => {
        const next = lowered(new Set(["a.tsx", "new.tsx"]), new Set(["a.tsx", "fixed.tsx"]));
        expect([...next]).toEqual(["a.tsx"]);
    });
});
