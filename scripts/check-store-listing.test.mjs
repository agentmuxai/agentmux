// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { CATALOG_URL, ISSUE_AUTHOR, ISSUE_MARKER, ISSUE_TITLE, STORE_PRODUCT_ID, catalogVersion, evaluate, listedAt, parseVersion, publishedReleases, renderBody, trackingIssue } from "./check-store-listing.mjs";

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

const catalogOf = (...names) => ({ Product: { DisplaySkuAvailabilities: [{ Sku: { Properties: { Packages: names.map((PackageFullName) => ({ PackageFullName })) } } }] } });

describe("catalogVersion: the package version the Store serves", () => {
    it("reads it from the package's full name", () => {
        expect(catalogVersion(catalogOf("AgentMux.AgentMux_0.60.35.0_x64__vqr1k32tkfk4y"))).toEqual([0, 60, 35]);
    });
    it("takes the newest when several packages or availabilities are named, and ignores packages of other apps", () => {
        const two = { Product: { DisplaySkuAvailabilities: [catalogOf("AgentMux.AgentMux_0.60.9.0_x64__v").Product.DisplaySkuAvailabilities[0], catalogOf("AgentMux.AgentMux_0.60.10.0_arm64__v", "Other.App_9.9.9.0_x64__v").Product.DisplaySkuAvailabilities[0]] } };
        expect(catalogVersion(two)).toEqual([0, 60, 10]);
    });
    it("throws when it names no package of the app, or the data is not what it expects", () => {
        expect(() => catalogVersion({})).toThrow(/no package/);
        expect(() => catalogVersion(catalogOf("Other.App_1.0.0.0_x64__v"))).toThrow(/no package/);
        expect(() => catalogVersion(catalogOf(undefined, 5, "AgentMux.AgentMux_latest_x64__v"))).toThrow(/no package/);
        expect(() => catalogVersion({ Product: { DisplaySkuAvailabilities: [{ Sku: { Properties: {} } }, null] } })).toThrow(/no package/);
    });
    it("reads the public catalog of the app's own product", () => {
        expect(CATALOG_URL).toContain(`displaycatalog.mp.microsoft.com/v7.0/products/${STORE_PRODUCT_ID}`);
    });
});

describe("parseVersion", () => {
    it("reads a plain version or a tag, and nothing else", () => {
        expect(parseVersion("v0.60.35")).toEqual([0, 60, 35]);
        expect(parseVersion("1.2.3")).toEqual([1, 2, 3]);
        for (const bad of ["v1", "v1.2", "v1.2.3-rc1", "nightly", "", undefined]) expect(parseVersion(bad), String(bad)).toBeUndefined();
    });
});

