// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-new-spec-status.mjs's pure parts — no git, no
// filesystem. The cases below are the real ones this gate was measured
// against (see the WHY comment in the script).

import { describe, expect, it } from "vitest";
import { docName, isSourcePath, isViolation, statusOf } from "./check-new-spec-status.mjs";

describe("statusOf", () => {
    it("takes the first word of the first Status line, lowercased", () => {
        expect(statusOf("# T\n\n**Status:** Proposed\n")).toBe("proposed");
        expect(statusOf("**Status:** draft — design\n")).toBe("draft");
        expect(statusOf("**Status:** implemented — #2410, #2414\n")).toBe("implemented");
    });

    it("strips the punctuation specs actually use as separators", () => {
        // Em dash, en dash, hyphen, comma and colon all appear in real headers.
        expect(statusOf("**Status:** active—Phase 0 shipped\n")).toBe("active");
        expect(statusOf("**Status:** Draft, ready to implement\n")).toBe("draft");
        expect(statusOf("**Status:** proposed. **Note:** ...\n")).toBe("proposed");
    });

    it("ignores a parenthetical that contradicts the first word", () => {
        // SPEC_INAPP_CLAUDE_OAUTH_LOGIN really said this for six weeks. The
        // first word is what the tooling buckets on, so it is what we read.
        expect(statusOf("**Status:** PROPOSED (implemented — see note below)\n")).toBe("proposed");
    });

    it("is null when there is no Status line", () => {
        expect(statusOf("# T\n\nStatus: proposed\n")).toBeNull();
    });
});

describe("docName", () => {
    it("is the basename without .md — what a source comment cites", () => {
        expect(docName("docs/specs/SPEC_FOO_2026_01_01.md")).toBe("SPEC_FOO_2026_01_01");
        expect(docName("docs/reports/REPORT_BAR.md")).toBe("REPORT_BAR");
    });
});

describe("isSourcePath", () => {
    it("accepts the languages that carry spec citations", () => {
        expect(isSourcePath("agentmux-srv/src/backend/rpc/engine.rs")).toBe(true);
        expect(isSourcePath("frontend/app/view/agent/agent-view.tsx")).toBe(true);
        expect(isSourcePath("scripts/check-doc-links.mjs")).toBe(true);
    });

    it("rejects docs and non-source files", () => {
        // A spec citing its own name must never count as source citing it.
        expect(isSourcePath("docs/specs/SPEC_FOO.md")).toBe(false);
        expect(isSourcePath("README.md")).toBe(false);
        expect(isSourcePath("Cargo.toml")).toBe(false);
    });
});

describe("isViolation", () => {
    it("fails a proposal that its own PR's source implements", () => {
        // The #3090 / #2932 / #1011 / #1301 shape: 12-14 source files cite it.
        expect(isViolation({ status: "proposed", citingSources: ["a.tsx", "b.ts"] })).toBe(true);
        expect(isViolation({ status: "draft", citingSources: ["a.rs"] })).toBe(true);
    });

    it("passes a proposal nothing in the PR implements", () => {
        // The #553 shape: SPEC_DECISION_PROMPT rode in on an unrelated
        // statusbar PR. `Draft` was honest, and failing it would have taught
        // people to route around this gate.
        expect(isViolation({ status: "proposed", citingSources: [] })).toBe(false);
        expect(isViolation({ status: "draft", citingSources: [] })).toBe(false);
    });

    it("passes a doc that already admits it is built", () => {
        expect(isViolation({ status: "implemented", citingSources: ["a.rs"] })).toBe(false);
        expect(isViolation({ status: "active", citingSources: ["a.rs"] })).toBe(false);
        expect(isViolation({ status: "living", citingSources: ["a.rs"] })).toBe(false);
        expect(isViolation({ status: "analysis", citingSources: ["a.rs"] })).toBe(false);
    });

    it("leaves a missing Status line to the gate that owns it", () => {
        // check-doc-status.sh / check-docs-lifecycle.mjs cover that; firing
        // here too would just double-report the same file.
        expect(isViolation({ status: null, citingSources: ["a.rs"] })).toBe(false);
    });
});
