// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an App skill's card and detail say about it (docs/specs/SPEC_PACKS_2026_11_05.md §8), worked out
 * from the manifest and the pass records srv sends with the skill. Pure: no RPC, no signals.
 */

import type { AppSkillCatalogEntry } from "@/types/rpc/AppSkillCatalogEntry";
import type { AppSkillCatalogItem } from "@/types/rpc/AppSkillCatalogItem";
import type { AppSkillHere } from "@/types/rpc/AppSkillHere";
import type { SkillApp } from "@/types/rpc/SkillApp";
import type { SkillCatalogItem } from "@/types/rpc/SkillCatalogItem";
import type { VerifiedRecord } from "@/types/rpc/VerifiedRecord";
import type { WidgetSignatureInfo } from "@/types/rpc/WidgetSignatureInfo";
import { getPlatform } from "@/util/platformutil";

export type AppFilter = "all" | "app";
export type AppTone = "ok" | "warn" | "idle" | "error";

export interface AppStatus {
    tone: AppTone;
    /** The short line a card shows: "verified 16.0", "not tested here". */
    label: string;
}

/** What an installed skill says when it is exactly what the AgentMux catalog listed (the widget catalog's line, for skills). */
export const FROM_CATALOG_LINE = "From the AgentMux catalog: its files and its publisher's key are the ones the catalog lists.";

/** The manifest's platform name for this machine. */
export function appPlatform(): "windows" | "macos" | "linux" {
    const p = getPlatform();
    return p === "win32" ? "windows" : p === "darwin" ? "macos" : "linux";
}

const OS_NAMES: Record<string, string> = { windows: "Windows", macos: "macOS", linux: "Linux" };

/** The platforms as a person names them ("Windows or macOS"). */
export function osNames(platforms: string[]): string {
    return platforms.map((p) => OS_NAMES[p] ?? p).join(" or ");
}

/** A catalog entry that lists platforms and not this one: it is for another OS, so there is nothing to install here. */
export function forAnotherOs(entry: AppSkillCatalogEntry, platform: string): boolean {
    return entry.platforms.length > 0 && !entry.platforms.includes(platform);
}

export const isAppSkill = (item: SkillCatalogItem): boolean => item.app != null;

export function filterSkills(items: SkillCatalogItem[], filter: AppFilter): SkillCatalogItem[] {
    return filter === "app" ? items.filter(isAppSkill) : items;
}

/** Records for the installed skill version on one platform, newest last (the date, then the order stored). */
export function recordsFor(app: SkillApp, platform: string): VerifiedRecord[] {
    return app.verified
        .map((r, i) => ({ r, i }))
        .filter(({ r }) => r.platform === platform && r.skillVersion === app.version)
        .sort((a, b) => (a.r.date === b.r.date ? a.i - b.i : a.r.date < b.r.date ? -1 : 1))
        .map(({ r }) => r);
}

/**
 * Whether this version of the skill was tested on this platform: the newest record decides, because a pass
 * followed by a fail means the skill stopped working. A pass elsewhere is said, not counted.
 */
export function appStatus(app: SkillApp, platform: string, onThisComputer?: AppSkillHere): AppStatus {
    // A skill for another platform's app is not "untested here": it does not apply here.
    if (app.platforms.length > 0 && !app.platforms.includes(platform)) {
        return { tone: "idle", label: `for ${osNames(app.platforms)} only` };
    }
    const here = recordsFor(app, platform);
    const last = here[here.length - 1];
    if (last) {
        if (last.result !== "pass") {
            return { tone: "error", label: `failed on ${last.appVersion}` };
        }
        // Drift: the app here is not the version the last pass was run against. Whether it still works is untested.
        return onThisComputer?.found && onThisComputer.version && !isBuildOf(onThisComputer.version, last.appVersion)
            ? { tone: "warn", label: `tested on ${last.appVersion}, installed ${onThisComputer.version}` }
            : { tone: "ok", label: `verified ${last.appVersion}` };
    }
    const elsewhere = app.verified.find((r) => r.skillVersion === app.version && r.result === "pass");
    return { tone: "idle", label: elsewhere ? `tested on ${osNames([elsewhere.platform])} only` : "not tested yet" };
}

