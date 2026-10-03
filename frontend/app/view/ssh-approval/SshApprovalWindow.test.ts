// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parseSshApprovalMeta } from "./SshApprovalWindow";

describe("parseSshApprovalMeta", () => {
    it("reads a consent request with its labels and checkbox", () => {
        const meta = parseSshApprovalMeta(
            JSON.stringify({
                approval_id: "a1",
                kind: "consent",
                title: "Agent access to area54",
                message: "korp wants to run this on area54:\n\nuptime\n\nAllow it?",
                checkbox: "Always allow korp on area54",
                ok_label: "Allow",
                cancel_label: "Deny",
            }),
        );
        expect(meta).toEqual({
            approvalId: "a1",
            kind: "consent",
            title: "Agent access to area54",
            message: "korp wants to run this on area54:\n\nuptime\n\nAllow it?",
            checkbox: "Always allow korp on area54",
            okLabel: "Allow",
            cancelLabel: "Deny",
        });
    });

    it("falls back to safe labels and no checkbox", () => {
        const meta = parseSshApprovalMeta(JSON.stringify({ approval_id: "a2", kind: "secret", message: "password:" }));
        expect(meta?.kind).toBe("secret");
        expect(meta?.checkbox).toBe("");
        expect(meta?.okLabel).toBe("OK");
        expect(meta?.cancelLabel).toBe("Cancel");
        // An unknown kind is a yes/no question, never a secret field.
        expect(parseSshApprovalMeta(JSON.stringify({ approval_id: "a3", kind: "weird" }))?.kind).toBe("yesno");
    });

    it("is null without an approval id or for bad JSON", () => {
        expect(parseSshApprovalMeta(null)).toBeNull();
        expect(parseSshApprovalMeta("{")).toBeNull();
        expect(parseSshApprovalMeta(JSON.stringify({ kind: "consent" }))).toBeNull();
    });
});
