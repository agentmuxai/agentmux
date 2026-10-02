// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { cliNoticeText, parseCliNoticeFrame } from "./cli-notice";
import { parseHistoryLines } from "./parseHistoryLines";
import type { CliNoticeNode } from "./types";

const TS = "2026-10-01T08:00:00+00:00";

const install = (state: string, extra: Record<string, unknown> = {}) => ({
    type: "system",
    subtype: "agentmux_cli_install",
    install_id: "claude-2.1.285-1",
    provider: "claude",
    version: "2.1.285",
    state,
    timestamp: TS,
    ...extra,
});

const changed = (pinned: string | null) => ({
    type: "system",
    subtype: "agentmux_cli_version_changed",
    provider: "claude",
    from: "2.1.218",
    to: "2.1.285",
    pinned,
    timestamp: TS,
});

describe("parseCliNoticeFrame", () => {
    it("reads an install frame, with one id for every state of the same install", () => {
        const installing = parseCliNoticeFrame(install("installing"), 0)!;
        const installed = parseCliNoticeFrame(install("installed", { seconds: 14 }), 0)!;
        expect(installing).toMatchObject({ type: "cli_notice", kind: "install", state: "installing", version: "2.1.285" });
        expect(installed.seconds).toBe(14);
        expect(installed.id).toBe(installing.id);
        expect(installing.timestamp).toBe(Date.parse(TS));
    });

    it("reads a version-change frame", () => {
        expect(parseCliNoticeFrame(changed("2.1.285"), 0)).toMatchObject({
            kind: "version_changed",
            from: "2.1.218",
            to: "2.1.285",
            pinned: "2.1.285",
        });
    });

    it("ignores other frames and malformed ones", () => {
        expect(parseCliNoticeFrame({ type: "system", subtype: "init" }, 0)).toBeNull();
        expect(parseCliNoticeFrame({ ...install("installing"), state: "maybe" }, 0)).toBeNull();
        expect(parseCliNoticeFrame({ ...changed(null), to: "" }, 0)).toBeNull();
        expect(parseCliNoticeFrame({ ...install("installing"), provider: undefined }, 0)).toBeNull();
        expect(parseCliNoticeFrame("not an object", 0)).toBeNull();
    });

    it("falls back to `now` when the frame has no usable timestamp", () => {
        expect(parseCliNoticeFrame({ ...install("installing"), timestamp: undefined }, 42)!.timestamp).toBe(42);
    });
});

describe("cliNoticeText", () => {
    const text = (frame: object) => cliNoticeText(parseCliNoticeFrame(frame, 0)!);

    it("names the CLI by its display name", () => {
        expect(text(install("installing"))).toEqual({
            label: "Installing Claude Code 2.1.285…",
            detail: "The agent starts once it's installed",
            tone: "info",
        });
        expect(text(install("installed", { seconds: 14.4 })).label).toBe("Installed Claude Code 2.1.285 (14 s)");
        expect(text(install("installed", { seconds: 0.2 })).label).toBe("Installed Claude Code 2.1.285 (1 s)");
    });

    it("shows a failed install as an error with npm's message", () => {
        expect(text(install("failed", { error: "exit 1; check the output above" }))).toEqual({
            label: "Couldn't install Claude Code 2.1.285",
            detail: "exit 1; check the output above",
            tone: "error",
        });
    });

    it("warns only when the new version isn't the pinned one", () => {
        expect(text(changed("2.1.285"))).toEqual({ label: "Claude Code updated: 2.1.218 → 2.1.285", detail: null, tone: "info" });
        expect(text(changed("2.1.290"))).toMatchObject({ detail: "Not the version AgentMux pins (2.1.290)", tone: "warn" });
        expect(text(changed(null)).tone).toBe("info");
    });

    it("uses the provider id when the catalog doesn't know it", () => {
        expect(text({ ...install("installing"), provider: "mystery" }).label).toBe("Installing mystery 2.1.285…");
    });
});

