// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// build-server-package.yml signs the server package's macOS programs in one job, `sign-macos`, which holds the identity
// token the Apple role accepts. The programs come from the `package` job, which runs the built commit's code, and go on to
// `pack-macos`, which runs the repository's packing script. Like build-macos.yml's signing job (see
// workflow-macos-signing-split.test.mjs), `sign-macos` runs in the `apple-signing` environment, checks what it receives
// against a digest that came by job output before anything reads it, holds nothing until then, and calls its tools by
// absolute path. This reads the jobs and steps by indentation and fails when a change loosens any of that.
import { readFileSync } from "node:fs";
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
        expect(text).not.toMatch(/(^|[\s(;|&`])(security|codesign|xcrun|rm|mv|uuidgen|ditto|shasum)\s+[-\w"$]/m);
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

describe("build-server-package.yml: a dispatch can ask for signing, on main only, and uploads nothing to a release", () => {
    it("has a boolean `sign` input on the dispatch, off by default", () => {
        expect(source).toMatch(/workflow_dispatch:[\s\S]*?\n {6}sign:\n[\s\S]*?type: boolean\n {8}default: false/);
    });

    it("the plan signs a dispatch only for a run on main building ref main, and a call only for main or a tag at its own commit", () => {
        const plan = stepsOf("plan").find((s) => /id: sign/.test(s));
        expect(plan).toMatch(/EVENT: \$\{\{ github\.event_name \}\}/);
        expect(plan).toMatch(/\[ "\$EVENT" = workflow_dispatch \]/);
        expect(plan).toMatch(/\[ "\$RUN_REF" = refs\/heads\/main \] && \[ "\$BUILD_REF" = main \]/);
        expect(plan).toMatch(/\[ "\$BUILD_REF" = "\$RUN_SHA" \]/);
    });

    it("the signing job refuses a dispatch that is not on main, whatever the plan said", () => {
        const identity = stepsOf("sign-macos").find((s) => /The identity token names build-server-package\.yml on main/.test(s));
        expect(identity).toMatch(/event_name\)" = workflow_dispatch \] && \[ "\$REF" != refs\/heads\/main \]/);
    });

    it("never uploads to a release", () => {
        expect(code(source)).not.toMatch(/gh release|release upload|release create|softprops\/action-gh-release/);
        expect(code(source)).not.toMatch(/contents: write/);
    });
});
