// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// build-server-package.yml signs the server package's macOS programs in one job, `sign-macos`, which holds the identity
// token the Apple role accepts. The programs come from the `package` job, which runs the built commit's code, and go on to
// `pack-macos`, which runs the repository's packing script. Like build-macos.yml's signing job (see
// workflow-macos-signing-split.test.mjs), `sign-macos` runs in the `apple-signing` environment, checks what it receives
// against a digest that came by job output before anything reads it, holds nothing until then, and calls its tools by
// absolute path. This reads the jobs and steps by indentation and fails when a change loosens any of that.
import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const source = readFileSync(join(ROOT, ".github/workflows/build-server-package.yml"), "utf8").replace(/\r\n/g, "\n");

function jobText(job) {
    const out = [];
    let inJob = false;
    let inJobs = false;
    for (const line of source.split("\n")) {
        if (/^jobs:\s*$/.test(line)) {
            inJobs = true;
            continue;
        }
        if (!inJobs) continue;
        if (/^ {2}[A-Za-z0-9_-]+:\s*$/.test(line)) inJob = line.trim() === `${job}:`;
        if (inJob) out.push(line);
    }
    return out.join("\n");
}

function stepsOf(job) {
    const steps = [];
    let inSteps = false;
    for (const line of jobText(job).split("\n")) {
        if (/^ {4}steps:\s*$/.test(line)) {
            inSteps = true;
            continue;
        }
        if (!inSteps) continue;
        if (/^ {6}- /.test(line)) steps.push([line]);
        else if (steps.length > 0) steps.at(-1).push(line);
    }
    return steps.map((s) => s.join("\n"));
}

