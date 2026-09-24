// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Step tracker for a system-tool install through the platform package
 * manager (winget, Homebrew, or pkexec with a Linux package manager) —
 * the §6.2 "System tool" plan of SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md:
 * Get ready → Ask for permission (only when elevation is needed) →
 * Install <Tool> → Check <Tool> is available, plus a "Restart AgentMux to
 * finish" row only when the backend's PATH re-check can't see the tool.
 *
 * The backend (system_install_handlers.rs) runs one package-manager
 * process and echoes `$ <program> <args>` before spawning it, so the
 * steps come from command boundaries plus a few output markers. Package
 * managers can't be interrupted safely, so this plan has no cancel path.
 */

import {
    failureMessage,
    typedErrorCategory,
    type InstallErrorCategory,
    type InstallFailure,
    type InstallStep as GenericInstallStep,
    type InstallStepStatus,
    type LineTone,
    type StepTracker,
} from "./install-types";

export type SystemStepId = "prepare" | "permission" | "install" | "verify" | "restart";
type InstallStep = GenericInstallStep<SystemStepId>;

// Emitted by the backend after a successful install when this process's
// PATH still can't see the tool (system_install_handlers.rs).
const RESTART_NOTE_RE = /^Note: .* restart AgentMux to pick up the updated PATH\.?$/;
// Homebrew `Error:`, apt `E:`, winget/pkexec `Error …`.
const ERROR_LINE_RE = /^(?:Error|E|error|ERROR)(?::|!|\s+executing\b)/;
const WARN_LINE_RE = /^(?:Warning|W|warning|WARNING)[:!]/;
// Spawn failure of the package manager itself: `spawn <program>: <os error>`.
const SPAWN_FAIL_RE = /^spawn (\S+):/;
// Non-zero exit: `<program> exited Some(<code>)`.
const EXIT_RE = /^(\S+) exited Some\((-?\d+)\)/;
const SUBLINE_MAX = 80;

export interface SystemStepTrackerOptions {
    /** Display name, e.g. "Node.js". */
    name: string;
    /** From `toolchain.resolve_install_command` — adds the permission row. */
    needsElevation: boolean;
}

export class SystemStepTracker implements StepTracker {
    private steps: InstallStep[] = [];
    private readonly name: string;
    private readonly needsElevation: boolean;
    private program: string | undefined;
    private restartNeeded = false;
    private lineIndex = 0;
    private firstErrorLine: number | undefined;

    constructor(opts: SystemStepTrackerOptions) {
        this.name = opts.name;
        this.needsElevation = opts.needsElevation;
        this.steps = this.plan();
    }

    snapshot(): InstallStep[] {
        return this.steps.map((s) => ({ ...s }));
    }

    start(): void {
        this.steps = this.plan();
        this.program = undefined;
        this.restartNeeded = false;
        this.lineIndex = 0;
        this.firstErrorLine = undefined;
        this.set("prepare", { status: "active" });
    }

    line(text: string): LineTone {
        const index = this.lineIndex++;

        // The backend's `$ <program> …` echo: the command is resolved and
        // about to run. With elevation, the OS prompt comes next.
        if (text.startsWith("$ ")) {
            this.program = text.slice(2).split(/\s+/)[0];
            this.finish("prepare");
            this.advanceTo(this.needsElevation ? "permission" : "install");
            return "command";
        }

        if (RESTART_NOTE_RE.test(text)) {
            this.restartNeeded = true;
            return "warning";
        }

        // Any package-manager output means it's running — and, with
        // elevation, that the permission prompt was answered.
        this.advanceTo("install");
        const tone: LineTone = ERROR_LINE_RE.test(text) ? "error" : WARN_LINE_RE.test(text) ? "warning" : "normal";
        if (tone === "error" && this.firstErrorLine === undefined) this.firstErrorLine = index;
        const subline = toSubline(text);
        if (subline && this.status("install") === "active") this.set("install", { subline });
        return tone;
    }

    tick(): void {
        // No time-based transitions: package managers report their own progress.
    }

