// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { injectHistoryLink } from "./inject-history-link";
import type { DocumentNode } from "./types";

const md = (id: string): DocumentNode => ({ type: "markdown", id, content: id });

const freshOutcome = (id = "boundary"): DocumentNode => ({
    type: "session_outcome",
    id,
    outcome: "fresh",
    attemptedSid: "sid-1",
    actualSid: null,
    timestamp: 0,
});

const resumedOutcome = (id = "boundary"): DocumentNode => ({
    type: "session_outcome",
    id,
    outcome: "resumed",
    attemptedSid: "sid-1",
    actualSid: "sid-1",
    timestamp: 0,
});

describe("injectHistoryLink (SPEC_AGENT_HISTORY_AS_TAB_AND_DRAFT_PRESERVATION_2026_08_11.md §3.2)", () => {
    it("is a no-op when show is false, even with a fresh boundary present", () => {
        const nodes = [freshOutcome(), md("a")];
        expect(injectHistoryLink(nodes, false)).toEqual(nodes);
    });

    it("inserts the link row above a leading fresh divider: what it points to is older (2026-10-01)", () => {
        const out = injectHistoryLink([freshOutcome(), md("a"), md("b")], true);
        expect(out.map((n) => n.type)).toEqual(["history_link", "session_outcome", "markdown", "markdown"]);
        expect(out[0].id).toBe("history-link");
    });

    it("inserts at the very front when the first node is not a divider", () => {
        const out = injectHistoryLink([md("a"), md("b")], true);
        expect(out.map((n) => n.type)).toEqual(["history_link", "markdown", "markdown"]);
    });

    it("inserts at the front when the first node is a resumed outcome", () => {
        const out = injectHistoryLink([resumedOutcome(), md("a")], true);
        expect(out.map((n) => n.type)).toEqual(["history_link", "session_outcome", "markdown"]);
    });

    it("falls back to the front on an empty node list without throwing", () => {
        expect(injectHistoryLink([], true)).toEqual([{ type: "history_link", id: "history-link" }]);
    });

    it("never mutates the input array", () => {
        const nodes = [freshOutcome(), md("a")];
        const snapshot = [...nodes];
        injectHistoryLink(nodes, true);
        expect(nodes).toEqual(snapshot);
    });
});
