// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for release-gate.mjs: the decision (evaluateGate) and the reading of
// the five version locations (parseVersions). The GitHub side is a thin wrapper
// over `gh api` and is exercised by the first run of release-core.yml.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it, vi } from "vitest";
import { VERSION_FILES, evaluateGate, main, parseVersions, pickPushRun, suiteProofArgs } from "./release-gate.mjs";

const SHA = "0123456789abcdef0123456789abcdef01234567";
const VERSION = "0.60.3";

const releaseFiles = () => [
    ...VERSION_FILES.map((filename) => ({ status: "modified", filename })),
    { status: "removed", filename: ".changesets/1792962560-srv-runtime-stall-log-opz3.md" },
    { status: "removed", filename: ".changesets/1792965462-agent-pane-note-1zbv.md" },
];

const good = (over = {}) => ({
    refType: "tag",
    refName: `v${VERSION}`,
    sha: SHA,
    commit: { status: "behind", subject: `chore: release v${VERSION} (#5621)`, files: releaseFiles() },
    versions: Object.fromEntries(VERSION_FILES.map((f) => [f, VERSION])),
    pushRun: { status: "completed", conclusion: "success", url: "https://github.com/agentmuxai/agentmux/actions/runs/1" },
    ...over,
});

const withCommit = (over) => good({ commit: { ...good().commit, ...over } });

describe("pickPushRun", () => {
    const run = (over) => ({ head_sha: SHA, event: "push", run_number: 5, run_attempt: 1, status: "completed", conclusion: "success", html_url: "u", ...over });

    it("answers with the newest attempt of the commit's own push run", () => {
        const runs = [run({ run_attempt: 1, conclusion: "failure", html_url: "first" }), run({ run_attempt: 2, conclusion: "success", html_url: "second" })];
        expect(pickPushRun(runs, SHA)).toEqual({ status: "completed", conclusion: "success", url: "second" });
        expect(pickPushRun([...runs].reverse(), SHA).url).toBe("second");
    });

    it("ignores other commits' runs and pull request runs, and answers null with none", () => {
        const other = "f".repeat(40);
        expect(pickPushRun([run({ head_sha: other }), run({ event: "pull_request" })], SHA)).toBeNull();
        expect(pickPushRun([], SHA)).toBeNull();
        expect(pickPushRun(undefined, SHA)).toBeNull();
    });

    it("keeps an unfinished run unfinished (no conclusion)", () => {
        expect(pickPushRun([run({ status: "in_progress", conclusion: null })], SHA)).toEqual({ status: "in_progress", conclusion: null, url: "u" });
    });
});

