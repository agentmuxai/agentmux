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
import {
    isPersistentLaunch,
    parseProviderFlags,
    selectLaunchArgs,
    withProviderFlags,
    type LaunchArgsProvider,
} from "./launch-args";

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

/**
 * The per-turn `cmd:args` rebuild derives its base from the provider CATALOG,
 * so anything the launch path appended afterwards is dropped unless something
 * puts it back. Two things get appended, and they are NOT the same kind of
 * thing (#2872):
 *
 *   provider_flags   durable  — how this agent always runs → reapply every turn
 *   --fork-session   one-shot — fork the session being resumed AT LAUNCH
 *
 * Reapplying `--fork-session` would pair it with the `--resume <sid>` srv adds
 * on every later turn, forking again each time instead of once. So the fix is
 * deliberately asymmetric, and that asymmetry is the part worth pinning.
 */
describe("provider_flags are durable; --fork-session is not", () => {
    it("splits provider_flags on whitespace, the way the launch path always has", () => {
        expect(parseProviderFlags("--foo --bar=1")).toEqual(["--foo", "--bar=1"]);
        expect(parseProviderFlags("  --a   --b  ")).toEqual(["--a", "--b"]);
    });

    /** A pane launched before the meta key existed, or an agent with none. */
    it("treats absent or non-string provider_flags as no flags", () => {
        expect(parseProviderFlags(undefined)).toEqual([]);
        expect(parseProviderFlags("")).toEqual([]);
        expect(parseProviderFlags(null)).toEqual([]);
        expect(parseProviderFlags(42)).toEqual([]);
    });

    /** The bug: the user's flags must survive a rebuild. */
    it("reapplies provider_flags onto a rebuilt argv", () => {
        const rebuilt = ["-p", "--model", "opus"];
        expect(withProviderFlags(rebuilt, "--my-flag 7")).toEqual([
            "-p", "--model", "opus", "--my-flag", "7",
        ]);
    });

    it("returns the argv untouched when there are no flags to reapply", () => {
        const rebuilt = ["-p", "--model", "opus"];
        expect(withProviderFlags(rebuilt, "")).toEqual(rebuilt);
        expect(withProviderFlags(rebuilt, undefined)).toEqual(rebuilt);
    });

    /** The asymmetry. `--fork-session` must NOT come back: this helper only
     *  ever knows about provider_flags, so a future caller can't accidentally
     *  route a one-shot launch intent through it. */
    it("never reintroduces --fork-session", () => {
        const rebuilt = ["-p", "--model", "opus"];
        expect(withProviderFlags(rebuilt, "--my-flag")).not.toContain("--fork-session");
        // Even if it were somehow stored there, it is the caller's job not to —
        // this documents that the helper is not a general "restore everything".
        expect(parseProviderFlags("--fork-session")).toEqual(["--fork-session"]);
    });

    /** Appends, never rewrites — the catalog base can't already carry these. */
    it("does not mutate the argv it is given", () => {
        const rebuilt = ["-p"];
        const out = withProviderFlags(rebuilt, "--x");
        expect(rebuilt).toEqual(["-p"]);
        expect(out).not.toBe(rebuilt);
    });
});
