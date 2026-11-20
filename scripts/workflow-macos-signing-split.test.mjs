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

    it("runs no repository code in `publish`", () => {
        const text = code(jobText("publish"));
        expect(text).not.toMatch(/actions\/checkout/);
        expect(text).not.toMatch(/\bbash scripts\/|\bnode scripts\/|\bnpm\b|\bcargo\b/);
    });
});