describe("evaluateGate", () => {
    it("passes a chore: release commit on main whose files and versions are the release's", () => {
        const r = evaluateGate(good());
        expect(r).toEqual({ ok: true, version: VERSION, reasons: [] });
    });

    it("passes the commit that is main's tip, and a subject with no pull request number", () => {
        expect(evaluateGate(withCommit({ status: "identical" })).ok).toBe(true);
        expect(evaluateGate(withCommit({ subject: `chore: release v${VERSION}` })).ok).toBe(true);
    });

    it("passes the subject as agents write it: the PR-title prefix, then the title, then the PR number", () => {
        for (const subject of [`AgentY@narko: chore: release v${VERSION} (#5621)`, `Maricon-3@charlie: chore: release v${VERSION}`, `a.b_c@host-1.local: chore: release v${VERSION} (#1)`]) {
            expect(evaluateGate(withCommit({ subject })).ok, subject).toBe(true);
        }
    });

    it("passes a release that consumed no changesets at all", () => {
        expect(evaluateGate(withCommit({ files: VERSION_FILES.map((filename) => ({ status: "modified", filename })) })).ok).toBe(true);
    });

    // 0.60.28 (2026-11-14): the release commit's own push run was red when it was tagged.
    it("refuses a commit whose push run is red, and says which run", () => {
        for (const conclusion of ["failure", "timed_out", "startup_failure", "cancelled", "skipped", "action_required", null]) {
            const r = evaluateGate(good({ pushRun: { status: "completed", conclusion, url: "https://github.com/o/r/actions/runs/9" } }));
            expect(r.ok, String(conclusion)).toBe(false);
            expect(r.reasons.join(), String(conclusion)).toMatch(/push run of CI \(PR\) on main for this commit finished with/);
        }
        const failed = evaluateGate(good({ pushRun: { status: "completed", conclusion: "failure", url: "https://github.com/o/r/actions/runs/9" } }));
        expect(failed.reasons.join()).toContain("https://github.com/o/r/actions/runs/9");
    });

    it("refuses a commit whose push run has not finished, or does not exist, or could not be read", () => {
        for (const status of ["queued", "in_progress", "waiting", "pending", undefined]) {
            const r = evaluateGate(good({ pushRun: { status, conclusion: null, url: "u" } }));
            expect(r.ok, String(status)).toBe(false);
            expect(r.reasons.join()).toMatch(/has not finished/);
        }
        expect(evaluateGate(good({ pushRun: null })).reasons.join()).toMatch(/was not found/);
        expect(evaluateGate(good({ pushRun: undefined })).reasons.join()).toMatch(/could not be read/);
        const noRun = good();
        delete noRun.pushRun;
        expect(evaluateGate(noRun).ok).toBe(false);
    });

    it("passes only a finished, successful push run, and still reports every other reason too", () => {
        expect(evaluateGate(good({ pushRun: { status: "completed", conclusion: "success", url: null } })).ok).toBe(true);
        const r = evaluateGate(withCommit({ status: "ahead" }));
        expect(r.ok).toBe(false);
        const both = evaluateGate({ ...withCommit({ status: "ahead" }), pushRun: null });
        expect(both.reasons.length).toBe(r.reasons.length + 1);
    });

    it("refuses a branch run, or a ref that is not a release tag", () => {
        expect(evaluateGate(good({ refType: "branch" })).reasons.join()).toMatch(/not a tag/);
        for (const refName of ["main", "v0.60", "v0.60.3-rc1", "0.60.3", "v0.60.3.1", "vx.y.z", ""]) {
            expect(evaluateGate(good({ refType: "tag", refName })).ok, refName).toBe(false);
        }
    });

    it("refuses a sha that is not a full commit hash", () => {
        expect(evaluateGate(good({ sha: "abc123" })).reasons.join()).toMatch(/not a full commit sha/);
    });

    it("refuses a commit that is not on main", () => {
        for (const status of ["ahead", "diverged", undefined]) {
            expect(evaluateGate(withCommit({ status })).reasons.join(), String(status)).toMatch(/not on main/);
        }
    });

    it("refuses a subject that is not exactly the release's", () => {
        const bad = [
            "chore: release-anything",
            `chore: release v${VERSION} extra`,
            `chore: release v${VERSION}.1`,
            "chore: release v0.60.4",
            `chore: release v${VERSION.replace(/\./g, "x")}`, // dots are literal, not wildcards
            `feat: something (#5621)\n\nchore: release v${VERSION}`,
            `chore: release v${VERSION} (#)`,
            `Chore: release v${VERSION}`,
            `AgentY narko: chore: release v${VERSION}`, // a prefix is <agent>@<host>: and nothing else
            `AgentY@narko:chore: release v${VERSION}`,
            `x: AgentY@narko: chore: release v${VERSION}`,
            `AgentY@narko: feat: chore: release v${VERSION}`,
            "",
        ];
        for (const subject of bad) {
            expect(evaluateGate(withCommit({ subject })).ok, subject).toBe(false);
        }
    });

    it("refuses a commit that changes anything but the release files", () => {
        const extras = [
            { status: "modified", filename: "scripts/release-manifest.mjs" },
            { status: "modified", filename: ".github/workflows/release-core.yml" },
            { status: "added", filename: "src/x.rs" },
            { status: "added", filename: ".changesets/new.md" },
            { status: "modified", filename: ".changesets/1792962560-edit.md" },
            { status: "removed", filename: ".changesets/README.md" },
            { status: "removed", filename: ".changesets/sub/dir.md" },
            { status: "removed", filename: "Cargo.toml" },
            { status: "renamed", filename: "package.json" },
        ];
        for (const extra of extras) {
            const r = evaluateGate(withCommit({ files: [...releaseFiles(), extra] }));
            expect(r.ok, JSON.stringify(extra)).toBe(false);
            expect(r.reasons.join()).toContain(extra.filename);
        }
    });

    it("refuses a commit that does not modify all five version files", () => {
        for (const name of VERSION_FILES) {
            const files = releaseFiles().filter((f) => f.filename !== name);
            const r = evaluateGate(withCommit({ files }));
            expect(r.reasons.join(), name).toContain(`does not modify ${name}`);
        }
    });

    it("refuses when any of the five locations is at another version", () => {
        for (const name of VERSION_FILES) {
            const versions = { ...good().versions, [name]: "0.60.2" };
            const r = evaluateGate(good({ versions }));
            expect(r.ok, name).toBe(false);
            expect(r.reasons.join()).toContain(`${name} is at version '0.60.2'`);
        }
    });

    it("reports every reason, not only the first, and refuses when the commit could not be read", () => {
        const r = evaluateGate(good({ refType: "branch", commit: undefined, versions: undefined }));
        expect(r.ok).toBe(false);
        expect(r.reasons.join()).toMatch(/not a tag.*could not be read/);
        const many = evaluateGate(withCommit({ status: "ahead", subject: "x", files: [{ status: "added", filename: "a" }] }));
        expect(many.reasons.length).toBeGreaterThanOrEqual(8);
    });
});

