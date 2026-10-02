// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { lastReplyModel, modelFamily, runningModelNote } from "./resolved-model";
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

describe("runningModelNote — what the CLI itself reports", () => {
    const base = { selectedModel: "sonnet", selectedEffort: "high", effortApplies: true, lastReply: null, agreement: agrees };

    it("falls back to the last reply when the CLI has not answered", () => {
        expect(runningModelNote({ ...base, effective: undefined, lastReply: "claude-sonnet-5-5" })?.text).toBe(
            "Last reply used claude-sonnet-5-5",
        );
        expect(runningModelNote({ ...base, effective: {} })).toBeNull();
    });

    it("states the resolved model and effort, with no warning when they match the selection", () => {
        expect(runningModelNote({ ...base, effective: { model: "claude-sonnet-5-5", effort: "high" } })).toEqual({
            text: "Running claude-sonnet-5-5 · effort high",
            differs: false,
        });
    });

    it("a model with no effort (Haiku) says only the model", () => {
        const r = runningModelNote({ ...base, selectedModel: "haiku", effortApplies: false, effective: { model: "claude-haiku-4-5-20251001" } });
        expect(r).toEqual({ text: "Running claude-haiku-4-5-20251001", differs: false });
    });

    it("flags a different family, and an effort other than the selected one, without the stale-reply caveat", () => {
        const m = runningModelNote({ ...base, effective: { model: "claude-opus-5-5", effort: "high" } })!;
        expect(m.differs).toBe(true);
        expect(m.text).toContain("not sonnet");
        expect(m.text).not.toMatch(/next reply/);
        const e = runningModelNote({ ...base, effective: { model: "claude-sonnet-5-5", effort: "medium" } })!;
        expect(e.differs).toBe(true);
        expect(e.text).toContain("not effort high");
    });

    it("does not judge a process that was not spawned with the selection", () => {
        for (const a of [{ kind: "differs", drift } as RuntimeAgreement, { kind: "pending", drift } as RuntimeAgreement, { kind: "unknown" } as RuntimeAgreement]) {
            const r = runningModelNote({ ...base, agreement: a, effective: { model: "claude-opus-5-5", effort: "medium" } })!;
            expect(r.differs, a.kind).toBe(false);
            expect(r.text).toContain("Running claude-opus-5-5");
        }
    });

    it("does not compare efforts for a model that takes none", () => {
        const r = runningModelNote({ ...base, selectedModel: "haiku", effortApplies: false, effective: { model: "claude-haiku-4-5-20251001", effort: "medium" } })!;
        expect(r.differs).toBe(false);
    });
});
