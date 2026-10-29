// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for check-product-text.mjs.

import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { ALLOW, checkText, run, targets } from "./check-product-text.mjs";

const found = (path, text, allow = []) => checkText(path, text, allow, new Set()).map((v) => `${v.term}:${v.line}`);

describe("the deny-list", () => {
    it("flags each private name, with its line", () => {
        expect(found("seed.json", 'ok\n"run `gh-agent auth status`"\nsee a5af/shared-infrastructure')).toEqual([
            "gh-agent:2",
            "a5af:3",
            "shared-infrastructure:3",
        ]);
        expect(found("seed.json", "ask muxreview, or Run Secrets Escalate").sort()).toEqual(["muxreview:1", "secrets escalate:1"]);
    });

    it("finds a name right after a JSON escape, as in a multi-line prompt", () => {
        expect(found("seed.json", '"a\\ngh-agent auth"')).toEqual(["gh-agent:1"]);
        expect(found("seed.json", '"a\\tmuxreview and \\"a5af\\""')).toEqual(["a5af:1", "muxreview:1"]);
    });

    it("matches whole names only", () => {
        expect(found("seed.json", "a5afe and gh-agents and my-gh-agent-x and reagents")).toEqual([]);
    });

    it("lets generic wording through", () => {
        expect(found("seed.json", "Check your GitHub identity with `gh auth status` where `gh` is installed.")).toEqual([]);
    });
});

describe("the allow list", () => {
    const entry = { file: "seed.json", text: "if a `gh-agent` command is installed", reason: "conditional" };

    it("lets the exact allowed wording through, and still flags the same name elsewhere", () => {
        const text = 'use gh-agent\n"if a `gh-agent` command is installed here"';
        expect(found("seed.json", text, [entry])).toEqual(["gh-agent:1"]);
    });

    it("reports each entry it used", () => {
        const used = new Set();
        checkText("seed.json", "if a `gh-agent` command is installed", [entry], used);
        expect(used.has(entry)).toBe(true);
    });

    it("every shipped entry names its file, its wording and a reason", () => {
        for (const a of ALLOW) {
            expect(a.file && a.text && a.reason.length > 20, a.text).toBeTruthy();
        }
    });
});

describe("the repository", () => {
    it("is clean, and none of the allow entries is stale", () => {
        const { violations, stale } = run();
        expect(violations).toEqual([]);
        expect(stale).toEqual([]);
    });

    it("covers the seed manifests", () => {
        const files = targets();
        expect(files).toContain("crates/srv/agent-seed.json");
        expect(files).toContain("crates/srv/operator-config-seed.json");
    });

    it("fails on a listed file that does not exist, and scans the ones that do", () => {
        const root = mkdtempSync(join(tmpdir(), "product-text-"));
        const [first, ...rest] = targets();
        mkdirSync(dirname(join(root, first)), { recursive: true });
        writeFileSync(join(root, first), "Run `gh-agent doctor` first.\n");
        const { violations, missing } = run(root);
        expect(missing).toEqual(rest);
        expect(violations.map((v) => `${v.path}:${v.line}:${v.term}`)).toEqual([`${first}:1:gh-agent`]);
    });

    it("finds every listed file in the repository", () => {
        expect(run().missing).toEqual([]);
    });
});
