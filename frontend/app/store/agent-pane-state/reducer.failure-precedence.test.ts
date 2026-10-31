// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// Who wins when several writers raise or clear the pane's failure (failure-precedence.ts).

import type { AgentFailure } from "@/types/rpc/AgentFailure";
import { describe, expect, it } from "vitest";
import { FAILURE_RANK, outranks, type FailureSource } from "./failure-precedence";
import { update } from "./reducer";
import { mk, ready, streaming } from "./reducer-test-fixtures";
import type { AgentPaneState } from "./types";

const failure = (code: AgentFailure["code"], title = code): AgentFailure => ({ code, title, detail: "", retryable: true }) as AgentFailure;
const SOURCES = Object.keys(FAILURE_RANK) as FailureSource[];

const raise = (state: AgentPaneState, source: FailureSource | undefined, code: AgentFailure["code"], at = 1000) =>
    update(state, { type: "FailureObserved", failure: failure(code), at, ...(source ? { source } : {}) }).state;

describe("a failure only replaces one of its own rank or lower", () => {
    for (const current of SOURCES) {
        for (const incoming of SOURCES) {
            const replaces = FAILURE_RANK[incoming] >= FAILURE_RANK[current];
            it(`${incoming} over a ${current} failure: ${replaces ? "replaces" : "is dropped"}`, () => {
                const s = raise(mk(), current, "overloaded");
                const next = raise(s, incoming, "network");
                expect(next.failure?.data.code).toBe(replaces ? "network" : "overloaded");
                if (replaces) expect(next.failure?.source).toBe(incoming);
            });
        }
    }

    it("nothing is outranked when no failure shows", () => {
        for (const source of SOURCES) expect(outranks(null, source)).toBe(false);
    });

    it("a failure with no recorded source is the backend's, so it outranks everything below live", () => {
        const s = raise(mk(), undefined, "overloaded");
        expect(raise(s, "command", "network").failure?.data.code).toBe("overloaded");
        expect(raise(s, "live", "network").failure?.data.code).toBe("network");
    });

    it("a dropped raise changes nothing and emits no event", () => {
        const s = raise(mk(), "live", "overloaded");
        const r = update(s, { type: "FailureObserved", failure: failure("network"), at: 5, source: "seed" });
        expect(r.state).toBe(s);
        expect(r.events).toEqual([]);
    });
});

describe("a stored failure that arrives late", () => {
    it("is raised on an idle pane that has had no turn since it mounted", () => {
        const s = update(ready(), { type: "FailureObserved", failure: failure("auth"), at: 500, source: "seed", since: 400 }).state;
        expect(s.failure?.data.code).toBe("auth");
    });

    it("never ends a turn that is running", () => {
        const s = streaming();
        const r = update(s, { type: "FailureObserved", failure: failure("auth"), at: 500, source: "seed", since: 50 });
        expect(r.state).toBe(s);
        expect(r.state.turnPhase.kind).toBe("Streaming");
    });

    it("is dropped when a turn finished after the pane mounted", () => {
        let s = streaming(100);
        s = update(s, { type: "TurnEnd", stats: null }).state;
        expect(s.turnPhase.kind).toBe("Done");
        // The turn ended just now; the pane mounted a minute ago.
        const r = update(s, { type: "FailureObserved", failure: failure("auth"), at: 500, source: "seed", since: Date.now() - 60_000 });
        expect(r.state.failure).toBeNull();
    });

    it("is raised when the turn that finished was before the pane mounted", () => {
        let s = streaming(100);
        s = update(s, { type: "TurnEnd", stats: null }).state;
        // The pane mounts after that turn ended.
        const r = update(s, { type: "FailureObserved", failure: failure("auth"), at: 500, source: "seed", since: Date.now() + 60_000 });
        expect(r.state.failure?.data.code).toBe("auth");
    });

    it("a live failure still ends a running turn", () => {
        const r = update(streaming(), { type: "FailureObserved", failure: failure("overloaded"), at: 500, source: "live" });
        expect(r.state.turnPhase.kind).toBe("Done");
        expect(r.events).toContainEqual({ type: "failure-observed", code: "overloaded", turnWasEnded: true });
    });
});

describe("clearing", () => {
    it("clears whatever shows when no class is named, and says why", () => {
        const s = raise(mk(), "live", "overloaded");
        const r = update(s, { type: "FailureCleared", reason: "dismissed" });
        expect(r.state.failure).toBeNull();
        expect(r.events).toEqual([{ type: "failure-cleared", reason: "dismissed" }]);
    });

    it("clears only the named class: an auth proof leaves another failure alone", () => {
        const other = raise(mk(), "live", "overloaded");
        const kept = update(other, { type: "FailureCleared", reason: "recovered", onlyCode: "auth" });
        expect(kept.state).toBe(other);
        expect(kept.events).toEqual([]);

        const auth = raise(mk(), "live", "auth");
        expect(update(auth, { type: "FailureCleared", reason: "recovered", onlyCode: "auth" }).state.failure).toBeNull();
    });

    it("is a no-op with nothing showing", () => {
        const s = mk();
        expect(update(s, { type: "FailureCleared", reason: "dismissed" }).state).toBe(s);
    });
});