/** The app version the newest pass on this platform was run against, if any. */
export function lastPassedAppVersion(app: SkillApp, platform: string): string | null {
    const passes = recordsFor(app, platform).filter((r) => r.result === "pass");
    return passes.length > 0 ? passes[passes.length - 1].appVersion : null;
}

/**
 * Reference crops against the app version they are meant for: "3 of 9 references were taken on 12.0" when the
 * app on this computer is 12.1 (`installed`, from `app_skills.here`), else when the last pass was on 12.1. A crop
 * taken on 12.1 still fits 12.1.5. Null when there are no crops, none is stale, or there is nothing to compare with.
 */
export function referenceDrift(app: SkillApp, platform: string, installed?: string | null): string | null {
    const current = installed || lastPassedAppVersion(app, platform);
    const here = app.references.filter((r) => r.platform === platform);
    if (!current || here.length === 0) return null;
    const older = here.filter((r) => !isBuildOf(current, r.appVersion));
    if (older.length === 0) return null;
    const versions = [...new Set(older.map((r) => r.appVersion))].join(", ");
    return `${older.length} of ${here.length} references were taken on ${versions}; ${installed ? `${app.name} here is` : "the last pass was on"} ${current}`;
}

/** The route the skill tries first ("api", "mcp", "shortcuts" or "desktop"), as a card fact. */
export function firstRoute(app: SkillApp): string | null {
    return app.ladder.length > 0 ? app.ladder[0].kind : null;
}

/** A file size the way the detail lists it. */
export function fileSize(bytes: number): string {
    return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KB`;
}

/** A script's state in the detail: a script is never given to an agent until a person approves it. */
export function scriptState(f: { script: boolean; approved: boolean }): string | null {
    return f.script ? (f.approved ? "approved" : "not approved") : null;
}

/** The tone a pill takes for a status (a status dot says "idle" where a pill says "neutral"). */
export const pillTone = (tone: AppTone): "ok" | "warn" | "neutral" | "error" => (tone === "idle" ? "neutral" : tone);

/** Whether the skill ships a test task (`test.json`) for this platform: the one thing Run test needs. */
export function hasTestTask(app: SkillApp, platform: string): boolean {
    const forHere = app.platforms.length === 0 || app.platforms.includes(platform);
    return forHere && app.files.some((f) => f.path === "test.json");
}

/** The task Run test files: its title, and the instruction the agent that takes it acts on alone (SPEC_PACKS §7). */
export function testTask(app: SkillApp, skillId: string, platform: string, here: AppSkillHere | undefined): { title: string; payload: string } {
    const where = osNames([platform]);
    const found =
        here?.found && here.version
            ? `AgentMux found ${app.name} ${here.version} on this computer; confirm the version in the app before you record it.`
            : "Take the app's version from the app itself.";
    return {
        title: `Run the test of ${app.name} ${app.version} on ${where}`,
        payload: [
            `Run the test of the computer-use skill "${app.name}" ${app.version} on ${where}. This is a normal task: you drive the app with the Desktop tool and the person's own per-app grant, so their prompt and the audit log are the usual ones. Do not change the skill.`,
            `1. Read the test: Skills get skill_id=${skillId} file=test.json. Read SKILL.md and the files it points at the same way (Skills get skill_id=${skillId}, then file=...).`,
            `2. ${found}`,
            "3. Do each step by the skill's ladder, best route first, and check every verify cue. Never mark a cue passed that you did not check.",
            `4. Record the result: Skills record skill_id=${skillId} skill_version=${app.version} app_version=<the version you confirmed> platform=${platform} result=pass or fail route=<the rung you used> notes=<what you saw, short; no host or user name> evidence_sha256=<SHA-256 of the final screenshot or output file, never the file>.`,
            "5. Say in your result what passed, what failed and where. The task is done whether the test passed or not. If the app is not installed, or Desktop is not enabled for it, say so and release the task instead of recording a result.",
        ].join("\n"),
    };
}

/**
 * The application itself on this computer, from `app_skills.here`: short for a card ("installed 12.2"), longer for the
 * detail. `null` while it is not known (the answer has not come, or the skill lists no executable for this platform).
 */
