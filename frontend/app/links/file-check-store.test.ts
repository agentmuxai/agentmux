// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const ask = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { FsLinkFileCheckCommand: (...a: unknown[]) => ask(...a) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { checkFileNow, deadReason, fileCheck, isPlain, requestFileCheck } from "./file-check-store";

const settle = () => new Promise((r) => setTimeout(r, 0));

describe("the file check store", () => {
    beforeEach(() => ask.mockReset());

    it("asks srv once per pane and path, however often the link is hovered", async () => {
        ask.mockResolvedValue({ state: "found", reason: "", resolved: "/w/a.md" });
        requestFileCheck("pane-1", "a.md");
        requestFileCheck("pane-1", "a.md");
        await settle();
        requestFileCheck("pane-1", "a.md");
        expect(ask).toHaveBeenCalledTimes(1);
        expect(ask).toHaveBeenCalledWith(expect.anything(), { path: "a.md", block_id: "pane-1" }, {});
        expect(fileCheck("pane-1", "a.md")?.state).toBe("found");
        expect(deadReason("pane-1", "a.md")).toBeUndefined();
    });

    it("does not make a missing name that is also a web site dead: it is the site", async () => {
        ask.mockResolvedValue({ state: "missing", reason: "Not found.", resolved: "" });
        await checkFileNow("pane-4", "docs.rs");
        expect(fileCheck("pane-4", "docs.rs")?.state).toBe("missing");
        expect(deadReason("pane-4", "docs.rs")).toBeUndefined();
        await checkFileNow("pane-4", "gone.md");
        expect(deadReason("pane-4", "gone.md")).toBe("Not found.");
    });

    it("draws a missing bare name that could be a site's as plain text, not dead; outside stays dead", async () => {
        ask.mockResolvedValueOnce({ state: "missing", reason: "Not found.", resolved: "" }).mockResolvedValueOnce({ state: "outside", reason: "Outside this agent's folders.", resolved: "" });
        expect(isPlain("pane-5", "parser.rs")).toBe(false);
        await checkFileNow("pane-5", "parser.rs");
        expect(isPlain("pane-5", "parser.rs")).toBe(true);
        expect(deadReason("pane-5", "parser.rs")).toBeUndefined();
        await checkFileNow("pane-5", "tool.sh");
        expect(isPlain("pane-5", "tool.sh")).toBe(false);
        expect(deadReason("pane-5", "tool.sh")).toBe("Outside this agent's folders.");
    });

    it("asks again about a missing name on a later hover, so a file made since shows as a link without a reload", async () => {
        const now = vi.spyOn(Date, "now");
        now.mockReturnValue(1_000_000);
        ask.mockResolvedValueOnce({ state: "missing", reason: "Not found.", resolved: "" });
        requestFileCheck("pane-6", "later.rs");
        await settle();
        expect(isPlain("pane-6", "later.rs")).toBe(true);
        // Soon after: the answer is reused, no second ask.
        now.mockReturnValue(1_005_000);
        requestFileCheck("pane-6", "later.rs");
        await settle();
        expect(ask).toHaveBeenCalledTimes(1);
        // A while later the file is there: the hover asks again and the name is a link.
        now.mockReturnValue(1_011_000);
        ask.mockResolvedValueOnce({ state: "found", reason: "", resolved: "/w/later.rs" });
        requestFileCheck("pane-6", "later.rs");
        await settle();
        expect(ask).toHaveBeenCalledTimes(2);
        expect(isPlain("pane-6", "later.rs")).toBe(false);
        expect(fileCheck("pane-6", "later.rs")?.state).toBe("found");
        // A found answer is kept for the page.
        now.mockReturnValue(1_100_000);
        requestFileCheck("pane-6", "later.rs");
        await settle();
        expect(ask).toHaveBeenCalledTimes(2);
        now.mockRestore();
    });

    it("makes a missing file, and one outside the folders, dead with the reason srv gave", async () => {
        ask.mockResolvedValueOnce({ state: "missing", reason: "Not found.", resolved: "" }).mockResolvedValueOnce({ state: "outside", reason: "Outside this agent's folders.", resolved: "" });
        await checkFileNow("pane-2", "gone.md");
        await checkFileNow("pane-2", "../x.md");
        expect(deadReason("pane-2", "gone.md")).toBe("Not found.");
        expect(deadReason("pane-2", "../x.md")).toBe("Outside this agent's folders.");
    });

    it("keeps the answers of two panes apart", async () => {
        ask.mockResolvedValueOnce({ state: "found", reason: "", resolved: "" }).mockResolvedValueOnce({ state: "outside", reason: "Outside this agent's folders.", resolved: "" });
        await checkFileNow("pane-3", "same.md");
        await checkFileNow("pane-4", "same.md");
        expect(deadReason("pane-3", "same.md")).toBeUndefined();
        expect(deadReason("pane-4", "same.md")).toBe("Outside this agent's folders.");
    });

    it("does not keep a limited or failed answer, so the link can be asked again", async () => {
        ask.mockResolvedValueOnce({ state: "limited", reason: "slow down", resolved: "" }).mockRejectedValueOnce(new Error("offline"));
        await checkFileNow("pane-5", "a.md");
        expect(fileCheck("pane-5", "a.md")).toBeUndefined();
        await checkFileNow("pane-5", "b.md");
        expect(fileCheck("pane-5", "b.md")).toBeUndefined();
        expect(deadReason("pane-5", "a.md")).toBeUndefined();
    });
});
