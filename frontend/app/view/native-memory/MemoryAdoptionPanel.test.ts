// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { outcomeText } from "./MemoryAdoptionPanel";
import { accountLabel, parseAdoptionMeta } from "@/app/view/memory-adoption-approval/MemoryAdoptionApprovalWindow";

describe("memory adoption", () => {
    it("says what an adoption did, and that it lands at the next launch", () => {
        const o = outcomeText({ status: "adopted", report: { files_added: 2, versions_kept: 1, index_lines_added: 1 } });
        expect(o?.status).toBe("adopted");
        expect(o?.text).toContain("2 file(s) added");
        expect(o?.text).toContain("next launch");
    });

    it("asks to choose again when a folder changed since the list", () => {
        const o = outcomeText({ status: "failed", error: "changed: folder 0 changed since the list was issued" });
        expect(o?.text).toContain("choose again");
    });

    it("ignores an unknown payload", () => {
        expect(outcomeText({})).toBeNull();
    });

    it("parses the approval window's request and refuses one without an approval id", () => {
        const meta = parseAdoptionMeta(
            JSON.stringify({ approval_id: "a1", summary: { agentName: "Opaz", folders: [{ account: "e562b87a-x", files: ["a.md"] }] } }),
        );
        expect(meta?.approvalId).toBe("a1");
        expect(meta?.summary.folders[0].files).toEqual(["a.md"]);
        expect(parseAdoptionMeta(JSON.stringify({ summary: {} }))).toBeNull();
        expect(parseAdoptionMeta("not json")).toBeNull();
    });

    it("names held files and accounts for the human", () => {
        expect(accountLabel("held")).toContain("another agent");
        expect(accountLabel("default")).toContain("default");
        expect(accountLabel("e562b87a-1234")).toBe("Account e562b87a");
    });
});