describe("parseVersions", () => {
    const texts = {
        "package.json": JSON.stringify({ name: "agentmux", version: "0.60.3" }),
        "package-lock.json": JSON.stringify({ name: "agentmux", version: "0.60.3", lockfileVersion: 3 }),
        "Cargo.toml": '[workspace]\nmembers = ["a"]\n\n[workspace.package]\nversion = "0.60.3"\nedition = "2021"\n',
        "VERSION_HISTORY.md": "# Version History\n\n## 0.60.3 - 2026-10-27\n\n- x\n\n## 0.60.2 - 2026-10-26\n",
        "Cargo.lock": 'name = "other"\nversion = "9.9.9"\n\n[[package]]\nname = "agentmux-cef"\nversion = "0.60.3"\ndependencies = []\n',
    };

    it("reads the five locations the way the old verify job did", () => {
        expect(parseVersions(texts)).toEqual({
            "package.json": "0.60.3",
            "package-lock.json": "0.60.3",
            "Cargo.toml": "0.60.3",
            "VERSION_HISTORY.md": "0.60.3",
            "Cargo.lock": "0.60.3",
        });
    });

    it("leaves a location undefined (and so a mismatch) when it cannot be found", () => {
        const v = parseVersions({ ...texts, "Cargo.lock": 'name = "other"\nversion = "1.0.0"\n', "VERSION_HISTORY.md": "no headings\n" });
        expect(v["Cargo.lock"]).toBeUndefined();
        expect(v["VERSION_HISTORY.md"]).toBeUndefined();
        const r = evaluateGate(good({ versions: v }));
        expect(r.ok).toBe(false);
    });
});

// ── check 6 when the release proves the suite itself ─────────────────────────

const NO_PUSH_RUN = { pushRun: null };

