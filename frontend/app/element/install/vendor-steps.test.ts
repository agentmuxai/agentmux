// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { VendorStepTracker } from "./vendor-steps";

const statuses = (t: VendorStepTracker) => t.snapshot().map((s) => `${s.id}:${s.status}`);

const RESTART_NOTE =
    'Note: the installer finished successfully, but AgentMux\'s current session doesn\'t see "Aider" yet — restart AgentMux to pick up the updated PATH.';

describe("VendorStepTracker", () => {
    it("plans get ready, install, check; never a permission row", () => {
        expect(new VendorStepTracker("Aider").snapshot().map((s) => s.label)).toEqual([
            "Get ready",
            "Install Aider",
            "Check Aider is available",
        ]);
    });

    it("walks srv's lines: the command, the download and hash, the run, then the finding", () => {
        const t = new VendorStepTracker("Aider");
        t.start();
        expect(t.line("$ curl -fsSL https://example.test/download_cli.sh | CONFIGURE=false bash")).toBe("command");
        t.line("Downloading https://example.test/download_cli.sh");
        t.line("SHA-256 ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        expect(statuses(t)).toEqual(["prepare:done", "install:active", "verify:pending"]);
        t.line("Found Aider at /home/me/.local/bin/Aider");
        t.succeed();
        expect(statuses(t)).toEqual(["prepare:done", "install:done", "verify:done"]);
    });

    it("adds the restart row when srv can't see the CLI yet", () => {
        const t = new VendorStepTracker("Aider");
        t.start();
        t.line("$ bash /tmp/x.sh");
        expect(t.line(RESTART_NOTE)).toBe("warning");
        t.succeed();
        expect(t.snapshot().map((s) => s.id)).toContain("restart");
        expect(t.snapshot().find((s) => s.id === "verify")?.status).toBe("failed");
    });

    it("says an installer's failure may have left files, and shows that on the failed step", () => {
        const t = new VendorStepTracker("Aider");
        t.start();
        t.line("$ bash /tmp/x.sh");
        const f = t.fail("bash exited Some(1)");
        expect(f.message).toContain("may have left files");
        expect(t.snapshot().find((s) => s.status === "failed")?.subline).toBe(f.message);
    });

    it("words a cancel without claiming nothing was left behind", () => {
        const t = new VendorStepTracker("Aider");
        t.start();
        t.line("$ sh /tmp/x.sh");
        const f = t.fail("cancelled");
        expect(f.category).toBe("cancelled");
        expect(f.message).toContain("may already have written some files");
        expect(f.message).not.toContain("Nothing was left behind");
    });

    it("reads srv's refusal to start as the person not approving", () => {
        for (const text of [
            "toolchain.install_vendor_cli: the user declined to install Aider",
            "toolchain.install_vendor_cli: the user did not approve installing Aider in time",
            "toolchain.install_vendor_cli: installing Aider needs the user's approval, and no AgentMux window is connected to ask the user in",
        ]) {
            const t = new VendorStepTracker("Aider");
            t.start();
            expect(t.fail(text).message).toBe("You didn't approve the install, so nothing was run.");
        }
    });

    it("reads a failed download as a network failure", () => {
        const t = new VendorStepTracker("Grok Build");
        t.start();
        t.line("$ curl -fsSL https://x.ai/cli/install.sh | bash");
        const f = t.fail("download https://x.ai/cli/install.sh: the site answered 404 Not Found");
        expect(f.category).toBe("network");
        expect(f.message).toBe("Couldn't download Grok Build's installer from its site.");
    });

    it("keeps the missing-Python reading for Kimi", () => {
        const t = new VendorStepTracker("Kimi Code CLI");
        t.start();
        t.line("$ pip install kimi-cli");
        const f = t.fail("spawn python: Python isn't installed, or isn't on PATH. Install it from the Toolchain first.");
        expect(f.category).toBe("missing_prereq");
        expect(f.message).toBe("python is needed first.");
    });

    it("says a timed-out installer was stopped", () => {
        const t = new VendorStepTracker("Muse Code");
        t.start();
        t.line("$ sh /tmp/x.sh");
        expect(t.fail("timed out after 10 minutes").message).toContain("took too long");
    });

    it("starts a retry clean", () => {
        const t = new VendorStepTracker("Aider");
        t.start();
        t.line("$ bash /tmp/x.sh");
        t.fail("bash exited Some(1)");
        t.start();
        expect(statuses(t)).toEqual(["prepare:active", "install:pending", "verify:pending"]);
        expect(t.snapshot().every((s) => s.subline === undefined)).toBe(true);
    });
});
