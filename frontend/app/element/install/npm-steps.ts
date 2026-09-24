// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Frontend line classifier for an `npm install` session — phase 1 of
 * SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §10. Turns the raw
 * `install_chunk` lines into the §6.2 npm plan (Check requirements →
 * Download packages → Set up files → Run setup scripts → Check it's
 * installed) plus a per-line tone, so the dialog can show plain steps by
 * default and colour only real errors and warnings in Details.
 *
 * Pure and clock-injected: every method that depends on time takes `now`,
 * so tests replay recorded npm logs deterministically. Phase 3 moves this
 * to the backend; this stays as the fallback for old backends.
 *
 * Derivation is best-effort (§6.3). npm ≥ 7 extracts each tarball as it
 * downloads, so there is no line that marks "setup" starting. The download
 * step ends at the first non-download phase line, or after
 * `DOWNLOAD_IDLE_MS` with no new download line (see `tick`). A missed
 * pattern can make a step look instant but never makes the plan wrong.
 */

export type InstallStepStatus = "pending" | "active" | "done" | "failed" | "skipped";

export interface InstallStep {
    id: NpmStepId;
    label: string;
    status: InstallStepStatus;
    /** Right-aligned short fact, e.g. "189 packages". */
    hint?: string;
    /** One muted live line under the step, e.g. "Downloading chalk". */
    subline?: string;
}

export type NpmStepId = "requirements" | "download" | "setup" | "scripts" | "verify";

/** How Details should colour a line. Only `error`/`warning` get colour. */
export type LineTone = "normal" | "command" | "warning" | "error";

export type InstallErrorCategory =
    | "network"
    | "permission"
    | "disk"
    | "missing_prereq"
    | "not_on_path"
    | "cancelled"
    | "unknown";

export interface InstallFailure {
    category: InstallErrorCategory;
    /** Plain-language Layer 1 message. */
    message: string;
    /** 0-based index into the lines fed to `line()` of the first error line. */
    firstErrorLine?: number;
}

/** Quiet gap after the last download line before "Set up files" starts. */
export const DOWNLOAD_IDLE_MS = 1500;

const NPM_CODE_CATEGORY: Record<string, InstallErrorCategory> = {
    ENOTFOUND: "network",
    EAI_AGAIN: "network",
    ETIMEDOUT: "network",
    ECONNRESET: "network",
    ECONNREFUSED: "network",
    EACCES: "permission",
    EPERM: "permission",
    ENOSPC: "disk",
};