describe("evaluateGate: the release's own suite (check 6)", () => {
    it("without any flag, no push run refuses: a run outside release-core.yml keeps the push-run check", () => {
        const r = evaluateGate(good(NO_PUSH_RUN));
        expect(r.ok).toBe(false);
        expect(r.reasons.join()).toMatch(/was not found/);
        expect(evaluateGate(good({ pushRun: undefined })).ok).toBe(false);
    });

    it("the suite's `success` stands in for the push run, with none or a red one", () => {
        expect(evaluateGate(good({ ...NO_PUSH_RUN, suiteResult: "success" })).ok).toBe(true);
        const red = { status: "completed", conclusion: "failure", url: "u" };
        expect(evaluateGate(good({ pushRun: red, suiteResult: "success" })).ok).toBe(true);
    });

    it.each(["failure", "cancelled", "skipped", ""])("a suite result of '%s' refuses, whatever the push run says", (suiteResult) => {
        const green = { status: "completed", conclusion: "success", url: "u" };
        for (const pushRun of [null, green]) {
            const r = evaluateGate(good({ pushRun, suiteResult }));
            expect(r.ok, `${suiteResult} with ${pushRun ? "a green push run" : "no push run"}`).toBe(false);
            expect(r.reasons.join()).toMatch(/own test suite finished/);
        }
    });

    it("`checked later` skips only check 6, and every other check still refuses", () => {
        expect(evaluateGate(good({ ...NO_PUSH_RUN, suiteCheckedLater: true })).ok).toBe(true);
        const r = evaluateGate({ ...withCommit({ status: "ahead" }), pushRun: null, suiteCheckedLater: true });
        expect(r.ok).toBe(false);
        expect(r.reasons.join()).toMatch(/not on main/);
    });

    it("the suite's success does not excuse a commit that is not a release commit", () => {
        const r = evaluateGate({ ...withCommit({ subject: "feat: something" }), pushRun: null, suiteResult: "success" });
        expect(r.ok).toBe(false);
        expect(r.reasons.join()).toMatch(/is not 'chore: release/);
    });
});

describe("release-gate command line (check 6 options)", () => {
    const base = ["--ref-type", "tag", "--ref-name", `v${VERSION}`, "--sha", SHA, "--repo", "agentmuxai/agentmux"];
    const quiet = (argv) => {
        const spy = vi.spyOn(console, "error").mockImplementation(() => {});
        try {
            return main(argv);
        } finally {
            spy.mockRestore();
        }
    };

    it.each([["bogus"], [""], ["Success"], ["SUCCESS"]])("--suite-result '%s' is a usage error, before anything is read", (value) => {
        expect(quiet([...base, "--suite-result", value])).toBe(2);
    });

    it("--suite-result with --suite-checked-later, and --suite-proof with either, are usage errors", () => {
        expect(quiet([...base, "--suite-result", "success", "--suite-checked-later"])).toBe(2);
        expect(quiet([...base, "--suite-proof", "later", "--suite-result", "success"])).toBe(2);
        expect(quiet([...base, "--suite-proof", "success", "--suite-checked-later"])).toBe(2);
    });

    it("--suite-proof maps what a reusable workflow passes: blank is no proof, `later` and a result are themselves", () => {
        expect(suiteProofArgs(undefined)).toBeNull();
        expect(suiteProofArgs("")).toEqual({ suiteResult: undefined, suiteCheckedLater: false });
        expect(suiteProofArgs("later")).toEqual({ suiteResult: undefined, suiteCheckedLater: true });
        expect(suiteProofArgs("success")).toEqual({ suiteResult: "success", suiteCheckedLater: false });
        expect(suiteProofArgs("skipped")).toEqual({ suiteResult: "skipped", suiteCheckedLater: false });
        // Not a result: a usage error in main(), never a pass.
        expect(quiet([...base, "--suite-proof", "yes"])).toBe(2);
    });
});

// ── release-core.yml: what is behind the suite, and that the suite is ci-pr.yml's ──

const workflow = (name) => readFileSync(fileURLToPath(new URL(`../.github/workflows/${name}`, import.meta.url)), "utf8").replace(/\r\n/g, "\n");

/** The text of each job of a workflow, by id: from `  id:` to the next job. */
function jobsOf(text) {
    const lines = text.split("\n");
    const start = lines.findIndex((l) => l === "jobs:");
    const jobs = {};
    let id = null;
    for (const line of lines.slice(start + 1)) {
        const m = /^ {2}([a-z0-9-]+):\s*$/.exec(line);
        if (m) {
            id = m[1];
            jobs[id] = [];
        } else if (id) {
            jobs[id].push(line);
        }
    }
    return Object.fromEntries(Object.entries(jobs).map(([k, v]) => [k, v.join("\n")]));
}

/** The job ids a job needs: `needs: x` or `needs: [x, y]`. */
function needsOf(job) {
    const m = /^ {4}needs:\s*(.+)$/m.exec(job);
    if (!m) return [];
    return m[1]
        .replace(/[[\]]/g, "")
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean);
}

/** The `run:` of the step with this name, comments and blank lines dropped, each line trimmed. */
function runOf(job, stepName) {
    const lines = job.split("\n");
    const at = lines.findIndex((l) => l.trim() === `- name: ${stepName}`);
    if (at < 0) return undefined;
    const indent = lines[at].indexOf("- name");
    for (let i = at + 1; i < lines.length; i++) {
        const l = lines[i];
        if (l.trim().startsWith("- ") && l.indexOf("- ") === indent) return undefined;
        const m = /^\s+run:\s*(.*)$/.exec(l);
        if (!m) continue;
        if (m[1] && m[1] !== "|") return m[1].trim();
        const body = [];
        for (let j = i + 1; j < lines.length; j++) {
            if (lines[j].trim() !== "" && lines[j].search(/\S/) <= l.search(/\S/)) break;
            body.push(lines[j]);
        }
        return body
            .map((x) => x.trim())
            .filter((x) => x && !x.startsWith("#"))
            .join("\n");
    }
    return undefined;
}

