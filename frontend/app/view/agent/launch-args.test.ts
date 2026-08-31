// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Container agents must never launch with `--input-format stream-json`.
 *
 * This is the fix for the reason container ("sandbox") agents had never once
 * started: they inherited the PERSISTENT controller's args while running the
 * SUBPROCESS controller's per-turn `docker exec`, whose stdin is raw text. The
 * CLI met the startup markdown as its first line and died in under a second
 * with `JSON Parse error: Unrecognized token '#'`.
 */

import { describe, expect, it } from "vitest";
import { isPersistentLaunch, selectLaunchArgs, type LaunchArgsProvider } from "./launch-args";

/** Claude's real shape, copied from providers/catalog.ts. */
const claude: LaunchArgsProvider = {
    controllerType: "persistent",
    launchArgs: ["--output-format", "stream-json", "--verbose", "--include-partial-messages"],
    persistentLaunchArgs: [
        "--input-format", "stream-json",
        "--output-format", "stream-json",
        "--verbose", "--include-partial-messages",
        "--permission-prompt-tool", "stdio",
        "--permission-mode", "default",
    ],
};

/** A provider that is subprocess-shaped to begin with. */
const codex: LaunchArgsProvider = {
    controllerType: "subprocess",
    launchArgs: ["--json"],
};

describe("isPersistentLaunch", () => {
    it("is true for a persistent provider on a host agent — unchanged behaviour", () => {
        expect(isPersistentLaunch(claude, "host")).toBe(true);
        expect(isPersistentLaunch(claude, undefined)).toBe(true);
    });

    /** The fix. A container agent is subprocess-shaped whatever the provider says. */
    it("is FALSE for a container agent, even on a persistent provider", () => {
        expect(isPersistentLaunch(claude, "container")).toBe(false);
    });

    it("stays false for a non-persistent provider regardless of mode", () => {
        expect(isPersistentLaunch(codex, "host")).toBe(false);
        expect(isPersistentLaunch(codex, "container")).toBe(false);
    });
});

describe("selectLaunchArgs", () => {
    /** The single assertion this whole fix exists for. */
    it("never gives a container agent --input-format", () => {
        const args = selectLaunchArgs(claude, "container");
        expect(args).not.toContain("--input-format");
    });

    it("gives a container agent the plain launchArgs", () => {
        expect(selectLaunchArgs(claude, "container")).toEqual(claude.launchArgs);
    });

    /** …while a host agent still gets them — the persistent controller owns a
     *  long-lived stdin and genuinely does write JSON envelopes over it. */
    it("still gives a host agent --input-format", () => {
        const args = selectLaunchArgs(claude, "host");
        expect(args).toContain("--input-format");
        expect(args).toEqual(claude.persistentLaunchArgs);
    });

    /** --output-format is required in BOTH cases: it's how the pane parses the
     *  agent's output at all. Only the input side differs. */
    it("keeps --output-format stream-json for container and host alike", () => {
        expect(selectLaunchArgs(claude, "container")).toContain("--output-format");
        expect(selectLaunchArgs(claude, "host")).toContain("--output-format");
    });

    it("falls back to launchArgs when the provider declares no persistent variant", () => {
        expect(selectLaunchArgs(codex, "host")).toEqual(["--json"]);
    });

    /** Returns a copy — callers push provider_flags onto the result, and
     *  mutating the catalog would corrupt every later launch in the session. */
    it("returns a fresh array, never the provider's own", () => {
        const args = selectLaunchArgs(claude, "host");
        expect(args).not.toBe(claude.persistentLaunchArgs);
        args.push("--mutated");
        expect(claude.persistentLaunchArgs).not.toContain("--mutated");
    });
});
