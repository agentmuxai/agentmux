// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { ISSUE_MARKER, ISSUE_TITLE, evaluate, listedAt, publishedReleases, renderBody } from "./check-store-listing.mjs";

const at = (s) => new Date(s);
const rel = (tag, published_at, extra = {}) => ({ tag_name: tag, published_at, draft: false, prerelease: false, ...extra });
const NOW = at("2026-11-19T12:00:00Z");

describe("listedAt", () => {
    it("reads LastUpdateDateUtc from the product data", () => {
        expect(listedAt({ Payload: { LastUpdateDateUtc: "2026-11-16T16:14:27Z" } }).toISOString()).toBe("2026-11-16T16:14:27.000Z");
    });
    it("throws when it is missing or not a date", () => {
        expect(() => listedAt({})).toThrow(/LastUpdateDateUtc/);
        expect(() => listedAt({ Payload: { LastUpdateDateUtc: "soon" } })).toThrow(/LastUpdateDateUtc/);
        expect(() => listedAt({ Payload: { LastUpdateDateUtc: 5 } })).toThrow(/LastUpdateDateUtc/);
    });
});

describe("publishedReleases", () => {
    it("drops drafts and pre-releases and sorts newest first", () => {
        const list = [
            rel("v1", "2026-11-01T00:00:00Z"),
            rel("v3", "2026-11-03T00:00:00Z"),
            rel("v2-draft", "2026-11-02T00:00:00Z", { draft: true }),
            rel("v2-pre", "2026-11-02T00:00:00Z", { prerelease: true }),
            { tag_name: "v-unpublished", published_at: null, draft: false, prerelease: false },
        ];
        expect(publishedReleases(list).map((r) => r.tag)).toEqual(["v3", "v1"]);
    });
});

describe("evaluate", () => {
    const releases = publishedReleases([
        rel("v0.60.35", "2026-11-19T08:48:08Z"),
        rel("v0.60.34", "2026-11-18T19:00:30Z"),
        rel("v0.60.33", "2026-11-17T20:39:52Z"),
        rel("v0.60.32", "2026-11-17T08:24:06Z"),
        rel("v0.60.31", "2026-11-17T05:58:29Z"),
        rel("v0.60.30", "2026-11-16T15:15:37Z"),
    ]);

    it("a release published before the listing's last update is covered (0.60.30, an hour before its listing update)", () => {
        const r = evaluate({ listed: at("2026-11-16T16:14:27Z"), releases: releases.slice(5), now: NOW });
        expect(r.behind).toBe(false);
        expect(r.uncovered).toEqual([]);
    });

    it("five releases after the listing's update: behind, and the oldest of them is named", () => {
        const r = evaluate({ listed: at("2026-11-16T16:14:27Z"), releases, now: NOW });
        expect(r.behind).toBe(true);
        expect(r.uncovered.map((x) => x.tag)).toEqual(["v0.60.35", "v0.60.34", "v0.60.33", "v0.60.32", "v0.60.31"]);
        expect(r.oldest.tag).toBe("v0.60.31");
    });

    it("one fresh release missing is not behind", () => {
        const r = evaluate({ listed: at("2026-11-19T07:00:00Z"), releases, now: NOW });
        expect(r.uncovered.map((x) => x.tag)).toEqual(["v0.60.35"]);
        expect(r.behind).toBe(false);
    });

    it("one release missing for more than two days is behind", () => {
        const r = evaluate({ listed: at("2026-11-19T07:00:00Z"), releases, now: at("2026-11-21T09:00:00Z") });
        expect(r.behind).toBe(true);
    });

    it("exactly two days is not yet behind; two releases missing are", () => {
        expect(evaluate({ listed: at("2026-11-19T07:00:00Z"), releases, now: at("2026-11-21T08:48:08Z") }).behind).toBe(false);
        expect(evaluate({ listed: at("2026-11-18T10:00:00Z"), releases, now: NOW }).behind).toBe(true);
    });

    it("the limits are settable", () => {
        const listed = at("2026-11-19T07:00:00Z");
        expect(evaluate({ listed, releases, now: NOW, maxReleases: 0 }).behind).toBe(true);
        expect(evaluate({ listed, releases, now: at("2026-11-21T09:00:00Z"), maxDays: 5 }).behind).toBe(false);
    });
});