describe("release-core.yml: the suite", () => {
    const core = jobsOf(workflow("release-core.yml"));
    const reachable = (id, seen = new Set()) => {
        for (const n of needsOf(core[id] ?? "")) {
            if (!seen.has(n)) {
                seen.add(n);
                reachable(n, seen);
            }
        }
        return seen;
    };

    it("every job that sends anything out of the run is behind the suite, directly or through a job that is", () => {
        for (const id of ["container-image", "sign-release", "publish", "publish-linux-repo", "winget", "ms-store", "linux-upgrade-check"]) {
            expect(core[id], `${id} exists`).toBeDefined();
            expect(reachable(id).has("suite"), `${id} needs the suite`).toBe(true);
        }
    });

    it("sign-release and publish need the suite's result to be `success`, and the signing gate passes it", () => {
        for (const id of ["sign-release", "publish"]) expect(core[id], id).toMatch(/needs\.suite\.result == 'success'/);
        expect(core["sign-release"]).toMatch(/--suite-result "\$SUITE_RESULT"/);
        expect(core["sign-release"]).toMatch(/SUITE_RESULT: \$\{\{ needs\.suite\.result \}\}/);
    });

    it("only the four builds run beside the suite (`later`); the registry push and the repository pass its literal result", () => {
        const later = Object.keys(core).filter((id) => /suite-proof: later/.test(core[id]));
        expect(later.sort()).toEqual(["build-linux", "build-macos", "build-server-package", "build-windows"]);
        for (const id of later) expect(reachable(id).has("suite"), `${id} must not wait for the suite`).toBe(false);
        for (const id of ["container-image", "publish-linux-repo"]) {
            expect(core[id], id).toMatch(/suite-proof: \$\{\{ needs\.suite\.result \}\}/);
        }
    });

    it("the first gate is the only gate call of release-core.yml that leaves check 6 to the suite", () => {
        expect(core.gate).toMatch(/--suite-checked-later/);
        for (const id of Object.keys(core).filter((k) => k !== "gate")) {
            expect(core[id], id).not.toMatch(/--suite-checked-later/);
        }
    });

    it("a workflow that can start on its own passes check 6 nothing unless release-core.yml gave it a proof", () => {
        for (const name of ["build-linux.yml", "build-windows.yml", "build-macos.yml", "build-server-package.yml", "container-image.yml", "publish-linux-repo.yml"]) {
            const text = workflow(name);
            expect(text, name).toMatch(/--suite-proof "\$SUITE_PROOF"/);
            expect(text, name).toMatch(/SUITE_PROOF: \$\{\{ inputs\.suite-proof \}\}/);
            expect(text, name).not.toMatch(/--suite-(result|checked-later)/);
            // The input is blank unless the caller sets it.
            expect(text, name).toMatch(/suite-proof:\n(?: {8}.*\n)*? {8}default: ""/);
        }
    });

    it("the suite's commands are the ones of ci-pr.yml's `rust` job", () => {
        const ci = jobsOf(workflow("ci-pr.yml")).rust;
        const suite = core.suite;
        const steps = [
            "Free disk and add swap (Linux)",
            "Install build deps (Linux — launcher Wayland toolkit + agentmux-mcp's xcap)",
            "cargo check --tests (CEF-free crates)",
            "A shipped build never has the test-only feature",
            "cargo nextest (CEF-free crates)",
            "cargo test --doc (CEF-free crates)",
            "RPC bindings are current",
            "ts-rs parses every serde attribute",
        ];
        for (const name of steps) {
            const theirs = runOf(ci, name);
            expect(theirs, `ci-pr.yml has '${name}'`).toBeDefined();
            expect(runOf(suite, name), `release-core.yml's suite runs the same '${name}'`).toBe(theirs);
        }
        // The toolchain the suite builds with is the PR's.
        const pin = (job) => /dtolnay\/rust-toolchain@([\w.]+)/.exec(job)?.[1];
        expect(pin(suite)).toBe(pin(ci));
    });

    it("the suite checks out the commit the run is for, never the tag's name, and needs only the gate", () => {
        expect(core.suite).toMatch(/ref: \$\{\{ github\.sha \}\}/);
        expect(needsOf(core.suite)).toEqual(["gate"]);
    });
});
