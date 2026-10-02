// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { assert, describe, test } from "vitest";
import { readActivitySummary, readSwarmSummary } from "./activitySummary";

describe("readActivitySummary", () => {
    test("prefers term:ambient_summary over term:osc_title when both present", () => {
        assert.equal(
            readActivitySummary({ "term:ambient_summary": "fixing auth bug", "term:osc_title": "claude - auth" }),
            "fixing auth bug",
        );
    });

    test("falls back to term:osc_title when term:ambient_summary is absent", () => {
        assert.equal(readActivitySummary({ "term:osc_title": "claude - auth refactor" }), "claude - auth refactor");
    });

    test("falls back to term:osc_title when term:ambient_summary is empty", () => {
        assert.equal(
            readActivitySummary({ "term:ambient_summary": "", "term:osc_title": "claude - auth refactor" }),
            "claude - auth refactor",
        );
    });

    test("returns undefined when neither key is present", () => {
        assert.equal(readActivitySummary({}), undefined);
        assert.equal(readActivitySummary(undefined), undefined);
    });

    test("returns undefined when both keys are empty strings", () => {
        assert.equal(readActivitySummary({ "term:ambient_summary": "", "term:osc_title": "" }), undefined);
    });

    // The 2026-10-02 failure: the swarm row read "(none yet)" because that is what was
    // stored. A stored value that is not a title is not shown, whatever is in the meta.
    test("does not show a stored placeholder, and falls through to the OSC title", () => {
        assert.equal(readActivitySummary({ "term:ambient_summary": "(none yet)" }), undefined);
        assert.equal(readActivitySummary({ "term:ambient_summary": "no goal established yet" }), undefined);
        assert.equal(
            readActivitySummary({ "term:ambient_summary": "(none yet)", "term:osc_title": "claude - auth" }),
            "claude - auth",
        );
    });

    test("a real title is shown unchanged", () => {
        assert.equal(
            readActivitySummary({ "term:ambient_summary": "Develop hardening spec for swarm ambient summary quality" }),
            "Develop hardening spec for swarm ambient summary quality",
        );
    });
});

describe("readSwarmSummary", () => {
    test("is what the swarm row and the pane tab tooltip share: trimmed, or null", () => {
        assert.equal(readSwarmSummary({ "term:ambient_summary": "  Fix login  " }), "Fix login");
        assert.equal(readSwarmSummary({ "term:ambient_summary": "(none yet)" }), null);
        assert.equal(readSwarmSummary({}), null);
        assert.equal(readSwarmSummary(undefined), null);
    });
});
