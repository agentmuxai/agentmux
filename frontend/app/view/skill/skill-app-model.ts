// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an App skill's card and detail say about it (docs/specs/SPEC_PACKS_2026_11_05.md §8), worked out
 * from the manifest and the pass records srv sends with the skill. Pure: no RPC, no signals.
 */

import type { AppSkillCatalogEntry } from "@/types/rpc/AppSkillCatalogEntry";
import type { SkillApp } from "@/types/rpc/SkillApp";
import type { SkillCatalogItem } from "@/types/rpc/SkillCatalogItem";
import type { VerifiedRecord } from "@/types/rpc/VerifiedRecord";
import type { WidgetSignatureInfo } from "@/types/rpc/WidgetSignatureInfo";
import { getPlatform } from "@/util/platformutil";

export type AppFilter = "all" | "app";
export type AppTone = "ok" | "idle" | "error";

export interface AppStatus {
    tone: AppTone;
    /** The short line a card shows: "verified 16.0", "not tested here". */
    label: string;
}

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
export function appStatus(app: SkillApp, platform: string): AppStatus {
    // A skill for another platform's app is not "untested here": it does not apply here.
    if (app.platforms.length > 0 && !app.platforms.includes(platform)) {
        return { tone: "idle", label: `for ${app.platforms.join(", ")} only` };
    }
    const here = recordsFor(app, platform);
    const last = here[here.length - 1];
    if (last) {
        return last.result === "pass"
            ? { tone: "ok", label: `verified ${last.appVersion}` }
            : { tone: "error", label: `failed on ${last.appVersion}` };
    }
    const elsewhere = app.verified.find((r) => r.skillVersion === app.version && r.result === "pass");
    return { tone: "idle", label: elsewhere ? `tested on ${elsewhere.platform} only` : "not tested yet" };
}

/** The app version the newest pass on this platform was run against, if any. */
export function lastPassedAppVersion(app: SkillApp, platform: string): string | null {
    const passes = recordsFor(app, platform).filter((r) => r.result === "pass");
    return passes.length > 0 ? passes[passes.length - 1].appVersion : null;
}

/**
 * Reference crops against the last tested app version: "3 of 9 references were taken on 12.0" when the last
 * pass was on 12.1. Null when there are no crops, or none is stale (or nothing passed yet to compare with).
 */
export function referenceDrift(app: SkillApp, platform: string): string | null {
    const current = lastPassedAppVersion(app, platform);
    const here = app.references.filter((r) => r.platform === platform);
    if (!current || here.length === 0) return null;
    const older = here.filter((r) => r.appVersion !== current);
    if (older.length === 0) return null;
    const versions = [...new Set(older.map((r) => r.appVersion))].join(", ");
    return `${older.length} of ${here.length} references were taken on ${versions}; the last pass was on ${current}`;
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
export const pillTone = (tone: AppTone): "ok" | "neutral" | "error" => (tone === "idle" ? "neutral" : tone);

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
        return { tone: "idle", label: `for ${entry.platforms.join(", ")} only` };
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
    return { tone: "idle", label: elsewhere ? `tested on ${elsewhere.platform} only` : "not tested yet" };
}
