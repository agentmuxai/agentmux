// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Is the Microsoft Store behind the latest release? (.github/workflows/store-listing-check.yml, daily.)
//
// Reads three public things, with no secret:
//   - the Store's display catalog (what the Store app installs from): the package version the Store serves;
//   - the Store listing's product data: its "last updated" time, used only when the catalog cannot be read;
//   - this repository's published releases.
// A release newer than the version the Store serves is not in the Store. The Store is behind when more than
// `--max-releases` such releases exist, or the oldest of them is older than `--max-days`. Without a readable package
// version (the catalog is down, or names no package) the check falls back to the listing's time: a release published after
// the listing was last updated is not in the Store. It says which of the two it used.
//
// What it cannot see: a submission still in certification (the catalog does not change until it passes), and, in the
// fallback, a listing edit that is not a new package (it moves the date too). So it is a smoke alarm, not the record of what
// the Store holds. When neither the catalog nor the listing can be read, that is its own outcome (exit 2), never "behind":
// nothing is opened or closed.
//
//   node scripts/check-store-listing.mjs --repo owner/name [--max-releases 1] [--max-days 2] [--out dir]
//   node scripts/check-store-listing.mjs --find-issue issues.json   (the tracking issue's number, by title AND author)
//
// Writes <out>/verdict.txt ("behind" or "current") and <out>/body.md (the issue text), and `verdict` to
// $GITHUB_OUTPUT. Exit 0: it read both and decided; 2: it could not read one.

import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const STORE_PRODUCT_ID = "9P9QCXNNCRK3";
export const STORE_URL = `https://storeedgefd.dsx.mp.microsoft.com/v9.0/products/${STORE_PRODUCT_ID}?market=US&locale=en-US&deviceFamily=Windows.Desktop`;
export const CATALOG_URL = `https://displaycatalog.mp.microsoft.com/v7.0/products/${STORE_PRODUCT_ID}?market=US&languages=en-US`;
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

/** The package family the Store serves the app under: `<family>_<version>.0_<arch>__<publisher id>`. */
const PACKAGE_FULL_NAME = /^AgentMux\.AgentMux_(\d+)\.(\d+)\.(\d+)\.\d+_/;

/** A version as [major, minor, patch] from `1.2.3` or a tag `v1.2.3`; undefined for anything else. */
export function parseVersion(text) {
    const m = /^v?(\d+)\.(\d+)\.(\d+)$/.exec(String(text));
    return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : undefined;
}

const compareVersions = (a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2];
const showVersion = (v) => v.join(".");

/**
 * The newest package version the Store serves, as [major, minor, patch], from the display catalog's product data (each package
 * is named like `AgentMux.AgentMux_0.60.35.0_x64__<id>`). Throws when no package of the app is named there.
 */