// `npm http fetch GET 200 <url> …` and `npm http cache <name>@<url> …`.
// Tarball URLs look like `<registry>/<name>/-/<file>.tgz`, where <name> may
// be scoped (`@scope/name`).
const HTTP_RE = /^npm http (?:fetch|cache)\b/;
const TARBALL_RE = /(https?:\/\/\S+?\/((?:@[^/\s]+\/)?[^/\s]+)\/-\/\S+?\.tgz)/;
const METADATA_RE = /^npm http (?:fetch \S+(?: \d+)?|cache) (https?:\/\/\S+\/((?:@|%40)?[^/\s]+))(?:\s|$)/;
// `npm info run <pkg>@<ver> <event> <path> <cmd>`; the completion line of
// the same script ends in `{ code: 0, signal: null }`.
const RUN_RE = /^npm info run (\S+)@\S+ (\S+) /;
const RUN_DONE_RE = /^npm info run \S+ \S+ \{ code:/;
const SUMMARY_RE = /^(?:added|changed|removed) (\d+) packages?\b|^up to date\b/;
const ERROR_LINE_RE = /^npm (?:error|ERR!)(?:\s|$)/;
const ERROR_CODE_RE = /^npm (?:error|ERR!) code (\S+)/;
const WARN_LINE_RE = /^npm (?:warn|WARN)(?:\s|$)/;

const PLAN: { id: NpmStepId; label: (name: string) => string }[] = [
    { id: "requirements", label: () => "Check requirements" },
    { id: "download", label: () => "Download packages" },
    { id: "setup", label: () => "Set up files" },
    { id: "scripts", label: () => "Run setup scripts" },
    { id: "verify", label: (name) => `Check ${name} is installed` },
];

export class NpmStepTracker {
    private steps: InstallStep[];
    private readonly name: string;
    private tarballs = new Set<string>();
    private lastDownloadAt = 0;
    private lineIndex = 0;
    private firstErrorLine: number | undefined;
    private npmErrorCode: string | undefined;

    constructor(name: string) {
        this.name = name;
        this.steps = PLAN.map((s) => ({ id: s.id, label: s.label(name), status: "pending" as const }));
    }

    /** A fresh copy, safe to hand to a signal. */
    snapshot(): InstallStep[] {
        return this.steps.map((s) => ({ ...s }));
    }

    /** Resets every step to pending and marks "Check requirements" active. */
    start(): void {
        this.steps = PLAN.map((s) => ({ id: s.id, label: s.label(this.name), status: "pending" as const }));
        this.tarballs.clear();
        this.lastDownloadAt = 0;
        this.lineIndex = 0;
        this.firstErrorLine = undefined;
        this.npmErrorCode = undefined;
        this.activate("requirements");
    }

    /** Classifies one output line, advances steps, and returns its tone. */
    line(text: string, now: number): LineTone {
        const index = this.lineIndex++;

        // The backend echoes `$ npm install …` before it spawns npm, so
        // the echo alone does not prove npm exists.
        if (text.startsWith("$ ")) return "command";

        // Any other output means npm started.
        this.complete("requirements");

        if (ERROR_LINE_RE.test(text)) {
            if (this.firstErrorLine === undefined) this.firstErrorLine = index;
            const code = ERROR_CODE_RE.exec(text)?.[1];
            if (code && !this.npmErrorCode) this.npmErrorCode = code;
            return "error";
        }
        if (WARN_LINE_RE.test(text)) return "warning";

        if (HTTP_RE.test(text)) {
            this.onDownload(text, now);
            return "normal";
        }

        const run = RUN_RE.exec(text);
        if (run && !RUN_DONE_RE.test(text)) {
            this.complete("download");
            this.complete("setup");
            this.activate("scripts");
            this.set("scripts", { subline: `Running ${run[2]} for ${run[1]}` });
            return "normal";
        }

        const summary = SUMMARY_RE.exec(text);
        if (summary) {
            if (summary[1]) this.set("download", { hint: `${summary[1]} packages` });
            this.onNpmFinished();
            return "normal";
        }
        if (text === "npm verbose exit 0" || text === "npm info ok") {
            this.onNpmFinished();
        }
        return "normal";
    }

    /** Call periodically while running: ends "Download packages" after a quiet gap. */
    tick(now: number): void {
        if (this.status("download") !== "active" || this.tarballs.size === 0) return;
        if (now - this.lastDownloadAt < DOWNLOAD_IDLE_MS) return;
        this.complete("download");
        this.activate("setup");
    }

    /** The install finished successfully: every step is done or skipped. */
    succeed(): void {
        for (const s of this.steps) {
            if (s.status === "done" || s.status === "skipped") continue;
            if (s.id === "scripts" && s.status === "pending") {
                this.set("scripts", { status: "skipped", hint: "none needed", subline: undefined });
            } else {
                this.set(s.id, { status: "done", subline: undefined });
            }
        }
    }

    /**
     * The install failed. Marks the step that was running as failed with a
     * plain-language message, and returns the failure for the dialog.
     * `error` is the `done` event's error: a free-text string or a typed
     * AgentMuxError object.
     */
    fail(error: unknown): InstallFailure {
        const text = typeof error === "string" ? error : "";
        let category: InstallErrorCategory;
        let failedStep: NpmStepId;
        let missingTool: string | undefined;

        if (text === "cancelled") {
            category = "cancelled";
            failedStep = this.activeStep() ?? "requirements";
        } else if (text.startsWith("spawn npm")) {
            category = "missing_prereq";
            missingTool = "npm";
            failedStep = "requirements";
        } else if (text.includes("reported success but")) {
            category = "not_on_path";
            failedStep = "verify";
        } else {
            category = (this.npmErrorCode && NPM_CODE_CATEGORY[this.npmErrorCode]) || "unknown";
            failedStep = this.activeStep() ?? "requirements";
        }

        const failedLabel = this.steps.find((s) => s.id === failedStep)?.label ?? "installing";
        const message = messageFor(category, this.name, failedLabel, missingTool);

        // Steps before the failed one finished; the failed one gets the
        // message; later ones stay pending.
        let reached = false;
        for (const s of this.steps) {
            if (s.id === failedStep) {
                reached = true;
                this.set(s.id, { status: "failed", subline: message });
            } else if (!reached && (s.status === "pending" || s.status === "active")) {
                this.set(s.id, { status: s.id === "scripts" ? "skipped" : "done", subline: undefined });
            } else if (reached && s.status === "active") {
                this.set(s.id, { status: "pending", subline: undefined });
            }
        }
        return { category, message, firstErrorLine: this.firstErrorLine };
    }

    private onDownload(text: string, now: number): void {
        if (this.status("download") === "pending") this.activate("download");
        if (this.status("download") !== "active") return;
        this.lastDownloadAt = now;
        const tarball = TARBALL_RE.exec(text);
        if (tarball) {
            this.tarballs.add(tarball[1]);
            this.set("download", {
                hint: `${this.tarballs.size} fetched`,
                subline: `Downloading ${decodeName(tarball[2])}`,
            });
            return;
        }
        const meta = METADATA_RE.exec(text);
        if (meta) this.set("download", { subline: `Looking up ${decodeName(meta[2])}` });
    }

    private onNpmFinished(): void {
        if (this.status("verify") !== "pending") return;
        this.complete("download");
        this.complete("setup");
        if (this.status("scripts") === "active") {
            this.complete("scripts");
        } else if (this.status("scripts") === "pending") {
            this.set("scripts", { status: "skipped", hint: "none needed" });
        }
        this.activate("verify");
    }

    private activeStep(): NpmStepId | undefined {
        return this.steps.find((s) => s.status === "active")?.id;
    }

    private status(id: NpmStepId): InstallStepStatus {
        return this.steps.find((s) => s.id === id)!.status;
    }

    /** Marks `id` active; every earlier unfinished step is completed first. */
    private activate(id: NpmStepId): void {
        for (const s of this.steps) {
            if (s.id === id) break;
            this.complete(s.id);
        }
        if (this.status(id) === "pending") this.set(id, { status: "active" });
    }

    /** Pending or active → done (scripts that never ran are left alone). */
    private complete(id: NpmStepId): void {
        const st = this.status(id);
        if (st === "active") {
            this.set(id, { status: "done", subline: undefined });
        } else if (st === "pending" && id !== "scripts") {
            this.set(id, { status: "done" });
        }
    }

    private set(id: NpmStepId, patch: Partial<InstallStep>): void {
        this.steps = this.steps.map((s) => (s.id === id ? { ...s, ...patch } : s));
    }
}

function decodeName(raw: string): string {
    try {
        return decodeURIComponent(raw);
    } catch {
        return raw;
    }
}

function messageFor(
    category: InstallErrorCategory,
    name: string,
    failedLabel: string,
    missingTool: string | undefined,
): string {
    switch (category) {
        case "network":
            return "Couldn't reach the package server.";
        case "permission":
            return "AgentMux wasn't allowed to write the files.";
        case "disk":
            return "The disk is full.";
        case "missing_prereq":
            return `${missingTool ?? "A required tool"} is needed first.`;
        case "not_on_path":
            return `Installed, but AgentMux can't find ${name} yet.`;
        case "cancelled":
            return "Cancelled. Nothing was left behind.";
        default:
            return `Something went wrong while running "${failedLabel}".`;
    }
}
