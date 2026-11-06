// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Step tracker for a vendor-installed agent CLI (Kimi, Antigravity, goose, Grok Build, Aider, Muse
 * Code): srv, after the person approves it, downloads the vendor's installer, shows its hash and
 * runs it (vendor_install_handlers.rs, SPEC_ONE_CLICK_VENDOR_CLI_INSTALL_2026_10_20.md). The
 * steps and the output markers are a system-tool install's, so this wraps that tracker and
 * words the failures for an installer AgentMux doesn't control: it can leave files behind, and a
 * cancel doesn't take them back.
 */

import type { InstallFailure, InstallStep, LineTone, StepTracker } from "./install-types";
import { SystemStepTracker } from "./system-steps";

// srv words a failed download `download <url>: <why>`.
const DOWNLOAD_FAIL_RE = /^download /;
const TIMEOUT_RE = /^timed out after /;
// srv refused to start: the person said no, or never answered.
const NOT_APPROVED_RE = /declined to install|did not approve installing|needs the user's approval/;

export class VendorStepTracker implements StepTracker {
    private readonly inner: SystemStepTracker;
    private note: string | undefined;

    constructor(private readonly name: string) {
        this.inner = new SystemStepTracker({ name, needsElevation: false });
    }

    snapshot(): InstallStep[] {
        const steps = this.inner.snapshot();
        const note = this.note;
        return note ? steps.map((s) => (s.status === "failed" ? { ...s, subline: note } : s)) : steps;
    }

    start(): void {
        this.note = undefined;
        this.inner.start();
    }

    line(text: string): LineTone {
        return this.inner.line(text);
    }

    tick(): void {
        this.inner.tick();
    }

    succeed(): void {
        this.inner.succeed();
    }

    fail(error: unknown): InstallFailure {
        const failure = this.inner.fail(error);
        const text = typeof error === "string" ? error : "";
        const worded = this.wording(failure, text);
        this.note = worded.message;
        return worded;
    }

    private wording(failure: InstallFailure, text: string): InstallFailure {
        const { name } = this;
        if (text === "cancelled") {
            return { ...failure, message: `Cancelled. ${name}'s installer may already have written some files.` };
        }
        if (NOT_APPROVED_RE.test(text)) {
            return { ...failure, message: "You didn't approve the install, so nothing was run." };
        }
        if (DOWNLOAD_FAIL_RE.test(text)) {
            return { ...failure, category: "network", message: `Couldn't download ${name}'s installer from its site.` };
        }
        if (TIMEOUT_RE.test(text)) {
            return { ...failure, message: `${name}'s installer took too long and was stopped. It may have left files.` };
        }
        if (failure.category === "unknown") {
            return { ...failure, message: `${name}'s installer didn't finish. It may have left files; the last lines are under Details.` };
        }
        return failure;
    }
}