export function catalogVersion(catalog) {
    const found = [];
    for (const availability of catalog?.Product?.DisplaySkuAvailabilities ?? []) {
        for (const pkg of availability?.Sku?.Properties?.Packages ?? []) {
            const m = PACKAGE_FULL_NAME.exec(typeof pkg?.PackageFullName === "string" ? pkg.PackageFullName : "");
            if (m) found.push([Number(m[1]), Number(m[2]), Number(m[3])]);
        }
    }
    if (found.length === 0) throw new Error("the Store's display catalog names no package of the app");
    return found.sort(compareVersions).at(-1);
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
 * The verdict. `uncovered` are the releases the Store does not have, newest first: those with a version above `storeVersion` when
 * it is known (a release whose tag is not a plain version cannot be compared and is left out), otherwise those published after
 * the listing's last update `listed`. Behind when there are more than `maxReleases` of them, or the oldest is more than `maxDays`
 * days old. `by` says which of the two decided.
 */
export function evaluate({ storeVersion, listed, releases, now, maxReleases = 1, maxDays = 2 }) {
    const by = storeVersion ? "version" : "date";
    const uncovered = storeVersion
        ? releases.filter((r) => {
              const v = parseVersion(r.tag);
              return v !== undefined && compareVersions(v, storeVersion) > 0;
          })
        : releases.filter((r) => r.at > listed);
    const oldest = uncovered.at(-1);
    const oldestDays = oldest ? (now - oldest.at) / DAY_MS : 0;
    const behind = uncovered.length > maxReleases || (uncovered.length > 0 && oldestDays > maxDays);
    return { behind, uncovered, oldest, oldestDays, by };
}

/** The only account whose issue counts as this check's own. */
export const ISSUE_AUTHOR = "github-actions[bot]";

/**
 * The one issue this check keeps: an open issue with the check's title that the Actions bot opened. The repository is public, so
 * anyone can open an issue with the same title; theirs is never the tracking issue, and so is never edited or closed. `issues` is
 * the REST list of issues (pull requests also appear in it and are skipped). Returns the issue's number, or undefined.
 */
export function trackingIssue(issues, title = ISSUE_TITLE) {
    return issues.find(
        (i) => i.state !== "closed" && !i.pull_request && i.title === title && i.user?.login === ISSUE_AUTHOR && i.user?.type === "Bot",
    )?.number;
}

const fmt = (d) => d.toISOString().slice(0, 16).replace("T", " ") + " UTC";

/** The first sentence of the issue: exactly what the Store has, and how that was read. */
function storeSentence({ storeVersion, listed, result }) {
    const n = result.uncovered.length;
    const missing = `${n} newer release${n === 1 ? "" : "s"} ${n === 1 ? "is" : "are"} on GitHub, so a Store install cannot move to ${n === 1 ? "it" : "them"} yet:`;
    if (result.by === "version") {
        const listedText = listed ? `, and its listing was last updated ${fmt(listed)}` : "";
        return `The Microsoft Store serves version ${showVersion(storeVersion)}${listedText}. ${missing}`;
    }
    return `The Microsoft Store's package version could not be read, so this compares dates instead: its listing was last updated ${fmt(listed)}, and ${n} release${n === 1 ? " was" : "s were"} published after that, so a Store install cannot move to ${n === 1 ? "it" : "them"} yet:`;
}

/** The issue text: what is behind, what this reads and cannot see. Behaviour only: nothing of the account or the release plumbing. */
export function renderBody({ storeVersion, listed, result, maxReleases, maxDays }) {
    const { uncovered, oldest, oldestDays } = result;
    const shown = uncovered.slice(0, 8).map((r) => `- ${r.tag}, published ${fmt(r.at)}`);
    const more = uncovered.length > shown.length ? [`- and ${uncovered.length - shown.length} older`] : [];
    const reads =
        result.by === "version"
            ? "What this reads: the package version the Store's public display catalog names for the app, and this repository's published releases. It cannot see a submission that is still in certification, so treat it as a smoke alarm."
            : "What this reads: the Store listing's public product data (its last-updated time) and this repository's published releases. It cannot see a submission that is still in certification, and a listing edit that is not a new package also moves the date, so treat it as a smoke alarm.";
    return [
        ISSUE_MARKER,
        storeSentence({ storeVersion, listed, result }),
        "",
        ...shown,
        ...more,
        "",
        `The oldest of them, ${oldest.tag}, is ${oldestDays.toFixed(1)} days old. The check flags the Store when more than ${maxReleases} release${maxReleases === 1 ? "" : "s"} is missing or the oldest missing one is more than ${maxDays} days old.`,
        "",
        "A new package has to be submitted to the Store and pass its certification before the catalog moves. Each release run's summary has a \"Microsoft Store\" line saying whether that release was submitted.",
        "",
        reads,
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

// `--find-issue <issues.json>`: print the number of the tracking issue in the REST issue list in that file, or nothing.
function findIssue(file) {
    const found = trackingIssue(JSON.parse(readFileSync(file, "utf8")));
    if (found !== undefined) console.log(found);
}

/** One public source, read and parsed; a failure is returned, not thrown, so the other source can still decide. */
async function tryRead(read) {
    try {
        return { value: await read() };
    } catch (e) {
        return { error: e.message };
    }
}

async function main() {
    if (process.argv[2] === "--find-issue") return findIssue(process.argv[3]);
    const o = args(process.argv.slice(2));
    const token = process.env.GH_TOKEN || process.env.GITHUB_TOKEN;
    const [catalog, listing] = await Promise.all([
        tryRead(async () => catalogVersion(await getJson(CATALOG_URL))),
        tryRead(async () => listedAt(await getJson(STORE_URL))),
    ]);
    // Neither source readable: the Store's state is unknown, which is not "behind". Nothing is opened or closed.
    if (catalog.error && listing.error) throw new Error(`the display catalog: ${catalog.error}; the listing: ${listing.error}`);
    if (catalog.error) console.log(`::warning title=Store package version::could not read the package version (${catalog.error}); comparing the listing's last-updated time instead`);
    const list = await getJson(`https://api.github.com/repos/${o.repo}/releases?per_page=50`, token ? { Authorization: `Bearer ${token}` } : {});
    const storeVersion = catalog.value;
    const listed = listing.value;
    const releases = publishedReleases(list);
    if (releases.length === 0) throw new Error("the repository has no published release to compare with");
    const result = evaluate({ storeVersion, listed, releases, now: new Date(), maxReleases: o.maxReleases, maxDays: o.maxDays });
    const verdict = result.behind ? "behind" : "current";
    mkdirSync(o.out, { recursive: true });
    writeFileSync(join(o.out, "verdict.txt"), `${verdict}\n`);
    if (result.behind) writeFileSync(join(o.out, "body.md"), renderBody({ storeVersion, listed, result, maxReleases: o.maxReleases, maxDays: o.maxDays }));
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `verdict=${verdict}\n`);
    const has = storeVersion ? `Store serves ${showVersion(storeVersion)}` : `Store listing last updated ${fmt(listed)} (package version unreadable)`;
    console.log(`${has}; ${result.uncovered.length} newer release(s); newest release ${releases[0].tag}: ${verdict}`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
    main().catch((e) => {
        console.error(`::error::could not read the Store or the releases: ${e.message}`);
        process.exit(2);
    });
}
