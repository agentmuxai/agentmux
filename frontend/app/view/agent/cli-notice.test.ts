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
