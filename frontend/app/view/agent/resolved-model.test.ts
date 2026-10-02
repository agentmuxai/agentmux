// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { lastReplyModel, modelFamily } from "./resolved-model";
import type { RuntimeAgreement } from "./process-runtime";

const agrees: RuntimeAgreement = { kind: "agrees" };
const drift = [{ axis: "model" as const, wanted: "sonnet", running: "opus" }];

describe("modelFamily", () => {
    it("reads aliases and concrete (also dated) ids", () => {
        expect(modelFamily("sonnet")).toBe("sonnet");
        expect(modelFamily("claude-opus-5-5")).toBe("opus");
        expect(modelFamily("claude-haiku-4-5-20251001")).toBe("haiku");
        expect(modelFamily("Claude-Sonnet-5")).toBe("sonnet");
    });
    it("makes no claim for anything else, or for an id naming two families", () => {
        for (const m of [undefined, null, "", "gpt-5", "default", "gemini-2.5-pro", "opus-or-sonnet"]) {
            expect(modelFamily(m), String(m)).toBeUndefined();
        }
    });
});

describe("lastReplyModel", () => {
    it("says nothing before there is a reply", () => {
        expect(lastReplyModel("sonnet", null, agrees)).toBeNull();
        expect(lastReplyModel("sonnet", undefined, agrees)).toBeNull();
    });

    it("reports the model without a flag when it matches the selection", () => {
        expect(lastReplyModel("sonnet", "claude-sonnet-5-5", agrees)).toEqual({
            text: "Last reply used claude-sonnet-5-5",
            differs: false,
        });
        // a dated id extends an undated one
        expect(lastReplyModel("claude-haiku-4-5", "claude-haiku-4-5-20251001", agrees)?.differs).toBe(false);
    });

    it("flags a different family when the process was spawned with the selection", () => {
        const r = lastReplyModel("sonnet", "claude-opus-5-5", agrees)!;
        expect(r.differs).toBe(true);
        expect(r.text).toContain("claude-opus-5-5");
        expect(r.text).toContain("not sonnet");
        expect(r.text).toMatch(/next reply/);
    });

    it("does NOT flag while the process differs, a restart is pending, or nothing is known", () => {
        for (const a of [{ kind: "differs", drift }, { kind: "pending", drift }, { kind: "unknown" }] as RuntimeAgreement[]) {
            expect(lastReplyModel("sonnet", "claude-opus-5-5", a)?.differs, a.kind).toBe(false);
        }
    });

    it("makes no claim for an unrecognised reported id or selection", () => {
        expect(lastReplyModel("sonnet", "some-new-model-9", agrees)?.differs).toBe(false);
        expect(lastReplyModel("default", "claude-opus-5-5", agrees)?.differs).toBe(false);
    });

    it("a same-family reply of another version is not a family mismatch", () => {
        expect(lastReplyModel("claude-sonnet-5-5", "claude-sonnet-5", agrees)?.differs).toBe(false);
    });
});