describe("replay", () => {
    it("collapses an install's frames into one row that ends in its final state", () => {
        const lines = [install("installing"), install("installed", { seconds: 9 }), changed("2.1.285")].map((f) =>
            JSON.stringify(f)
        );
        const notices = parseHistoryLines(lines, "claude-stream-json").nodes.filter(
            (n): n is CliNoticeNode => n.type === "cli_notice"
        );
        expect(notices.map((n) => [n.kind, n.state ?? null])).toEqual([
            ["install", "installed"],
            ["version_changed", null],
        ]);
    });
});

// A failed compaction (SPEC_COMPACTION_ESTIMATED_PROGRESS_AND_STREAM_FRAMES_2026_10_01.md §6/§10).
// The frame is the one the real CLI 2.1.287 wrote when the summarizing call
// failed; the turn then ended with `is_error: false`, so nothing else says so.
const COMPACT_FAILED = {
    type: "system",
    subtype: "status",
    status: null,
    compact_result: "failed",
    compact_error: "Error during compaction: API Error: 400 probe: simulated summarizer failure",
    session_id: "a9acc189-3ec1-4cce-960e-2bab4a55e486",
    uuid: "e211036a-3ae3-4dbf-a47f-9d6457209af0",
};

describe("a failed compaction", () => {
    it("is a cli_notice row carrying the CLI's own reason, without its boilerplate prefix", () => {
        expect(parseCliNoticeFrame(COMPACT_FAILED, 7)).toEqual({
            type: "cli_notice",
            id: "compaction-failed-e211036a-3ae3-4dbf-a47f-9d6457209af0",
            kind: "compaction_failed",
            provider: "claude",
            error: "API Error: 400 probe: simulated summarizer failure",
            timestamp: 7,
        });
    });

    it("reads as an error, naming the CLI and giving the reason", () => {
        expect(cliNoticeText(parseCliNoticeFrame(COMPACT_FAILED, 0)!)).toEqual({
            label: "Claude Code couldn't compact the conversation",
            detail: "API Error: 400 probe: simulated summarizer failure",
            tone: "error",
        });
    });

    it("still reports a failure that came without a reason", () => {
        const node = parseCliNoticeFrame({ ...COMPACT_FAILED, compact_error: undefined }, 0)!;
        expect(cliNoticeText(node)).toMatchObject({ detail: null, tone: "error" });
    });

    it("keeps a reason that lacks the usual prefix as it is", () => {
        expect(parseCliNoticeFrame({ ...COMPACT_FAILED, compact_error: "boom" }, 0)!.error).toBe("boom");
    });

    it.each([
        ["the start", { status: "compacting", compact_result: undefined, compact_error: undefined }],
        ["a success", { compact_result: "success", compact_error: undefined }],
        ["a status with no result", { compact_result: undefined, compact_error: undefined }],
        ["another status", { status: "requesting", compact_result: undefined, compact_error: undefined }],
    ])("ignores %s", (_name, patch) => {
        expect(parseCliNoticeFrame({ ...COMPACT_FAILED, ...patch }, 0)).toBeNull();
    });

    it("needs the frame's uuid for a stable id, and skips a frame without one", () => {
        expect(parseCliNoticeFrame({ ...COMPACT_FAILED, uuid: undefined }, 0)).toBeNull();
    });

    it("lands on one id live and on replay, so seeing the frame twice shows one row", () => {
        const live = parseCliNoticeFrame(COMPACT_FAILED, 1)!;
        const notices = parseHistoryLines([JSON.stringify(COMPACT_FAILED), JSON.stringify(COMPACT_FAILED)], "claude-stream-json").nodes.filter(
            (n): n is CliNoticeNode => n.type === "cli_notice"
        );
        expect(notices).toHaveLength(1);
        expect(notices[0].id).toBe(live.id);
        expect(notices[0]).toMatchObject({ kind: "compaction_failed" });
    });
});
