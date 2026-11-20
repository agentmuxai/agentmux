// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// build-macos.yml builds the app in a job that runs the commit's own scripts and its dependencies' scripts, and
// signs it in a job that holds the Apple credentials. The build job must hold none of them; the signing job must run
// nothing that came with the built commit; and the DMG the release picks up must be the checked, stapled one. This
// reads the workflow's jobs and steps by indentation (as workflow-main-only-gate.test.mjs does) and fails when a
// change puts a credential, a checkout's script or an unchecked DMG where it must not be.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const source = readFileSync(join(ROOT, ".github/workflows/build-macos.yml"), "utf8").replace(/\r\n/g, "\n");

/** The text of one job: from its `  name:` line to the next job. */
function jobText(job) {
    const lines = source.split("\n");
    const out = [];
    let inJob = false;
    let inJobs = false;
    for (const line of lines) {
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

/** The steps of one job: each is the text from its `      - ` line to the next one. */
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

/** The comment-free text: a rule about what a job runs is not broken by a comment that names it. */
const code = (text) =>
    text
        .split("\n")
        .filter((l) => !/^\s*#/.test(l))
        .join("\n");

/** The job's own `permissions:` block as { scope: level }. */
function permissionsOf(job) {
    const perms = {};
    let inBlock = false;
    for (const line of jobText(job).split("\n")) {
        if (/^ {4}permissions:\s*$/.test(line)) {
            inBlock = true;
            continue;
        }
        if (inBlock) {
            const m = /^ {6}([a-z-]+):\s*([a-z]+)/.exec(line);
            if (m) perms[m[1]] = m[2];
            else if (!/^\s*(#.*)?$/.test(line)) inBlock = false;
        }
    }
    return perms;
}

const JOBS = ["gate", "build", "sign", "publish"];
// Anything that is, or reads, an Apple signing credential or the keychain that holds one.
const CREDENTIAL = /configure-aws-credentials|APPLE_|apple-signing\.json|\bsecurity (create|import|unlock|default|find)-|notarytool|codesign|KEYCHAIN/;

describe("build-macos.yml jobs", () => {
    it("has the four jobs and no other", () => {
        const names = [...source.matchAll(/^ {2}([A-Za-z0-9_-]+):\s*$/gm)].map((m) => m[1]);
        // `inputs:` under workflow_call is at the same indent: keep only the keys that are jobs.
        expect(names.filter((n) => jobText(n).includes("runs-on:"))).toEqual(JOBS);
    });

    it("gives no job a grant it did not name: the workflow's own grant is empty", () => {
        expect(source).toMatch(/^permissions: \{\}\s*$/m);
        for (const job of JOBS) expect(Object.keys(permissionsOf(job)).length).toBeGreaterThan(0);
    });

    it("lets only `build` (the broker) and `sign` ask for an identity token", () => {
        expect(permissionsOf("gate")["id-token"]).toBeUndefined();
        expect(permissionsOf("publish")["id-token"]).toBeUndefined();
        expect(permissionsOf("build")["id-token"]).toBe("write");
        expect(permissionsOf("sign")["id-token"]).toBe("write");
    });

    it("gives contents:write to `publish` alone", () => {
        for (const job of ["gate", "build", "sign"]) expect(permissionsOf(job).contents).toBe("read");
        expect(permissionsOf("publish")).toEqual({ contents: "write" });
    });

    it("puts only `sign` in the apple-signing environment", () => {
        expect(jobText("sign")).toMatch(/^ {4}environment: apple-signing\s*$/m);
        for (const job of ["gate", "build", "publish"]) expect(jobText(job)).not.toMatch(/^ {4}environment:/m);
    });
});

describe("the gate", () => {
    it("is the first job: build and sign wait for it, and nothing in it asks the broker or the Apple role", () => {
        expect(jobText("build")).toMatch(/^ {4}needs: gate\s*$/m);
        expect(jobText("sign")).toMatch(/^ {4}needs: \[gate, build\]\s*$/m);
        expect(code(jobText("gate"))).not.toMatch(/actions-broker|configure-aws-credentials|ACTIONS_ID_TOKEN/);
    });

    it("runs the main-only check before it names the commit", () => {
        const steps = stepsOf("gate");
        const gate = steps.findIndex((s) => /name: The code to be signed is main's/.test(s));
        const pin = steps.findIndex((s) => /id: pin/.test(s));
        expect(gate).toBeGreaterThan(-1);
        expect(pin).toBeGreaterThan(gate);
    });

    it("passes the verified commit on, and the build checks out that commit, not the ref", () => {
        expect(jobText("gate")).toMatch(/sha: \$\{\{ steps\.pin\.outputs\.sha \}\}/);
        const checkout = stepsOf("build").find((s) => /uses: actions\/checkout@v4/.test(s) && !/path: \.ci-helper/.test(s));
        expect(checkout).toMatch(/ref: \$\{\{ needs\.gate\.outputs\.sha \}\}/);
    });

    it("reads hand-given inputs through env, never by interpolation into a script", () => {
        for (const job of JOBS) {
            for (const step of stepsOf(job)) {
                const run = /\brun:\s*[|>]?-?\s*\n?([\s\S]*)$/.exec(step);
                if (!run) continue;
                const script = code(run[1].split(/\n {8}[a-z-]+:/)[0]);
                expect(script, `${job}: a run script interpolates an input`).not.toMatch(/\$\{\{\s*(inputs|github\.event\.client_payload)\./);
            }
        }
    });
});

describe("the build job holds no signing credential", () => {
    it("has no Apple secret, no AWS role, no keychain, no signing, no notarization", () => {
        expect(code(jobText("build"))).not.toMatch(CREDENTIAL);
    });

    it("stops at the unsigned app and hands it on as a zip that is not a .dmg", () => {
        const text = code(jobText("build"));
        expect(text).toMatch(/package-macos\.sh --unsigned-app/);
        expect(text).not.toMatch(/\.dmg/);
    });
});

describe("the signing job runs nothing from the built commit", () => {
    const text = code(jobText("sign"));

    it("takes its scripts from the revision of this workflow file named by its identity token", () => {
        expect(text).toMatch(/job_workflow_sha/);
        const checkouts = stepsOf("sign").filter((s) => /uses: actions\/checkout@/.test(s));
        expect(checkouts).toHaveLength(1);
        expect(checkouts[0]).toMatch(/ref: \$\{\{ steps\.identity\.outputs\.sha \}\}/);
        expect(checkouts[0]).toMatch(/path: signing-scripts/);
        expect(checkouts[0]).toMatch(/persist-credentials: false/);
    });

    it("unpacks the build job's zip with the checking script before it signs anything", () => {
        const names = stepsOf("sign").map((s) => (/- name: (.*)/.exec(s) ?? [])[1] ?? "");
        const unpack = names.findIndex((n) => /^Unpack the unsigned app/.test(n));
        const sign = names.findIndex((n) => /^Sign the app/.test(n));
        expect(unpack).toBeGreaterThan(-1);
        expect(sign).toBeGreaterThan(unpack);
        expect(text).toMatch(/bash signing-scripts\/scripts\/unpack-macos-app\.sh /);
        expect(text).not.toMatch(/ditto -x/);
    });

    it("runs only scripts from that checkout, and no package manager or build tool", () => {
        for (const m of text.matchAll(/\bbash (\S+)/g)) expect(m[1], `bash ${m[1]}`).toMatch(/^signing-scripts\/scripts\//);
        expect(text).not.toMatch(/\b(npm|npx|cargo|task|node)\b\s/);
        expect(text).not.toMatch(/\bbash scripts\//);
    });

    it("accepts the apple-signing environment's token and no other subject", () => {
        expect(text).toMatch(/repo:\$\{GITHUB_REPOSITORY\}:environment:apple-signing/);
        expect(text).not.toMatch(/ref:refs\/heads\/main"\|/);
    });

    it("keeps the order: sign, notarize and staple the app, make the DMG, sign it, notarize and staple it", () => {
        const names = stepsOf("sign").map((s) => (/- name: (.*)/.exec(s) ?? [])[1] ?? "");
        const at = (re) => names.findIndex((n) => re.test(n));
        const order = [/^Sign the app/, /^Notarize and staple the app/, /^Make the DMG/, /^Sign, notarize and staple the DMG/];
        const idx = order.map(at);
        expect(idx.every((i) => i > -1)).toBe(true);
        expect([...idx].sort((a, b) => a - b)).toEqual(idx);
        expect(text).toMatch(/notarize-macos\.sh app/);
        expect(text).toMatch(/notarize-macos\.sh dmg/);
    });

    it("deletes the keychain whatever happened", () => {
        const last = stepsOf("sign").at(-1);
        expect(last).toMatch(/if: always\(\)/);
        expect(last).toMatch(/security delete-keychain/);
    });
});

describe("what the release picks up", () => {
    it("uploads a .dmg artifact from `publish` only, after the stapled-ticket and Gatekeeper checks", () => {
        for (const job of ["build", "sign"]) {
            const uploads = stepsOf(job).filter((s) => /upload-artifact@/.test(s));
            for (const u of uploads) expect(u, `${job} uploads a .dmg`).not.toMatch(/path:.*\.dmg(?!\.pending)/);
        }
        const steps = stepsOf("publish");
        const check = steps.findIndex((s) => /name: Check the DMG as a Mac would/.test(s));
        const upload = steps.findIndex((s) => /name: macos-dmg\b(?!-)/.test(s));
        expect(check).toBeGreaterThan(-1);
        expect(upload).toBeGreaterThan(check);
        expect(steps[check]).toMatch(/stapler validate "\$DMG"/);
        expect(steps[check]).toMatch(/stapler validate "\$APP"/);
        expect(steps[check]).toMatch(/source=Notarized Developer ID/);
    });

    it("checks in `publish` whose signature it is: the team of the DMG and of the app, from the repository variable", () => {
        const check = stepsOf("publish").find((s) => /name: Check the DMG as a Mac would/.test(s));
        expect(jobText("publish")).toMatch(/EXPECTED_TEAM_ID: \$\{\{ vars\.APPLE_TEAM_ID \}\}/);
        expect(check).toMatch(/team_of "\$DMG"\)" = "\$EXPECTED_TEAM_ID"/);
        expect(check).toMatch(/team_of "\$APP"\)" = "\$EXPECTED_TEAM_ID"/);
        expect(check).toMatch(/\^\[A-Z0-9\]\{10\}\$/);
    });

    it("pins every third-party action of `sign` and `publish` by commit", () => {
        for (const job of ["sign", "publish"]) {
            for (const step of stepsOf(job)) {
                const uses = /^\s*(?:- )?uses: (\S+)/m.exec(step);
                if (uses) expect(uses[1], `${job}: ${uses[1]}`).toMatch(/@[0-9a-f]{40}$/);
            }
        }
    });

    it("runs no repository code in `publish`", () => {
        const text = code(jobText("publish"));
        expect(text).not.toMatch(/actions\/checkout/);
        expect(text).not.toMatch(/\bbash scripts\/|\bnode scripts\/|\bnpm\b|\bcargo\b/);
    });
});

// release-core.yml gathers the files of a release from the run's artifacts. A build job runs the built commit's code and can
// upload an artifact of its own, so a .dmg is taken from the artifact the publish job uploads and from no other.
describe("release-core.yml takes the macOS DMG from the macos-dmg artifact only", () => {
    const releaseCore = readFileSync(join(ROOT, ".github/workflows/release-core.yml"), "utf8").replace(/\r\n/g, "\n");
    const finds = releaseCore.split("\n").filter((l) => /^\s+find release-artifacts -type f \\\(/.test(l));

    it("has the two flatten steps (sign-release and publish), neither selecting *.dmg generally", () => {
        expect(finds).toHaveLength(2);
        for (const f of finds) expect(f).not.toContain('-name "*.dmg"');
    });

    it("stops the release on a .dmg in any other artifact, in both steps", () => {
        const guard = "find release-artifacts -type f -name '*.dmg' ! -path 'release-artifacts/macos-dmg/*'";
        expect(releaseCore.split(guard)).toHaveLength(3);
        expect(releaseCore.split('find release-artifacts/macos-dmg -type f -name "*.dmg" -exec cp {} dist-flat/').length).toBe(3);
    });
});

// What `sign` and `publish` receive from another job is input: found by name in the run, where another job of the run
// (a caller's own, a sibling build job) could put a file of its own under the name. Each hand-off is bound by a digest
// that comes through a job output, which no other job can set, and is checked before anything reads the file.
describe("the hand-offs are bound by digest", () => {
    it("`build` reports the unsigned app's digest and `sign` checks it before it unpacks, and unpacks before it holds anything", () => {
        expect(jobText("build")).toMatch(/^ {6}app_sha256: \$\{\{ steps\.zip\.outputs\.sha256 \}\}\s*$/m);
        const names = stepsOf("sign").map((s) => (/- name: (.*)/.exec(s) ?? [])[1] ?? "");
        const at = (re) => names.findIndex((n) => re.test(n));
        const digest = at(/^The zip is the one the build job made/);
        const unpack = at(/^Unpack the unsigned app/);
        expect(digest).toBeGreaterThan(-1);
        expect(unpack).toBeGreaterThan(digest);
        const steps = stepsOf("sign");
        const awsRole = steps.findIndex((s) => /configure-aws-credentials/.test(s));
        const secret = at(/^Fetch the Apple signing secrets/);
        const keychain = at(/^Import Developer ID certificate/);
        for (const later of [awsRole, secret, keychain]) expect(later).toBeGreaterThan(unpack);
        expect(steps[digest]).toMatch(/WANT: \$\{\{ needs\.build\.outputs\.app_sha256 \}\}/);
        expect(steps[digest]).toMatch(/\[ "\$GOT" = "\$WANT" \]/);
    });

    it("`sign` reports the DMG's digest of the final bytes and `publish` checks it first", () => {
        expect(jobText("sign")).toMatch(/^ {6}dmg_sha256: \$\{\{ steps\.final\.outputs\.sha256 \}\}\s*$/m);
        const final = stepsOf("sign").find((s) => /id: final/.test(s));
        // after the DMG is signed, notarized and stapled, before it is renamed for upload
        expect(final.indexOf("notarize-macos.sh dmg")).toBeLessThan(final.indexOf("shasum -a 256"));
        expect(final.indexOf("shasum -a 256")).toBeLessThan(final.indexOf(".pending"));
        const check = stepsOf("publish").find((s) => /name: Check the DMG as a Mac would/.test(s));
        expect(check).toMatch(/WANT_SHA256: \$\{\{ needs\.sign\.outputs\.dmg_sha256 \}\}/);
        expect(check.indexOf("WANT_SHA256\" = ") === -1 ? check.indexOf('"$(shasum') : check.indexOf("WANT_SHA256\" =")).toBeLessThan(
            check.indexOf("stapler validate"),
        );
    });

    it("hands the published DMG's digest to the caller, and release-core checks the file against it in both flatten steps", () => {
        expect(source).toMatch(/outputs:\n {6}dmg-sha256:[\s\S]*?value: \$\{\{ jobs\.publish\.outputs\.dmg_sha256 \}\}/);
        const releaseCore = readFileSync(join(ROOT, ".github/workflows/release-core.yml"), "utf8").replace(/\r\n/g, "\n");
        expect(releaseCore.split("MACOS_DMG_SHA256: ${{ needs.build-macos.outputs.dmg-sha256 }}")).toHaveLength(3);
        expect(releaseCore.split("the macOS DMG is not the one build-macos.yml published")).toHaveLength(3);
    });
});

// The signing job holds the keychain and can ask for the identity token: the commands it runs by one bare word are
// called by absolute path, so nothing earlier on a search path can answer for them.
describe("the signing job calls its tools by absolute path", () => {
    it("has no bare security, codesign, xcrun, rm, mv or uuidgen", () => {
        const text = code(jobText("sign"));
        expect(text).not.toMatch(/(^|[\s(;|&`])(security|codesign|xcrun|rm|mv|uuidgen)\s+[-\w"$]/m);
    });
});

// `needs.<job>.outputs` holds only the direct needs of a job: a job that reads build-macos's digest without listing the
// job in its own `needs` gets an empty value, and every release that has a DMG stops at the flatten step.
describe("release-core.yml jobs that read the macOS build's output name it in their needs", () => {
    const releaseCore = readFileSync(join(ROOT, ".github/workflows/release-core.yml"), "utf8").replace(/\r\n/g, "\n");
    const jobs = {};
    let current = null;
    let inJobs = false;
    for (const line of releaseCore.split("\n")) {
        if (/^jobs:\s*$/.test(line)) {
            inJobs = true;
            continue;
        }
        if (!inJobs) continue;
        const head = /^ {2}([A-Za-z0-9_-]+):\s*$/.exec(line);
        if (head) current = jobs[head[1]] = [];
        if (current) current.push(line);
    }
    const readers = Object.entries(jobs).filter(([, text]) => text.join("\n").includes("needs.build-macos.outputs."));

    it("has the two jobs that read it", () => {
        expect(readers.map(([name]) => name).sort()).toEqual(["publish", "sign-release"]);
    });

    it.each(readers)("%s lists build-macos in its needs", (name, text) => {
        const needs = text.find((l) => /^ {4}needs:/.test(l)) ?? "";
        const ids = needs.replace(/^ {4}needs:\s*/, "").replace(/[\[\]]/g, "").split(",").map((x) => x.trim());
        expect(ids).toContain("build-macos");
    });
});
