// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * version-drift — the one place that answers "is this tool behind, and behind
 * WHAT?"
 *
 * Three versions are in play and they are NOT interchangeable:
 *
 *   installed — what will actually run when you launch an agent
 *   pinned    — the version AgentMux validated and ships against
 *               (`pinnedVersion` in providers/catalog.ts)
 *   latest    — the newest published release (npm, via ToolchainVersionsCommand)
 *
 * Collapsing them into one "update available" boolean is the bug this module
 * exists to prevent. `installed < pinned` is the user's problem and is one
 * click to fix; `pinned < latest` is AgentMux's problem and the user can do
 * nothing about it. Before this module the toolchain pane compared installed
 * against `latest` ONLY — so it never surfaced the gap that actually breaks
 * agents (running older than what we validated), while nagging users to move
 * AHEAD of the pin onto a version we have not tested.
 *
 * Comparison is semver-aware on purpose. The previous check was
 * `row.version === row.latestVersion`, which is wrong two ways that are both
 * live today: `2.1.9` vs `2.1.10` orders incorrectly under string compare, and
 * `node --version` prints `v22.22.2` while `gemini --version` prints `0.60.0`,
 * so a bare `v` prefix alone produced a permanent false "update available".
 */

/** Drop a leading `v`, surrounding whitespace, and any build/pre-release tail. */
export function normalizeVersion(raw: string | undefined): string | undefined {
    if (!raw) return undefined;
    const trimmed = raw.trim().replace(/^v/i, "");
    // Take the leading dotted-numeric run: "0.60.0 (Gemini)" -> "0.60.0".
    const m = trimmed.match(/^\d+(?:\.\d+)*/);
    return m ? m[0] : undefined;
}

/**
 * Semver-ish ordering over dotted numeric versions.
 * Returns <0 if a<b, 0 if equal, >0 if a>b. Missing segments count as 0, so
 * `2.1` and `2.1.0` compare equal.
 */
export function compareVersions(a: string, b: string): number {
    const pa = a.split(".").map((n) => Number.parseInt(n, 10) || 0);
    const pb = b.split(".").map((n) => Number.parseInt(n, 10) || 0);
    const len = Math.max(pa.length, pb.length);
    for (let i = 0; i < len; i++) {
        const d = (pa[i] ?? 0) - (pb[i] ?? 0);
        if (d !== 0) return d < 0 ? -1 : 1;
    }
    return 0;
}

/**
 * How the installed version relates to the version AgentMux validated.
 *
 * `behind-pin` is the only actionable-by-the-user state, and the only one that
 * should be visually loud.
 */
export type PinDrift =
    | "current" // installed === pinned
    | "behind-pin" // installed < pinned — running older than we validated
    | "ahead-of-pin" // installed > pinned — untested territory, informational
    | "not-installed"
    | "unknown"; // no pin, or an unparseable version string

/** Whether AgentMux's own pin has fallen behind what upstream publishes. */
export type PinCurrency = "pin-current" | "pin-behind-upstream" | "unknown";

export interface DriftInput {
    /** Version string as reported by the tool itself; `v` prefixes are fine. */
    installed?: string;
    /** `pinnedVersion` from the provider catalog; absent for system tools. */
    pinned?: string;
    /** Newest published version, when a registry lookup has succeeded. */
    latest?: string;
    /** False when the tool was probed and not found. */
    found: boolean;
}

export interface DriftResult {
    drift: PinDrift;
    currency: PinCurrency;
    /** Normalized values, so callers render the same strings they compared. */
    installed?: string;
    pinned?: string;
    latest?: string;
}

export function resolveDrift(input: DriftInput): DriftResult {
    const installed = normalizeVersion(input.installed);
    const pinned = normalizeVersion(input.pinned);
    const latest = normalizeVersion(input.latest);

    const currency: PinCurrency =
        pinned && latest ? (compareVersions(pinned, latest) < 0 ? "pin-behind-upstream" : "pin-current") : "unknown";

    if (!input.found) return { drift: "not-installed", currency, installed, pinned, latest };

    // No pin (every system tool) or an unparseable report: we genuinely do not
    // know, and must say so rather than guess "current" and look reassuring.
    if (!installed || !pinned) return { drift: "unknown", currency, installed, pinned, latest };

    const cmp = compareVersions(installed, pinned);
    const drift: PinDrift = cmp === 0 ? "current" : cmp < 0 ? "behind-pin" : "ahead-of-pin";
    return { drift, currency, installed, pinned, latest };
}

/** True when the user can act on this row right now. Drives the loud styling. */
export function isActionable(r: DriftResult): boolean {
    return r.drift === "behind-pin" || r.drift === "not-installed";
}
