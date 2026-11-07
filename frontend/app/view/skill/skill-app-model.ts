// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What an App skill's card and detail say about it (docs/specs/SPEC_PACKS_2026_11_05.md §8), worked out
 * from the manifest and the pass records srv sends with the skill. Pure: no RPC, no signals.
 */

import type { SkillApp } from "@/types/rpc/SkillApp";
import type { SkillCatalogItem } from "@/types/rpc/SkillCatalogItem";
import type { VerifiedRecord } from "@/types/rpc/VerifiedRecord";
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

/** A script's state in the detail: stage 4 adds approval, so until then it is stored and not given to an agent (the pill's tooltip says so). */
export function scriptState(f: { script: boolean; approved: boolean }): string | null {
    return f.script ? (f.approved ? "approved" : "not approved") : null;
}

/** The tone a pill takes for a status (a status dot says "idle" where a pill says "neutral"). */
export const pillTone = (tone: AppTone): "ok" | "neutral" | "error" => (tone === "idle" ? "neutral" : tone);