const code = (text) =>
    text
        .split("\n")
        .filter((l) => !/^\s*#/.test(l))
        .join("\n");

const names = (job) => stepsOf(job).map((s) => (/- name: (.*)/.exec(s) ?? [])[1] ?? (/- uses: (\S+)/.exec(s) ?? [])[1] ?? "");

describe("build-server-package.yml: the signing job", () => {
    it("is the only job with the identity token and the only one in an environment", () => {
        expect(source.match(/id-token: write/g)).toHaveLength(1);
        expect(jobText("sign-macos")).toMatch(/id-token: write/);
        expect(source.match(/^ {4}environment:/gm)).toHaveLength(1);
        expect(jobText("sign-macos")).toMatch(/^ {4}environment: apple-signing\s*$/m);
    });

    it("accepts the apple-signing environment's token, and only main and v* tags", () => {
        const text = code(jobText("sign-macos"));
        expect(text).toMatch(/repo:\$\{GITHUB_REPOSITORY\}:environment:apple-signing/);
        expect(text).toMatch(/refs\/heads\/main\|refs\/tags\/v\*/);
        expect(text).not.toMatch(/ref:refs\/heads\/main"\|/);
    });

    it("checks the programs against the build job's digest before the identity token, the role, the secret and the keychain", () => {
        const n = names("sign-macos");
        const at = (re) => n.findIndex((x) => re.test(x));
        const digest = at(/^The programs are the ones the build job made/);
        expect(digest).toBeGreaterThan(-1);
        for (const later of [at(/^The identity token/), at(/configure-aws-credentials/), at(/^Fetch the Apple signing secrets/), at(/^Import the Developer ID/)]) {
            expect(later).toBeGreaterThan(digest);
        }
        const step = stepsOf("sign-macos")[digest];
        expect(step).toMatch(/WANT: \$\{\{ needs\.package\.outputs\[format\('programs_\{0\}_\{1\}', matrix\.os, matrix\.arch\)\] \}\}/);
        expect(step).toMatch(/\[ "\$\(programs_digest programs\)" = "\$WANT" \]/);
    });

    it("has no repository code: no checkout, no repository script, no package manager", () => {
        const text = code(jobText("sign-macos"));
        expect(text).not.toMatch(/actions\/checkout/);
        expect(text).not.toMatch(/\bbash scripts\/|\bnode scripts\/|\bnpm\b|\bcargo\b/);
    });

    it("calls its tools by absolute path", () => {
        const text = code(jobText("sign-macos"));
        expect(text).not.toMatch(/(^|[\s(;|&`])(security|codesign|xcrun|rm|mv|uuidgen|ditto|shasum|jq|curl|awk|base64|chmod|grep|mkdir|cut|find|wc|tr)\s+[-+\w"'$]/m);
    });

    it("pins every third-party action by commit", () => {
        for (const step of stepsOf("sign-macos")) {
            const uses = /^\s*(?:- )?uses: (\S+)/m.exec(step);
            if (uses) expect(uses[1], uses[1]).toMatch(/@[0-9a-f]{40}$/);
        }
    });
});

describe("build-server-package.yml: the hand-offs are bound by digest", () => {
    it("`package` reports one digest per macOS leg, from the step that names it", () => {
        const text = jobText("package");
        expect(text).toMatch(/programs_macos_arm64: \$\{\{ steps\.programs_digest\.outputs\.programs_macos_arm64 \}\}/);
        expect(text).toMatch(/programs_macos_x64: \$\{\{ steps\.programs_digest\.outputs\.programs_macos_x64 \}\}/);
        const step = stepsOf("package").find((s) => /id: programs_digest/.test(s));
        expect(step).toMatch(/programs_\$\{LEG\}=\$\(programs_digest target\/fast-release\)/);
        // the digest is named before the programs are uploaded
        const n = names("package");
        expect(n.findIndex((x) => /^Name the digest of the unsigned programs/.test(x))).toBeLessThan(
            n.findIndex((x) => /^Keep the unsigned programs/.test(x)),
        );
    });

    it("`sign-macos` reports the signed programs' digest after notarization, and `pack-macos` checks it before it packs", () => {
        expect(jobText("sign-macos")).toMatch(/signed_arm64: \$\{\{ steps\.signed\.outputs\.signed_arm64 \}\}/);
        const n = names("sign-macos");
        const notarize = n.findIndex((x) => /^Notarize the programs/.test(x));
        const signed = n.findIndex((x) => /^Name the digest of the signed programs/.test(x));
        const upload = n.findIndex((x) => /upload-artifact/.test(x));
        expect(notarize).toBeGreaterThan(-1);
        expect(signed).toBeGreaterThan(notarize);
        expect(upload).toBeGreaterThan(signed);
        const pack = names("pack-macos");
        const check = pack.findIndex((x) => /^The programs are the ones the signing job made/.test(x));
        expect(check).toBeGreaterThan(-1);
        expect(check).toBeLessThan(pack.findIndex((x) => /^Pack/.test(x)));
        expect(stepsOf("pack-macos")[check]).toMatch(/WANT: \$\{\{ needs\.sign-macos\.outputs\[format\('signed_\{0\}', matrix\.arch\)\] \}\}/);
    });

    it("hashes each file on stdin, as the repository's hash gate asks", () => {
        for (const line of source.split("\n")) {
            if (/shasum -a 256 /.test(line) && !/^\s*#/.test(line)) expect(line, line).toMatch(/shasum -a 256 (< |\|)|shasum -a 256$/);
        }
    });
});

describe("build-server-package.yml: what a signing run builds", () => {
    it("the plan names the commit of the run for a run that signs, and the checkouts take it from the plan", () => {
        const plan = stepsOf("plan").find((s) => /id: sign/.test(s));
        expect(plan).toMatch(/if \[ "\$sign" = true \]; then build_ref="\$RUN_SHA"; else build_ref="\$\{BUILD_REF:-\$RUN_SHA\}"; fi/);
        expect(plan).toMatch(/echo "build_ref=\$build_ref" >> "\$GITHUB_OUTPUT"/);
        expect(jobText("plan")).toMatch(/build_ref: \$\{\{ steps\.sign\.outputs\.build_ref \}\}/);
        for (const job of ["package", "pack-macos"]) {
            const checkout = stepsOf(job).find((s) => /actions\/checkout/.test(s));
            expect(checkout, job).toMatch(/ref: \$\{\{ needs\.plan\.outputs\.build_ref \}\}/);
        }
        expect(code(source)).not.toMatch(/ref: \$\{\{ inputs\.ref/);
    });
});

describe("build-server-package.yml: a dispatch can ask for signing, on main only, and uploads nothing to a release", () => {
    it("has a boolean `sign` input on the dispatch, off by default", () => {
        expect(source).toMatch(/workflow_dispatch:[\s\S]*?\n {6}sign:\n[\s\S]*?type: boolean\n {8}default: false/);
    });

    it("the plan signs a dispatch only for a run on main building ref main, and a call only for main or a tag at its own commit", () => {
        const plan = stepsOf("plan").find((s) => /id: sign/.test(s));
        // a call from release.yml's own dispatch has the caller's event: the run of this file itself is told by workflow_ref
        expect(plan).toMatch(/WORKFLOW_REF: \$\{\{ github\.workflow_ref \}\}/);
        expect(plan).toMatch(/"\$REPO"\/\.github\/workflows\/build-server-package\.yml@\*\) direct=true/);
        expect(plan).toMatch(/\[ "\$direct" = true \]/);
        expect(code(plan)).not.toMatch(/event_name|EVENT/);
        expect(plan).toMatch(/\[ "\$RUN_REF" = refs\/heads\/main \] && \[ "\$BUILD_REF" = main \]/);
        expect(plan).toMatch(/\[ "\$BUILD_REF" = "\$RUN_SHA" \]/);
    });

    it("the signing job refuses a dispatch that is not on main, whatever the plan said", () => {
        const identity = stepsOf("sign-macos").find((s) => /The identity token names build-server-package\.yml on main/.test(s));
        expect(identity).toMatch(/jq -r \.workflow_ref\)" in\n\s+"\$\{GITHUB_REPOSITORY\}"\/\.github\/workflows\/build-server-package\.yml@\*\)\n\s+\[ "\$REF" = refs\/heads\/main \] \|\|/);
        expect(code(identity)).not.toMatch(/\.event_name/);
    });

    it("never uploads to a release", () => {
        expect(code(source)).not.toMatch(/gh release|release upload|release create|softprops\/action-gh-release/);
        expect(code(source)).not.toMatch(/contents: write/);
    });
});

// The step that checks the packed programs runs here with stand-ins for codesign, spctl and sleep: what it does when Apple's
// answer is "notarized", "rejected", or only comes late.
describe("build-server-package.yml: the packed programs must read as notarized", () => {
    const step = stepsOf("pack-macos").find((x) => /Check the signatures of the packed programs/.test(x));
    const script = step
        .split("\n")
        .slice(step.split("\n").findIndex((l) => /^\s+run: \|/.test(l)) + 1)
        .map((l) => l.replace(/^ {10}/, ""))
        .join("\n");

    // `spctl` answers by the file name; `late` answers "rejected" the first `n` times it is asked about a program
    function run(spctlBody) {
        const dir = mkdtempSync(join(tmpdir(), "pack-check-"));
        const bin = join(dir, "stubs");
        mkdirSync(bin);
        const stub = (name, body) => {
            writeFileSync(join(bin, name), `#!/bin/bash\n${body}\n`);
            chmodSync(join(bin, name), 0o755);
        };
        stub("codesign", `case "$1" in -dvv) printf 'Identifier=x\\nAuthority=Developer ID Application: T\\nTeamIdentifier=ABCDE12345\\nCodeDirectory v=20500 flags=0x10000(runtime)\\n' >&2 ;; esac`);
        stub("sleep", "true");
        stub("spctl", spctlBody);
        mkdirSync(join(dir, "unpacked/pkg/bin"), { recursive: true });
        for (const p of ["agentmux", "agentmux-srv", "agentmux-mcp", "agentmux-bashwrap"]) writeFileSync(join(dir, "unpacked/pkg/bin", p), "x");
        const r = spawnSync("bash", ["-c", script], { cwd: dir, env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, COUNT_DIR: dir }, encoding: "utf8" });
        return { status: r.status, out: `${r.stdout}${r.stderr}` };
    }

    it("asks the assessment that works for a bare program, not the one for app bundles", () => {
        expect(code(step)).toMatch(/spctl -a -vvv -t open --context context:primary-signature "\$f"/);
        expect(code(step)).not.toMatch(/-t exec/);
    });

    it("passes when every program reads as a notarized Developer ID program", () => {
        const r = run(`echo "$@ : accepted" ; echo "source=Notarized Developer ID"`);
        expect(r.status, r.out).toBe(0);
    });

    it("fails when a program never reads as notarized (rejected, or Apple not reachable)", () => {
        const r = run(`echo "$*: rejected" >&2; exit 3`);
        expect(r.status, r.out).toBe(1);
        expect(r.out).toMatch(/::error::Gatekeeper did not report agentmux as a notarized Developer ID program after five tries/);
    });

    it("fails when only one of the four is not notarized", () => {
        const r = run(`case "$*" in *agentmux-mcp) echo "rejected" >&2; exit 3 ;; *) echo "source=Notarized Developer ID" ;; esac`);
        expect(r.status, r.out).toBe(1);
        expect(r.out).toMatch(/agentmux-mcp as a notarized/);
    });

    it("waits for a ticket that comes late: the fourth try is enough", () => {
        const r = run(`f="$COUNT_DIR/n"; n=$(cat "$f" 2>/dev/null || echo 0); n=$((n+1)); echo $n > "$f"; if [ $n -lt 4 ]; then echo rejected >&2; exit 3; fi; echo "source=Notarized Developer ID"; rm -f "$f"`);
        expect(r.status, r.out).toBe(0);
    });
});
