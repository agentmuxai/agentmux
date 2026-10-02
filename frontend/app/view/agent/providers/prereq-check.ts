// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pre-launch prereq decisions for a provider's `systemPrereqs`, kept pure so
 * they can be tested without the picker. A prereq blocks launch when its tool
 * is missing from PATH, or when it declares a `minVersion` and the installed
 * version is known to be older. An unreadable version doesn't block, the same
 * way a failed probe doesn't (the CLI's own error is the fallback).
 */

import type { SystemPrereq } from "./types";
import { compareVersions, normalizeVersion } from "./version-drift";

/** One tool's probe result. `version` is `undefined` when it wasn't asked for
 *  and `null` when it was asked for but couldn't be read. */
export interface PrereqProbe {
    found: boolean;
    version?: string | null;
}

export type PrereqShortfall = { kind: "missing" } | { kind: "too-old"; found: string; min: string };

/** Whether `req` needs a (re)probe given what's cached for its tool. A cached
 *  path-only answer isn't enough once some provider needs the version. */
export function needsProbe(req: SystemPrereq, cached: PrereqProbe | undefined): boolean {
    if (!cached) return true;
    return !!req.minVersion && cached.found && cached.version === undefined;
}

export function prereqShortfall(req: SystemPrereq, probe: PrereqProbe | undefined): PrereqShortfall | null {
    if (!probe) return null;
    if (!probe.found) return { kind: "missing" };
    if (!req.minVersion) return null;
    const found = normalizeVersion(probe.version ?? undefined);
    if (!found) return null;
    return compareVersions(found, req.minVersion) < 0 ? { kind: "too-old", found, min: req.minVersion } : null;
}

/** The modal row label: the tool's label, plus the version gap when it's too old. */
export function shortfallLabel(req: SystemPrereq, shortfall: PrereqShortfall): string {
    const label = req.label ?? req.tool;
    return shortfall.kind === "too-old" ? `${label} ${shortfall.min}+ (you have ${shortfall.found})` : label;
}
