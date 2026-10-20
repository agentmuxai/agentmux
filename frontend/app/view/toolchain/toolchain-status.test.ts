// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { resolveDrift } from "@/app/view/agent/providers/version-drift";
import { describe, expect, it } from "vitest";
import { toolNotes, toolStatus, widgetStatus, type ToolFacts } from "./toolchain-status";

const facts = (over: Partial<ToolFacts> = {}): ToolFacts => ({ loading: false, found: true, kind: "core", ...over });
const drift = (installed?: string, pinned?: string, latest?: string, found = true) =>
    resolveDrift({ installed, pinned, latest, found });

describe("toolStatus", () => {
    it("is busy while it checks", () => {
        expect(
            toolStatus(facts({ loading: true, found: false }), drift(undefined, undefined, undefined, false))
        ).toEqual({
            tone: "busy",
            label: "Checking",
            subtitle: "Checking…",
            attention: false,
        });
    });

    it("asks for the user when a required tool is missing, and stays quiet for an optional one", () => {
        const none = drift(undefined, undefined, undefined, false);
        expect(toolStatus(facts({ found: false }), none)).toMatchObject({
            tone: "warn",
            label: "Not found",
            attention: true,
        });
        expect(toolStatus(facts({ found: false, optional: true }), none)).toMatchObject({
            tone: "idle",
            label: "Not installed (optional)",
            subtitle: "Optional",
            attention: false,
        });
    });

    it("shows the version when found, and a warning when the pin is ahead of it", () => {
        expect(toolStatus(facts({ version: "2.1.0" }), drift("2.1.0"))).toMatchObject({
            tone: "ok",
            subtitle: "2.1.0",
        });
        expect(toolStatus(facts({ version: "2.0.0" }), drift("2.0.0", "2.1.0"))).toMatchObject({
            tone: "warn",
            label: "Update available: 2.1.0",
            subtitle: "2.0.0",
            attention: false,
        });
        expect(toolStatus(facts(), drift())).toMatchObject({ tone: "ok", subtitle: "Installed" });
    });
});

describe("toolNotes", () => {
    it("has none while checking or when missing", () => {
        expect(toolNotes(facts({ loading: true }), drift())).toEqual([]);
        expect(toolNotes(facts({ found: false }), drift(undefined, undefined, undefined, false))).toEqual([]);
    });

    it("says where the tool came from", () => {
        expect(toolNotes(facts({ source: "local_install" }), drift()).map((n) => n.text)).toEqual(["managed"]);
        expect(toolNotes(facts({ kind: "provider", source: "system_path" }), drift()).map((n) => n.text)).toEqual([
            "your install",
        ]);
        expect(toolNotes(facts({ kind: "core", source: "system_path" }), drift())).toEqual([]);
    });

    it("compares with the pin, and shows a pin behind upstream quietly", () => {
        expect(toolNotes(facts(), drift("2.1.0", "2.1.0")).map((n) => n.text)).toEqual(["up to date"]);
        expect(toolNotes(facts(), drift("2.0.0", "2.1.0")).map((n) => n.text)).toEqual(["2.1.0 available"]);
        expect(toolNotes(facts(), drift("2.2.0", "2.1.0")).map((n) => n.text)).toEqual(["ahead of pin"]);
        const behindUpstream = toolNotes(facts(), drift("2.1.0", "2.1.0", "2.3.0"));
        expect(behindUpstream.map((n) => n.text)).toEqual(["up to date", "pin 2.1.0 · upstream 2.3.0"]);
        expect(behindUpstream[1].title).toContain("Nothing for you to do");
    });
});

describe("widgetStatus", () => {
    const w = { cliLoading: false, cliFound: false, healthLoading: false, running: false, installKind: "pip" as const };

    it("names the port while it runs", () => {
        expect(widgetStatus({ ...w, running: true }, 8188)).toEqual({
            tone: "ok",
            label: "Running",
            subtitle: "Running on port 8188",
        });
    });

    it("distinguishes checking, installed, and not installed or detected", () => {
        expect(widgetStatus({ ...w, cliLoading: true }, 1).tone).toBe("busy");
        expect(widgetStatus({ ...w, cliFound: true }, 1)).toMatchObject({
            tone: "idle",
            subtitle: "Installed, not running",
        });
        expect(widgetStatus(w, 1).subtitle).toBe("Not installed");
        expect(widgetStatus({ ...w, installKind: "manual" }, 1).subtitle).toBe("Not detected");
    });
});
