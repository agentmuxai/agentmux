// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from "vitest";
import { buildResumePreflightNode, injectResumePreflight } from "./inject-resume-preflight";
import type { DocumentNode } from "./types";

const md = (id: string): DocumentNode => ({ type: "markdown", id, content: "hi" } as DocumentNode);

const outcome = (): DocumentNode =>
    ({
        type: "session_outcome",
        id: "so-1",
        outcome: "fresh",
        attemptedSid: "",
        actualSid: null,
        timestamp: 0,
    } as DocumentNode);

const result = (
    verdict: SessionResumePreflightResult["verdict"],
    extra: Partial<SessionResumePreflightResult> = {},
): SessionResumePreflightResult => ({
    block_id: "b",
    verdict,
    steps: [],
    duration_ms: 3,
    ...extra,
});

describe("buildResumePreflightNode", () => {
    it("warns when the next spawn will start a new conversation", () => {
        const node = buildResumePreflightNode([md("a")], result("fresh"), false);
        expect(node).toMatchObject({ type: "resume_preflight", verdict: "fresh", pending: false });
    });

    it("carries the recoverable session id through as evidence history exists", () => {
        const node = buildResumePreflightNode(
            [md("a")],
            result("fresh", { recoverable_session_id: "sid-orphaned" }),
            false,
        );
        expect(node?.recoverableSessionId).toBe("sid-orphaned");
    });

    it("reports a recover verdict distinctly — continuity survives, it just pauses", () => {
        const node = buildResumePreflightNode([md("a")], result("recover"), false);
        expect(node?.verdict).toBe("recover");
    });

    it("says nothing when the conversation will simply resume", () => {
        expect(buildResumePreflightNode([md("a")], result("resume"), false)).toBeNull();
    });

    it("says nothing when the verdict is unknowable", () => {
        expect(buildResumePreflightNode([md("a")], result("unknown"), false)).toBeNull();
    });

    // A brand-new agent has no "conversation below" to lose — warning there
    // would put a scary row on every first open.
    it("says nothing on an empty document, even for a fresh verdict", () => {
        expect(buildResumePreflightNode([], result("fresh"), false)).toBeNull();
    });

    // The prediction must never sit next to the retrospective fact: once a real
    // session_outcome exists the spawn has happened and reported for itself.
    it("stands down once a real session_outcome divider is present", () => {
        expect(buildResumePreflightNode([outcome(), md("a")], result("fresh"), false)).toBeNull();
    });

    it("shows a pending row while the check is still running", () => {
        const node = buildResumePreflightNode([md("a")], null, true);
        expect(node).toMatchObject({ pending: true });
        expect(node?.verdict).toBeUndefined();
    });

    it("shows nothing while the check is fast — pending false, no result yet", () => {
        expect(buildResumePreflightNode([md("a")], null, false)).toBeNull();
    });

    // Even mid-check, a document that already reported its own outcome must not
    // sprout a spinner about a question that's already answered.
    it("suppresses the pending row too when a session_outcome is present", () => {
        expect(buildResumePreflightNode([outcome()], null, true)).toBeNull();
    });
});

describe("injectResumePreflight", () => {
    it("prepends the notice ahead of everything it describes", () => {
        const nodes = [md("a"), md("b")];
        const node = buildResumePreflightNode(nodes, result("fresh"), false);
        const out = injectResumePreflight(nodes, node);
        expect(out.map((n) => n.type)).toEqual(["resume_preflight", "markdown", "markdown"]);
    });

    it("returns the document untouched when there's nothing to say", () => {
        const nodes = [md("a")];
        expect(injectResumePreflight(nodes, null)).toBe(nodes);
    });
});
