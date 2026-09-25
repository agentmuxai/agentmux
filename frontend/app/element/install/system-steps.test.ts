// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { SystemStepTracker } from "./system-steps";

const statuses = (t: SystemStepTracker) => t.snapshot().map((s) => `${s.id}:${s.status}`);

const RESTART_NOTE =
    'Note: brew finished successfully, but AgentMux\'s current session doesn\'t see "node" yet — restart AgentMux to pick up the updated PATH.';

describe("SystemStepTracker", () => {
    it("adds the permission row only when elevation is needed", () => {
        expect(new SystemStepTracker({ name: "Node.js", needsElevation: false }).snapshot().map((s) => s.label)).toEqual([
            "Get ready",
            "Install Node.js",
            "Check Node.js is available",
        ]);
        expect(new SystemStepTracker({ name: "Git", needsElevation: true }).snapshot().map((s) => s.label)).toEqual([
            "Get ready",
            "Ask for permission",
            "Install Git",
            "Check Git is available",
        ]);
    });

    it("walks a Homebrew install and shows the latest output as the subline", () => {
        const t = new SystemStepTracker({ name: "Node.js", needsElevation: false });
        t.start();
        expect(statuses(t)).toEqual(["prepare:active", "install:pending", "verify:pending"]);

        expect(t.line("$ brew install node")).toBe("command");
        expect(t.packageManager()).toBe("brew");
        expect(statuses(t)).toEqual(["prepare:done", "install:active", "verify:pending"]);

        expect(t.line("==> Pouring node--24.9.0.arm64_sequoia.bottle.tar.gz")).toBe("normal");
        expect(t.snapshot().find((s) => s.id === "install")?.subline).toBe("Pouring node--24.9.0.arm64_sequoia.bottle.tar.gz");
        // A progress-bar-only line doesn't replace the subline.
        t.line("######################################## 100.0%");
        expect(t.snapshot().find((s) => s.id === "install")?.subline).toBe("Pouring node--24.9.0.arm64_sequoia.bottle.tar.gz");

        t.succeed();
        expect(statuses(t)).toEqual(["prepare:done", "install:done", "verify:done"]);
    });

    it("waits on the permission prompt until the package manager produces output", () => {
        const t = new SystemStepTracker({ name: "Git", needsElevation: true });
        t.start();
        t.line("$ pkexec apt-get install -y git");
        expect(statuses(t)).toEqual(["prepare:done", "permission:active", "install:pending", "verify:pending"]);
        t.line("Reading package lists...");
        expect(statuses(t)).toEqual(["prepare:done", "permission:done", "install:active", "verify:pending"]);
    });

    it("turns the backend's PATH note into a restart row instead of a free-text line", () => {
        const t = new SystemStepTracker({ name: "Node.js", needsElevation: false });
        t.start();
        t.line("$ brew install node");
        expect(t.line(RESTART_NOTE)).toBe("warning");
        t.succeed();
        expect(statuses(t)).toEqual(["prepare:done", "install:done", "verify:failed", "restart:pending"]);
        const steps = t.snapshot();
        expect(steps.find((s) => s.id === "verify")?.subline).toBe("Installed, but AgentMux can't find Node.js yet.");
        expect(steps.find((s) => s.id === "restart")?.label).toBe("Restart AgentMux to finish");
    });

    it("colours package-manager errors and warnings, and remembers the first error", () => {
        const t = new SystemStepTracker({ name: "Node.js", needsElevation: false });
        t.start();
        t.line("$ brew install node");
        expect(t.line("Warning: node 24.9.0 is already installed")).toBe("warning");
        expect(t.line("W: Some index files failed to download.")).toBe("warning");
        expect(t.line("Error: node: no bottle available!")).toBe("error");
        expect(t.line("E: Unable to locate package nodejs")).toBe("error");
        expect(t.line("Already downloaded: /Users/x/Library/Caches/Homebrew/node.tar.gz")).toBe("normal");
        expect(t.fail("brew exited Some(1)").firstErrorLine).toBe(3);
    });

    it("maps a dismissed pkexec prompt to the permission row", () => {
        const t = new SystemStepTracker({ name: "Git", needsElevation: true });
        t.start();
        t.line("$ pkexec apt-get install -y git");
        const failure = t.fail("pkexec exited Some(126)");
        expect(failure.category).toBe("permission");
        expect(statuses(t)).toEqual(["prepare:done", "permission:failed", "install:pending", "verify:pending"]);
    });

    it("names the missing package manager when it can't be started", () => {
        const t = new SystemStepTracker({ name: "Node.js", needsElevation: false });
        t.start();
        t.line("$ brew install node");
        const failure = t.fail("spawn brew: No such file or directory (os error 2)");
        expect(failure.category).toBe("missing_prereq");
        expect(failure.message).toBe("brew is needed first.");
        expect(statuses(t)[0]).toBe("prepare:failed");
    });

    it("fails the running step with a plain message for an ordinary non-zero exit", () => {
        const t = new SystemStepTracker({ name: "Node.js", needsElevation: false });
        t.start();
        t.line("$ brew install node");
        t.line("==> Downloading https://ghcr.io/v2/homebrew/core/node");
        const failure = t.fail("brew exited Some(1)");
        expect(failure.category).toBe("unknown");
        expect(failure.message).toBe('Something went wrong while running "Install Node.js".');
        expect(statuses(t)).toEqual(["prepare:done", "install:failed", "verify:pending"]);
    });

    it("start() resets for Retry, dropping a restart row from the previous run", () => {
        const t = new SystemStepTracker({ name: "Node.js", needsElevation: false });
        t.start();
        t.line(RESTART_NOTE);
        t.succeed();
        t.start();
        expect(statuses(t)).toEqual(["prepare:active", "install:pending", "verify:pending"]);
    });
});