describe("evaluate, by the package version the Store serves", () => {
    const releases = publishedReleases([
        rel("v0.60.36", "2026-11-20T08:00:00Z"),
        rel("v0.60.35", "2026-11-19T08:48:08Z"),
        rel("v0.60.34", "2026-11-18T19:00:30Z"),
        rel("v0.60.9", "2026-11-10T00:00:00Z"),
    ]);

    it("names the releases above the Store's version, and the version compares as numbers (0.60.9 is below 0.60.35)", () => {
        const r = evaluate({ storeVersion: [0, 60, 34], releases, now: NOW });
        expect(r.by).toBe("version");
        expect(r.uncovered.map((x) => x.tag)).toEqual(["v0.60.36", "v0.60.35"]);
        expect(r.oldest.tag).toBe("v0.60.35");
    });

    it("the Store at the latest release is current, and a Store above every release is not behind", () => {
        expect(evaluate({ storeVersion: [0, 60, 36], releases, now: NOW }).uncovered).toEqual([]);
        expect(evaluate({ storeVersion: [0, 61, 0], releases, now: NOW }).behind).toBe(false);
    });

    it("the package version decides, not the listing's time: a listing edit after the release does not hide a missing one", () => {
        const r = evaluate({ storeVersion: [0, 60, 34], listed: at("2026-11-20T12:00:00Z"), releases, now: at("2026-11-22T12:00:00Z") });
        expect(r.uncovered.map((x) => x.tag)).toEqual(["v0.60.36", "v0.60.35"]);
        expect(r.behind).toBe(true);
    });

    it("a release whose tag is not a plain version cannot be compared and is left out", () => {
        const odd = publishedReleases([rel("nightly-1", "2026-11-20T09:00:00Z"), rel("v0.60.36", "2026-11-20T08:00:00Z")]);
        expect(evaluate({ storeVersion: [0, 60, 35], releases: odd, now: NOW }).uncovered.map((x) => x.tag)).toEqual(["v0.60.36"]);
    });

    it("the same limits apply: one fresh release missing is not behind, one older than two days is", () => {
        expect(evaluate({ storeVersion: [0, 60, 35], releases, now: NOW }).behind).toBe(false);
        expect(evaluate({ storeVersion: [0, 60, 35], releases, now: at("2026-11-23T00:00:00Z") }).behind).toBe(true);
    });

    it("without a package version it falls back to the listing's time, and says so", () => {
        const r = evaluate({ listed: at("2026-11-19T07:00:00Z"), releases, now: NOW });
        expect(r.by).toBe("date");
        expect(r.uncovered.map((x) => x.tag)).toEqual(["v0.60.36", "v0.60.35"]);
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

describe("renderBody, by the package version", () => {
    const releases = publishedReleases([rel("v0.60.36", "2026-11-20T08:00:00Z"), rel("v0.60.35", "2026-11-19T08:48:08Z")]);
    const storeVersion = [0, 60, 34];
    const listed = at("2026-11-19T22:36:00Z");
    const result = evaluate({ storeVersion, listed, releases, now: at("2026-11-22T12:00:00Z") });
    const body = renderBody({ storeVersion, listed, result, maxReleases: 1, maxDays: 2 });

    it("says exactly which version the Store serves and which releases are newer", () => {
        expect(body.startsWith(ISSUE_MARKER)).toBe(true);
        expect(body).toContain("The Microsoft Store serves version 0.60.34");
        expect(body).toContain("2 newer releases are on GitHub");
        expect(body).toContain("- v0.60.36, published 2026-11-20 08:00 UTC");
        expect(body).toContain("- v0.60.35, published 2026-11-19 08:48 UTC");
        expect(body).toContain("The oldest of them, v0.60.35, is 3.1 days old");
    });

    it("says it read the catalog's package version, and names nothing of the account or the plumbing", () => {
        expect(body).toMatch(/package version the Store's public display catalog names/);
        expect(body).not.toMatch(/MSSTORE|Partner Center|tenant|secret|environment|variable/i);
    });

    it("the fallback says the version could not be read and that dates were compared", () => {
        const byDate = evaluate({ listed: at("2026-11-16T16:14:27Z"), releases, now: at("2026-11-22T12:00:00Z") });
        const text = renderBody({ listed: at("2026-11-16T16:14:27Z"), result: byDate, maxReleases: 1, maxDays: 2 });
        expect(text).toContain("package version could not be read, so this compares dates instead");
        expect(text).toContain("last updated 2026-11-16 16:14 UTC");
        expect(text).not.toContain("serves version");
    });

    it("a single missing release reads in the singular", () => {
        const one = evaluate({ storeVersion: [0, 60, 35], releases, now: at("2026-11-23T00:00:00Z") });
        expect(renderBody({ storeVersion: [0, 60, 35], result: one, maxReleases: 1, maxDays: 2 })).toContain("1 newer release is on GitHub, so a Store install cannot move to it yet");
    });
});

describe("trackingIssue: only the Actions bot's own issue is the tracking issue", () => {
    const issue = (number, login, extra = {}) => ({ number, title: ISSUE_TITLE, state: "open", user: { login, type: login.endsWith("[bot]") ? "Bot" : "User" }, ...extra });
    const bot = ISSUE_AUTHOR;

    it("an issue with the same title by someone else is ignored, so the check opens its own (nothing to edit or close)", () => {
        expect(trackingIssue([issue(7, "someone")])).toBeUndefined();
    });
    it("a user whose name only looks like the bot's is ignored; an app account of another name is ignored", () => {
        expect(trackingIssue([issue(7, "github-actions"), { ...issue(8, "github-actions[bot]"), user: { login: "github-actions[bot]", type: "User" } }, issue(9, "other-app[bot]")])).toBeUndefined();
    });
    it("the bot's own open issue is reused, even when a stranger's with the same title is listed first", () => {
        expect(trackingIssue([issue(7, "someone"), issue(5, bot)])).toBe(5);
    });
    it("when the Store has caught up the number returned is the bot's alone: a stranger's issue is never the one closed", () => {
        expect(trackingIssue([issue(7, "someone"), issue(8, "another")])).toBeUndefined();
        expect(trackingIssue([issue(8, "another"), issue(5, bot), issue(7, "someone")])).toBe(5);
    });
    it("a closed issue, a pull request and a different title do not count", () => {
        expect(trackingIssue([issue(1, bot, { state: "closed" }), issue(2, bot, { pull_request: {} }), issue(3, bot, { title: "something else" })])).toBeUndefined();
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

    it("a Store job that read as success is 'submitted' only when the submit step itself said so (a job that stopped before any step ran is not)", () => {
        expect(store).toMatch(/outputs:\n[\s\S]*?submitted: \$\{\{ steps\.submit\.outputs\.sent \}\}/);
        const submit = store.slice(store.indexOf("- name: Submit to the Microsoft Store"), store.indexOf("- name: Say which step failed"));
        expect(submit).toMatch(/msstore publish[\s\S]*exit \$LASTEXITCODE[\s\S]*'sent=true' \| Out-File -Append -Encoding utf8 \$env:GITHUB_OUTPUT/);
        expect(note).toMatch(/SUBMITTED: \$\{\{ needs\.ms-store\.outputs\.submitted \}\}/);
        const notSent = note.indexOf('[ "$RESULT" = success ] && [ "$SUBMITTED" != true ]');
        expect(notSent).toBeGreaterThan(-1);
        expect(notSent).toBeLessThan(note.indexOf("submitted:"));
        expect(note.slice(notSent, note.indexOf("elif", notSent + 10))).toContain("warn=1");
    });

    it("it says submitted, submitted as a draft, skipped, failed and cancelled, and warns on everything but a sent submission", () => {
        for (const word of ["submitted:", "submitted as a draft:", "skipped:", "failed:", "cancelled:"]) expect(note).toContain(word);
        expect(note).toContain("::warning title=Microsoft Store::");
        expect(note).toContain("$GITHUB_STEP_SUMMARY");
    });
});

describe("release-core.yml: the WinGet line of the run summary", () => {
    const core = text("release-core.yml");
    const winget = jobBlock(core, "winget");
    const note = jobBlock(core, "winget-note");

    it("submission itself is as it was: off unless the switch is on, after publish, and a WinGet problem never fails the release", () => {
        expect(winget).toMatch(/if: \$\{\{ vars\.WINGET_SUBMIT_ENABLED == 'true' && !cancelled\(\) && needs\.publish\.result == 'success' \}\}/);
        expect(winget).toMatch(/needs: \[gate, publish\]/);
        expect(winget).toMatch(/continue-on-error: true/);
        expect(winget).toContain("./wingetcreate.exe update AgentMux.AI");
        expect(winget).toContain("--submit");
    });

    it("the WinGet job says by itself that a pull request was opened, only after wingetcreate succeeded, and names the step that failed", () => {
        expect(winget).toMatch(/outputs:\n {6}failed_step: \$\{\{ steps\.failed\.outputs\.step \}\}\n {6}submitted: \$\{\{ steps\.submit\.outputs\.sent \}\}/);
        for (const id of ["tag", "install", "submit"]) expect(winget, id).toMatch(new RegExp(`\\n {8}id: ${id}\\n`));
        const submit = winget.slice(winget.indexOf("- name: Submit the WinGet manifest pull request"), winget.indexOf("- name: Say which step failed"));
        expect(submit).toMatch(/--token "\$WINGET_TOKEN"\n\s+#[^\n]*\n\s+echo "sent=true" >> "\$GITHUB_OUTPUT"/);
        const last = winget.slice(winget.indexOf("- name: Say which step failed"));
        expect(last).toMatch(/if: \$\{\{ failure\(\) \}\}/);
        expect(last).not.toMatch(/secrets\./);
    });

    it("the token is still given to the one step that submits", () => {
        expect(winget.match(/secrets\.WINGET_TOKEN/g)).toHaveLength(1);
        expect(winget.slice(winget.indexOf("- name: Submit the WinGet manifest pull request"))).toContain("WINGET_TOKEN: ${{ secrets.WINGET_TOKEN }}");
    });

    it("the note runs after a published release whatever the WinGet job did, with read-only repository access and no token", () => {
        expect(note).toMatch(/needs: \[gate, publish, winget\]/);
        expect(note).toMatch(/if: \$\{\{ always\(\) && needs\.publish\.result == 'success' \}\}/);
        expect(note).toMatch(/permissions:\n {6}contents: read\n {4}env:/);
        expect(note).not.toMatch(/id-token|actions: read|secrets\./);
        expect(note).toMatch(/SUBMITTED: \$\{\{ needs\.winget\.outputs\.submitted \}\}/);
    });

    it("a job that read as success is 'submitted' only when the submit step said so", () => {
        const notSent = note.indexOf('[ "$RESULT" = success ] && [ "$SUBMITTED" != true ]');
        expect(notSent).toBeGreaterThan(-1);
        expect(notSent).toBeLessThan(note.indexOf("submitted:"));
        expect(note.slice(notSent, note.indexOf("elif", notSent + 10))).toContain("warn=1");
    });

    it("it says submitted, skipped, failed and cancelled in the Store line's style, and warns on everything but a submitted pull request", () => {
        for (const word of ["submitted:", "skipped: WinGet submission is switched off", "failed:", "cancelled:"]) expect(note).toContain(word);
        expect(note).toContain("::warning title=WinGet::");
        expect(note).toContain("$GITHUB_STEP_SUMMARY");
        expect(note).toContain("### WinGet");
        const submitted = note.slice(note.indexOf('line="submitted:'));
        expect(submitted.slice(0, submitted.indexOf("fi\n"))).toContain("warn=0");
        for (const word of ["failed:", "skipped:", "cancelled:"]) {
            const at = note.indexOf(`line="${word}`);
            expect(note.slice(at, note.indexOf("elif", at) === -1 ? undefined : note.indexOf("elif", at))).toContain("warn=1");
        }
    });

    it("the note's job is a need of nothing: the release does not wait for it", () => {
        for (const other of ["publish", "sign-release", "ms-store", "ms-store-note"]) {
            const block = jobBlock(core, other);
            expect(block.match(/^ {4}needs:.*$/m)?.[0] ?? "", other).not.toContain("winget-note");
        }
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

    it("finds the issue through the script by title and author, asks only for the bot's issues, and edits or closes only that one", () => {
        expect(w).toContain("--find-issue store-check/issues.json");
        expect(w).toContain("creator=github-actions%5Bbot%5D");
        expect(w).not.toMatch(/gh issue list/);
        expect(w).toMatch(/gh issue edit "\$existing"/);
        expect(w).toMatch(/gh issue close "\$existing"/);
        expect(w).toMatch(/^permissions:\n {2}contents: read\n {2}issues: write\n/m);
    });

    it("the issue title the workflow uses is the script's", () => {
        expect(w).toContain(`ISSUE_TITLE: ${ISSUE_TITLE}`);
    });
});
