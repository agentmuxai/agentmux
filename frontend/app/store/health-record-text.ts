// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The words for what the rules that read the long-run record have flagged (the `record-flags` event, kind
 * `record` in the status bar's health store): a sentence per flag, then where the days behind it are.
 * A flag is a note: nothing was stopped, and the notice says so.
 */

import { t } from "@/app/i18n/i18n";
import type { RecordFlagLine } from "@/types/rpc/RecordFlagLine";

const MB = 1024 * 1024;

export function recordLabel(): string {
    return t("healthRecord.label");
}

/** A size in what it is counted in: bytes are said in MB. */
function amount(unit: string | null, value: number | null): string {
    const n = value ?? 0;
    if (unit === "bytes") return t("healthRecord.mb", { n: Math.round(n / MB) });
    if (unit === "mb") return t("healthRecord.mb", { n });
    return t("healthRecord.elements", { n });
}

/** One flag as a sentence; empty for a rule this build does not know. */
export function recordFlagText(flag: RecordFlagLine): string {
    const window = flag.key === "ui_stall";
    const secs = Math.round((flag.worst_ms ?? 0) / 1000);
    if (flag.rule === "stalls") {
        const n = flag.n ?? 0;
        return window ? t("healthRecord.stalls.window", { n, secs }) : t("healthRecord.stalls.backend", { n, secs });
    }
    if (flag.rule === "long_stall") {
        return window ? t("healthRecord.longStall.window", { secs }) : t("healthRecord.longStall.backend", { secs });
    }
    if (flag.rule === "growth") {
        const sizes = { rise: amount(flag.unit, flag.rise), level: amount(flag.unit, flag.level) };
        if (!flag.key.startsWith("page:")) return t("healthRecord.growth.core", sizes);
        return flag.key.endsWith(":dom") ? t("healthRecord.growth.pageElements", sizes) : t("healthRecord.growth.pageHeap", sizes);
    }
    return "";
}

/** The whole notice: each flag, then that nothing was stopped and where to look. */
export function recordMessage(flags: RecordFlagLine[] | undefined): string {
    const said = (flags ?? []).map(recordFlagText).filter((s) => s !== "");
    return [...said, t("healthRecord.tail")].join(" ");
}
