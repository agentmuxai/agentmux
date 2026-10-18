// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { attentionLook, DIALOG_MESSAGE_MAX, dialogMessage } from "./attention-kinds";

const a = (over: Partial<BrowserAttention>): BrowserAttention => ({ id: "1", kind: "approval", ...over });

describe("attentionLook", () => {
    it("keeps each existing kind's buttons", () => {
        expect(attentionLook(a({ kind: "handoff" }))).toMatchObject({ yes: "Done", answer: "done", no: "Cancel" });
        expect(attentionLook(a({ kind: "navigation" }))).toMatchObject({ yes: "Allow", no: "Block", agent: false });
        expect(attentionLook(a({ kind: "bypass" }))).toMatchObject({ yes: "Allow auto-approve", no: "No", tone: "danger" });
        expect(attentionLook(a({ kind: "approval" }))).toMatchObject({ yes: "Approve", answer: "approve" });
    });

    it("answers a page's dialog in the browser's own words", () => {
        expect(attentionLook(a({ kind: "dialog", dialogKind: "confirm" }))).toMatchObject({ yes: "OK", no: "Cancel" });
        expect(attentionLook(a({ kind: "dialog", dialogKind: "beforeunload" }))).toMatchObject({
            yes: "Leave",
            no: "Stay",
            answer: "approve",
        });
    });
});

describe("dialogMessage", () => {
    it("shows the page's words trimmed and clipped", () => {
        expect(dialogMessage("  Are you sure?  ")).toBe("Are you sure?");
        expect(dialogMessage(undefined)).toBe("");
        const long = dialogMessage("x".repeat(DIALOG_MESSAGE_MAX + 10));
        expect(long.length).toBe(DIALOG_MESSAGE_MAX + 1);
        expect(long.endsWith("…")).toBe(true);
    });
});