export function appHereFact(app: SkillApp, here: AppSkillHere | undefined, long: boolean): string | null {
    if (!here) return null;
    if (!here.found) return long ? `${app.name} is not found on this computer.` : "not found here";
    if (!here.version) return long ? `${app.name} is on this computer; its version could not be read.` : "found here";
    return long ? `${app.name} on this computer: version ${here.version}.` : `installed ${here.version}`;
}

/**
 * Who signed the skill, in the words the detail and the import preview use. A key that is not the one this
 * publisher's scripts were approved under is a different author, not an update.
 */
export function describeSignature(sig: WidgetSignatureInfo): { text: string; tone: "ok" | "neutral" | "warn" } {
    const who = sig.publisher;
    switch (sig.state) {
        case "signed":
            return { tone: "ok", text: `Signed by ${who} · ${sig.fingerprint} (the key you approved ${who}'s scripts under)` };
        case "signed_new":
            return { tone: "neutral", text: `Signed by ${who} · ${sig.fingerprint}. Nothing from ${who} is approved here yet: check the key with its author if it matters.` };
        case "key_changed":
            return {
                tone: "warn",
                text: sig.fingerprint
                    ? `Signed by ${sig.fingerprint}, not the key ${who} is pinned to (${sig.pinned}). It may not be from the same author.`
                    : `Not signed, though ${who}'s other skills are (${sig.pinned}). It may not be from the same author.`,
            };
        default:
            return { tone: "neutral", text: "Not signed: AgentMux can't tell who made it." };
    }
}

/**
 * What a catalog entry says about itself on this platform, from the tests the catalog records for this version of
 * the skill: the newest decides, the same as for an installed skill. "Not tested yet" is a normal state.
 */
export function catalogStatus(entry: AppSkillCatalogEntry, platform: string): AppStatus {
    if (forAnotherOs(entry, platform)) {
        return { tone: "idle", label: `for ${osNames(entry.platforms)} only` };
    }
    const mine = entry.verification.filter((r) => r.skillVersion === entry.version);
    const here = mine
        .map((r, i) => ({ r, i }))
        .filter(({ r }) => r.platform === platform)
        .sort((a, b) => (a.r.date === b.r.date ? a.i - b.i : a.r.date < b.r.date ? -1 : 1))
        .map(({ r }) => r);
    const last = here[here.length - 1];
    if (last) {
        return last.result === "pass" ? { tone: "ok", label: `verified ${last.appVersion}` } : { tone: "error", label: `failed on ${last.appVersion}` };
    }
    const elsewhere = mine.find((r) => r.result === "pass");
    return { tone: "idle", label: elsewhere ? `tested on ${osNames([elsewhere.platform])} only` : "not tested yet" };
}

/** Whether `found` ("16.0.18227.20162") is the tested version `tested` ("16.0") or a build of it: whole parts only, a missing trailing part of `found` counting as 0 ("16" is a build of "16.0"). */
export function isBuildOf(found: string, tested: string): boolean {
    const f = found.split(".");
    return tested.split(".").every((part, i) => (f[i] ?? "0") === part);
}

/**
 * What Browse says about the application itself on this computer, from the executables the entry lists for this
 * platform: its version, set against the version the entry was last tested on here (the drift wording the card
 * uses). `null` when the entry lists none for this platform, or is for another OS, where there is nothing to look for.
 */
export function appHereLine(item: AppSkillCatalogItem, platform: string): string | null {
    const names = item.entry.app.executables?.[platform];
    if (forAnotherOs(item.entry, platform) || !names?.length) {
        return null;
    }
    if (!item.app_found) {
        return `${item.entry.name} is not found on this computer.`;
    }
    if (!item.app_version) {
        return `${item.entry.name} is on this computer; its version could not be read.`;
    }
    const here = `${item.entry.name} on this computer: version ${item.app_version}`;
    const passes = item.entry.verification.filter((r) => r.skillVersion === item.entry.version && r.platform === platform && r.result === "pass");
    const last = passes.reduce<(typeof passes)[number] | undefined>((a, r) => (!a || r.date >= a.date ? r : a), undefined);
    if (!last) {
        return `${here}.`;
    }
    return isBuildOf(item.app_version, last.appVersion)
        ? `${here}, the version it was tested on.`
        : `${here}. The last test passed on ${last.appVersion}: check before trusting it.`;
}
