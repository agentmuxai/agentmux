// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Is the Microsoft Store behind the latest release? (.github/workflows/store-listing-check.yml, daily.)
//
// Reads two public things, with no secret:
//   - the Store listing's product data (the same page the Store app shows): its "last updated" time;
//   - this repository's published releases.
// A release published after the listing was last updated is not in the Store. The Store is behind when more than
// `--max-releases` such releases exist, or the oldest of them is older than `--max-days`.
//
// What it cannot see: a submission still in certification (the listing does not change until it passes), and a
// listing edit that is not a new package (it moves the date too). So it is a smoke alarm, not the record of what
// the Store holds. A listing it cannot read is its own outcome (exit 2), never "behind": nothing is opened or closed.
//
//   node scripts/check-store-listing.mjs --repo owner/name [--max-releases 1] [--max-days 2] [--out dir]
//
// Writes <out>/verdict.txt ("behind" or "current") and <out>/body.md (the issue text), and `verdict` to
// $GITHUB_OUTPUT. Exit 0: it read both and decided; 2: it could not read one.

import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const STORE_PRODUCT_ID = "9P9QCXNNCRK3";
export const STORE_URL = `https://storeedgefd.dsx.mp.microsoft.com/v9.0/products/${STORE_PRODUCT_ID}?market=US&locale=en-US&deviceFamily=Windows.Desktop`;
export const ISSUE_TITLE = "The Microsoft Store is behind the latest release";
export const ISSUE_MARKER = "<!-- store-listing-check -->";

const DAY_MS = 24 * 60 * 60 * 1000;

/** The listing's last update, as a Date, from the Store's product data. Throws when it is not there. */
export function listedAt(product) {
    const raw = product?.Payload?.LastUpdateDateUtc;
    const at = new Date(raw);
    if (typeof raw !== "string" || Number.isNaN(at.getTime())) throw new Error("the Store's product data has no usable LastUpdateDateUtc");
    return at;
}

/** The published releases (not drafts, not pre-releases), newest first, as { tag, at }. */
export function publishedReleases(list) {
    return list
        .filter((r) => !r.draft && !r.prerelease && r.published_at)
        .map((r) => ({ tag: r.tag_name, at: new Date(r.published_at) }))
        .filter((r) => !Number.isNaN(r.at.getTime()))
        .sort((a, b) => b.at - a.at);
}

/**
 * The verdict. `uncovered` are the releases published after the listing's last update, newest first. Behind when there are more
 * than `maxReleases` of them, or the oldest is more than `maxDays` days old.
 */
export function evaluate({ listed, releases, now, maxReleases = 1, maxDays = 2 }) {
    const uncovered = releases.filter((r) => r.at > listed);
    const oldest = uncovered.at(-1);
    const oldestDays = oldest ? (now - oldest.at) / DAY_MS : 0;
    const behind = uncovered.length > maxReleases || (uncovered.length > 0 && oldestDays > maxDays);
    return { behind, uncovered, oldest, oldestDays };
}

const fmt = (d) => d.toISOString().slice(0, 16).replace("T", " ") + " UTC";

/** The issue text: what is behind, what this reads and cannot see. Behaviour only: nothing of the account or the release plumbing. */
export function renderBody({ listed, result, maxReleases, maxDays }) {
    const { uncovered, oldest, oldestDays } = result;
    const shown = uncovered.slice(0, 8).map((r) => `- ${r.tag}, published ${fmt(r.at)}`);
    const more = uncovered.length > shown.length ? [`- and ${uncovered.length - shown.length} older`] : [];
    return [
        ISSUE_MARKER,
        `The Microsoft Store listing was last updated ${fmt(listed)}. ${uncovered.length} release${uncovered.length === 1 ? " was" : "s were"} published after that, so a Store install cannot move to ${uncovered.length === 1 ? "it" : "them"} yet:`,
        "",
        ...shown,
        ...more,
        "",
        `The oldest of them, ${oldest.tag}, is ${oldestDays.toFixed(1)} days old. The check flags the Store when more than ${maxReleases} release${maxReleases === 1 ? "" : "s"} is missing or the oldest missing one is more than ${maxDays} days old.`,
        "",
        "A new package has to be submitted to the Store and pass its certification before the listing moves. Each release run's summary has a \"Microsoft Store\" line saying whether that release was submitted.",
        "",
        "What this reads: the Store listing's public product data (its last-updated time) and this repository's published releases. It cannot see a submission that is still in certification, and a listing edit that is not a new package also moves the date, so treat it as a smoke alarm.",
        "",
        "Kept up to date by the Store listing check workflow, which closes this issue when the Store catches up.",
        "",
    ].join("\n");
}

async function getJson(url, headers) {
    const res = await fetch(url, { headers: { "User-Agent": "agentmux-store-listing-check", Accept: "application/json", ...headers } });
    if (!res.ok) throw new Error(`${new URL(url).host} answered ${res.status}`);
    return res.json();
}

function args(argv) {
    const o = { maxReleases: 1, maxDays: 2, out: "store-check" };
    for (let i = 0; i < argv.length; i += 2) {
        const [k, v] = [argv[i], argv[i + 1]];
        if (k === "--repo") o.repo = v;
        else if (k === "--max-releases") o.maxReleases = Number(v);
        else if (k === "--max-days") o.maxDays = Number(v);
        else if (k === "--out") o.out = v;
        else throw new Error(`unknown option ${k}`);
    }
    if (!/^[\w.-]+\/[\w.-]+$/.test(o.repo ?? "")) throw new Error("--repo owner/name is required");
    if (!(o.maxReleases >= 0) || !(o.maxDays >= 0)) throw new Error("--max-releases and --max-days must be numbers of 0 or more");
    return o;
}

async function main() {
    const o = args(process.argv.slice(2));
    const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
    const product = await getJson(STORE_URL);
    const list = await getJson(`https://api.github.com/repos/${o.repo}/releases?per_page=50`, token ? { Authorization: `Bearer ${token}` } : {});
    const listed = listedAt(product);
    const releases = publishedReleases(list);
    if (releases.length === 0) throw new Error("the repository has no published release to compare with");
    const result = evaluate({ listed, releases, now: new Date(), maxReleases: o.maxReleases, maxDays: o.maxDays });
    const verdict = result.behind ? "behind" : "current";
    mkdirSync(o.out, { recursive: true });
    writeFileSync(join(o.out, "verdict.txt"), `${verdict}\n`);
    if (result.behind) writeFileSync(join(o.out, "body.md"), renderBody({ listed, result, maxReleases: o.maxReleases, maxDays: o.maxDays }));
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `verdict=${verdict}\n`);
    console.log(`Store listing last updated ${fmt(listed)}; ${result.uncovered.length} newer release(s); newest release ${releases[0].tag}: ${verdict}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
    main().catch((e) => {
        console.error(`::error::could not read the Store listing or the releases: ${e.message}`);
        process.exit(2);
    });
}