    succeed(): void {
        for (const id of ["prepare", "permission", "install"] as const) {
            if (this.has(id)) this.set(id, { status: "done", subline: undefined });
        }
        if (this.restartNeeded) {
            // Spec §6.2: the restart row replaces the old free-text note.
            this.set("verify", {
                status: "failed",
                subline: failureMessage("not_on_path", { name: this.name, failedLabel: "" }),
            });
            this.steps = [
                ...this.steps,
                {
                    id: "restart",
                    label: "Restart AgentMux to finish",
                    status: "pending",
                    subline: "Quit and reopen AgentMux, then check again.",
                },
            ];
        } else {
            this.set("verify", { status: "done", subline: undefined });
        }
    }

    fail(error: unknown): InstallFailure {
        const text = typeof error === "string" ? error : "";
        let category: InstallErrorCategory = typedErrorCategory(error) ?? "unknown";
        let missingTool: string | undefined;
        let failedStep: SystemStepId = this.activeStep() ?? "prepare";

        const spawn = SPAWN_FAIL_RE.exec(text);
        const exit = EXIT_RE.exec(text);
        if (text === "cancelled") {
            category = "cancelled";
        } else if (spawn) {
            // The package manager (or pkexec) isn't there to run.
            category = "missing_prereq";
            missingTool = spawn[1];
            failedStep = "prepare";
        } else if (exit && exit[1] === "pkexec" && exit[2] === "126") {
            // pkexec exits 126 when the user dismissed the auth dialog
            // (spec §13.3).
            category = "permission";
            failedStep = this.has("permission") ? "permission" : failedStep;
        }

        const failedLabel = this.steps.find((s) => s.id === failedStep)?.label ?? "installing";
        const message = failureMessage(category, { name: this.name, failedLabel, missingTool });

        let reached = false;
        for (const s of this.steps) {
            if (s.id === failedStep) {
                reached = true;
                this.set(s.id, { status: "failed", subline: message });
            } else if (!reached && (s.status === "pending" || s.status === "active")) {
                this.set(s.id, { status: "done", subline: undefined });
            } else if (reached && s.status === "active") {
                this.set(s.id, { status: "pending", subline: undefined });
            }
        }
        return { category, message, firstErrorLine: this.firstErrorLine };
    }

    /** The package manager the backend echoed, once known. */
    packageManager(): string | undefined {
        return this.program;
    }

    private plan(): InstallStep[] {
        const steps: InstallStep[] = [{ id: "prepare", label: "Get ready", status: "pending" }];
        if (this.needsElevation) steps.push({ id: "permission", label: "Ask for permission", status: "pending" });
        steps.push({ id: "install", label: `Install ${this.name}`, status: "pending" });
        steps.push({ id: "verify", label: `Check ${this.name} is available`, status: "pending" });
        return steps;
    }

    /** Finishes every step before `id` and makes `id` active. */
    private advanceTo(id: SystemStepId): void {
        if (!this.has(id)) return;
        for (const s of this.steps) {
            if (s.id === id) break;
            if (s.status === "pending" || s.status === "active") this.finish(s.id);
        }
        if (this.status(id) === "pending") this.set(id, { status: "active" });
    }

    private finish(id: SystemStepId): void {
        if (this.has(id)) this.set(id, { status: "done", subline: undefined });
    }

    private activeStep(): SystemStepId | undefined {
        return this.steps.find((s) => s.status === "active")?.id;
    }

    private has(id: SystemStepId): boolean {
        return this.steps.some((s) => s.id === id);
    }

    private status(id: SystemStepId): InstallStepStatus | undefined {
        return this.steps.find((s) => s.id === id)?.status;
    }

    private set(id: SystemStepId, patch: Partial<InstallStep>): void {
        this.steps = this.steps.map((s) => (s.id === id ? { ...s, ...patch } : s));
    }
}

/**
 * The latest output line, tidied for the active step's one-line subline:
 * Homebrew's `==> ` prefix dropped, progress-bar-only lines ignored.
 */
function toSubline(text: string): string | undefined {
    const t = text.replace(/^==>\s*/, "").trim();
    if (!/[A-Za-z]{2}/.test(t)) return undefined;
    return t.length > SUBLINE_MAX ? `${t.slice(0, SUBLINE_MAX - 1)}…` : t;
}
