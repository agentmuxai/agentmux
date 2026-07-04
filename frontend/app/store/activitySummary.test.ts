// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { assert, describe, test } from "vitest";
import { readActivitySummary } from "./activitySummary";

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
});
