// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A memory file's or entry's size over time, from its version history: each
 * version's change, growth over a period and a sparkline
 * (SPEC_MEMORY_PANE_TOKENS_2026_10_15.md §5.5). Versions are newest first, as
 * the history lists them.
 */

import { formatCompactNumber } from "@/util/format-count";

export interface SizedVersion {
    created_at: number;
    tokens_est?: number;
}

const DAY_MS = 86_400_000;

/** Version `i`'s change from the one before it ("+312", "−40"); "" for the
 *  first version, an unchanged size, or a version with no count. */
export function versionDelta(versions: SizedVersion[], i: number): string {
    const now = versions[i]?.tokens_est;
    const before = versions[i + 1]?.tokens_est;
    if (now == null || before == null || now === before) return "";
    const d = now - before;
    return `${d > 0 ? "+" : "−"}${formatCompactNumber(Math.abs(d))}`;
}

/** The change in tokens over the last `days` (to `nowMs`): the newest
 *  version's size minus the size that was current at the cutoff (0 when the
 *  file is younger). `null` with no counted versions. */
export function growthOver(versions: SizedVersion[], days: number, nowMs: number): number | null {
    const latest = versions[0]?.tokens_est;
    if (latest == null) return null;
    const cutoff = nowMs - days * DAY_MS;
    const atCutoff = versions.find((v) => v.created_at <= cutoff);
    return latest - (atCutoff?.tokens_est ?? 0);
}

/** "+312 tok" / "−40 tok" / "no change" for a growth figure. */
export function growthLabel(growth: number): string {
    if (growth === 0) return "no change";
    return `${growth > 0 ? "+" : "−"}${formatCompactNumber(Math.abs(growth))} tok`;
}

/** An SVG polyline's points for the sizes, oldest left, in a `w`×`h` box;
 *  "" with fewer than two counted versions. */
export function sparklinePoints(versions: SizedVersion[], w: number, h: number): string {
    const sizes = versions
        .map((v) => v.tokens_est)
        .filter((t): t is number => t != null)
        .reverse();
    if (sizes.length < 2) return "";
    const max = Math.max(...sizes, 1);
    const step = w / (sizes.length - 1);
    return sizes.map((t, i) => `${(i * step).toFixed(1)},${(h - (t / max) * h).toFixed(1)}`).join(" ");
}
