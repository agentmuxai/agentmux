// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A build that can be started by hand names any ref to build. The build workflows therefore refuse a
// commit that is not in main's history before any step asks the actions broker for anything or runs code
// from the checkout (npm scripts, build scripts); a release (a tag) goes through the release gate. This
// reads each build workflow's steps in order and fails if the gate is missing, comes late, or a hand-given
// input reaches a shell script by interpolation instead of through env.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (name) => readFileSync(join(ROOT, ".github/workflows", name), "utf8").replace(/\r\n/g, "\n");

/** The steps of one job: each is the text from its `      - ` line to the next one. */
function stepsOf(source, job = "build") {
    const lines = source.split("\n");
    const steps = [];
    let inJob = false;
    let inSteps = false;
    for (const line of lines) {
        if (/^ {2}[A-Za-z0-9_-]+:\s*$/.test(line)) inJob = line.trim() === `${job}:`;
        if (!inJob) continue;
        if (/^ {4}steps:\s*$/.test(line)) {
            inSteps = true;
            continue;
        }
        if (!inSteps) continue;
        if (/^ {6}- /.test(line)) steps.push([line]);
        else if (steps.length > 0 && !/^ {0,3}\S/.test(line)) steps.at(-1).push(line);
        else if (/^ {0,3}\S/.test(line) && !line.startsWith("#")) break;
    }
    return steps.map((s) => s.join("\n"));
}

const isGate = (step) => /name: The code to be (built|signed) is main's/.test(step);
const isBroker = (step) => /uses: agentmuxai\/agentmux\/\.github\/actions\/actions-broker@/.test(step);
// A step that runs code that came with the checkout.
const runsCheckoutCode = (step) =>
    /\brun:[\s\S]*\b(npm|npx|cargo|node scripts|bash scripts|task)\b/.test(step) && !isGate(step);

describe.each(["build-linux.yml", "build-windows.yml"])("%s", (file) => {
    const source = read(file);
    const steps = stepsOf(source);
    const gate = steps.findIndex(isGate);

    it("has the gate step", () => {
        expect(gate).toBeGreaterThan(-1);
        expect(steps.filter(isGate)).toHaveLength(1);
    });

    it("runs the gate before any broker request and any step that runs the checkout's code", () => {
        const firstBroker = steps.findIndex(isBroker);
        expect(firstBroker).toBeGreaterThan(gate);
        const firstCode = steps.findIndex((s, i) => i !== gate && runsCheckoutCode(s));
        expect(firstCode).toBeGreaterThan(gate);
    });

    it("checks the commit against main, and a tag against the release gate", () => {
        const g = steps[gate];
        expect(g).toContain('compare/main...${HEAD_SHA}');
        expect(g).toContain("identical|behind");
        expect(g).toContain('--ref-type "$REF_TYPE" --ref-name "$REF_NAME" --sha "$RUN_SHA" --repo "$REPO"');
        expect(g).toContain('[ "$HEAD_SHA" != "$RUN_SHA" ]');
        expect(g).toContain('[ "$RUN_REF" != "refs/heads/main" ]');
    });

    it("hands the dispatch inputs to the shell through env, after a shape check", () => {
        for (const line of source.split("\n")) {
            if (/\$\{\{\s*(inputs\.(ref|release-tag|cef-runtime-tag)|github\.event\.client_payload\.)/.test(line)) {
                expect(line, "an input interpolated outside an env entry").toMatch(/^\s+[IP]_[A-Z_]+: \$\{\{/);
            }
        }
        const resolve = steps.find((s) => /name: Resolve inputs/.test(s));
        expect(resolve).toContain('=~ ^[A-Za-z0-9._/+-]*$');
    });
});

describe("build-server-package.yml", () => {
    const steps = stepsOf(read("build-server-package.yml"), "package");
    const gate = steps.findIndex(isGate);

    it("has the gate step, which a pull request that changes the workflow does not run", () => {
        expect(gate).toBeGreaterThan(-1);
        expect(steps[gate]).toContain("if: github.event_name != 'pull_request'");
        expect(steps[gate]).toContain("identical|behind");
        expect(steps[gate]).toContain('--ref-type "$REF_TYPE" --ref-name "$REF_NAME" --sha "$RUN_SHA" --repo "$REPO"');
    });

    it("runs the gate before any step that runs the checkout's code", () => {
        expect(steps.findIndex((s, i) => i !== gate && runsCheckoutCode(s))).toBeGreaterThan(gate);
    });
});

describe("build-macos.yml", () => {
    it("keeps its gate before any step that runs the checkout's code", () => {
        const steps = stepsOf(read("build-macos.yml"));
        const gate = steps.findIndex(isGate);
        expect(gate).toBeGreaterThan(-1);
        expect(steps.findIndex((s, i) => i !== gate && runsCheckoutCode(s))).toBeGreaterThan(gate);
    });
});