describe("renderBody", () => {
    const releases = publishedReleases([rel("v2", "2026-11-18T00:00:00Z"), rel("v1", "2026-11-17T00:00:00Z")]);
    const listed = at("2026-11-16T16:14:27Z");
    const result = evaluate({ listed, releases, now: NOW });
    const body = renderBody({ listed, result, maxReleases: 1, maxDays: 2 });

    it("carries the marker, the listing time, each missing release and the oldest's age", () => {
        expect(body.startsWith(ISSUE_MARKER)).toBe(true);
        expect(body).toContain("last updated 2026-11-16 16:14 UTC");
        expect(body).toContain("- v2, published 2026-11-18 00:00 UTC");
        expect(body).toContain("The oldest of them, v1, is 2.5 days old");
    });

    it("says what it reads and cannot see, and names nothing of the account or the plumbing", () => {
        expect(body).toMatch(/cannot see a submission that is still in certification/);
        expect(body).not.toMatch(/MSSTORE|Partner Center|tenant|secret|environment|variable/i);
    });

    it("lists at most eight releases and counts the rest", () => {
        const many = publishedReleases(Array.from({ length: 12 }, (_, i) => rel(`v${i}`, `2026-11-${String(i + 1).padStart(2, "0")}T00:00:00Z`)));
        const r = evaluate({ listed: at("2026-10-01T00:00:00Z"), releases: many, now: NOW });
        const text = renderBody({ listed: at("2026-10-01T00:00:00Z"), result: r, maxReleases: 1, maxDays: 2 });
        expect(text.match(/^- v\d+, published/gm)).toHaveLength(8);
        expect(text).toContain("- and 4 older");
    });
});

// The two workflows around the script: the release run's Store line, and the daily check.
const WORKFLOWS = join(dirname(fileURLToPath(import.meta.url)), "..", ".github/workflows");
const text = (name) => readFileSync(join(WORKFLOWS, name), "utf8").replace(/\r\n/g, "\n");
const jobBlock = (source, id) => {
    const start = source.search(new RegExp(`^  ${id}:\n`, "m"));
    expect(start, `job ${id} exists`).toBeGreaterThan(-1);
    const rest = source.slice(start + 1);
    const next = rest.search(/^  [a-z][a-z0-9-]*:\n/m);
    return next === -1 ? rest : rest.slice(0, next);
};

describe("release-core.yml: the Store line of the run summary", () => {
    const core = text("release-core.yml");
    const store = jobBlock(core, "ms-store");
    const note = jobBlock(core, "ms-store-note");

    it("submission itself is as it was: off unless the switch is on, after publish, and a Store problem never fails the release", () => {
        expect(store).toMatch(/if: \$\{\{ vars\.MSSTORE_SUBMIT_ENABLED == 'true' && !cancelled\(\) && needs\.publish\.result == 'success' \}\}/);
        expect(store).toMatch(/needs: \[gate, publish\]/);
        expect(store).toMatch(/continue-on-error: true/);
    });

    it("the Store job reports the step that failed, through steps that have ids and a last step that runs on failure only", () => {
        expect(store).toMatch(/outputs:\n {6}failed_step: \$\{\{ steps\.failed\.outputs\.step \}\}/);
        for (const id of ["tag", "download", "cli", "token", "configure", "submit"]) expect(store, id).toMatch(new RegExp(`\\n {8}id: ${id}\\n`));
        const last = store.slice(store.indexOf("- name: Say which step failed"));
        expect(last).toMatch(/if: \$\{\{ failure\(\) \}\}/);
        expect(last).not.toMatch(/secrets\./);
    });

    it("the note runs after a published release whatever the Store job did, with read-only repository access and no actions: read", () => {
        expect(note).toMatch(/needs: \[gate, publish, ms-store\]/);
        expect(note).toMatch(/if: \$\{\{ always\(\) && needs\.publish\.result == 'success' \}\}/);
        expect(note).toMatch(/permissions:\n {6}contents: read\n {4}env:/);
        expect(note).not.toMatch(/id-token|actions: read|secrets\./);
    });

    it("it says submitted, submitted as a draft, skipped, failed and cancelled, and warns on everything but a sent submission", () => {
        for (const word of ["submitted:", "submitted as a draft:", "skipped:", "failed:", "cancelled:"]) expect(note).toContain(word);
        expect(note).toContain("::warning title=Microsoft Store::");
        expect(note).toContain("$GITHUB_STEP_SUMMARY");
    });
});

describe("store-listing-check.yml", () => {
    const w = text("store-listing-check.yml");

    it("runs daily and by hand, with read access to the repository and write access to issues only, and uses no secret", () => {
        expect(w).toMatch(/schedule:\n {4}- cron: '[^']+'/);
        expect(w).toMatch(/workflow_dispatch/);
        expect(w).toMatch(/^permissions:\n {2}contents: read\n {2}issues: write\n/m);
        expect(w).not.toMatch(/secrets\.|id-token/);
    });

    it("keeps one issue by its title: edits it while behind, opens it when there is none, closes it when current; an unreadable Store changes nothing", () => {
        expect(w).toContain("gh issue edit");
        expect(w).toContain("gh issue create");
        expect(w).toContain("gh issue close");
        expect(w).toMatch(/ISSUE_TITLE: The Microsoft Store is behind the latest release/);
        expect(w).toMatch(/\[ "\$VERDICT" = behind \]/);
    });

    it("the issue title the workflow uses is the script's", () => {
        expect(w).toContain(`ISSUE_TITLE: ${ISSUE_TITLE}`);
    });
});
