// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A container agent's CPU and memory limits as the panel edits them
 * (docs/specs/SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md §3, §6). srv stores a limit as text: "" is auto
 * (the app-wide setting, then half of what Docker has), "0" is no limit, anything else is a number: MB of memory,
 * CPUs. The panel shows memory in GB and says what is in force.
 */

export type LimitMode = "auto" | "custom" | "none";

export interface LimitDraft {
    mode: LimitMode;
    /** What the box holds in `custom`: GB of memory, CPUs. */
    amount: string;
}

const MB_PER_GB = 1024;

/** `n` without float noise: 2, 1.5, 0.25. */
function plain(n: number): string {
    return String(Math.round(n * 100) / 100);
}

/** The draft for a stored value; `unit` is how many stored units make one shown unit (1024 for MB to GB, 1 for CPUs). */
export function draftFromStored(stored: string, unit: number): LimitDraft {
    const text = stored.trim();
    if (text === "" || text.toLowerCase() === "auto") return { mode: "auto", amount: "" };
    const n = Number(text);
    if (!Number.isFinite(n) || n === 0) return { mode: "none", amount: "" };
    return { mode: "custom", amount: plain(n / unit) };
}

/** The text to store for a draft, or why it can't be stored. */
export function storedFromDraft(draft: LimitDraft, unit: number, what: string): { value: string } | { error: string } {
    if (draft.mode === "auto") return { value: "" };
    if (draft.mode === "none") return { value: "0" };
    const n = Number(draft.amount.trim());
    if (draft.amount.trim() === "" || !Number.isFinite(n) || n <= 0) return { error: `${what} must be a number above 0` };
    return { value: plain(n * unit) };
}

export const memoryDraft = (stored: string): LimitDraft => draftFromStored(stored, MB_PER_GB);
export const cpuDraft = (stored: string): LimitDraft => draftFromStored(stored, 1);
export const memoryStored = (d: LimitDraft) => storedFromDraft(d, MB_PER_GB, "Memory");
export const cpuStored = (d: LimitDraft) => storedFromDraft(d, 1, "CPUs");

/** `8192` as "8 GB", `1536` as "1.5 GB", `256` as "256 MB". */
export function memoryText(mb: number): string {
    if (mb < MB_PER_GB) return `${mb} MB`;
    return `${plain(mb / MB_PER_GB)} GB`;
}

/** "6 of 8 CPUs", "1 CPU". */
export function cpuText(cpus: number, of?: number): string {
    const unit = (n: number) => `${plain(n)} ${n === 1 ? "CPU" : "CPUs"}`;
    return of != null && of > 0 ? `${plain(cpus)} of ${unit(of)}` : unit(cpus);
}

/** What the daemon has, as the settings explain it. Docker Desktop's own cap is outside AgentMux. */
const SMALL_DAEMON_MB = 4 * MB_PER_GB;

/**
 * The line under the memory box that says what is in force, and (when Docker has little, or the box asks for more
 * than it has) why a limit alone can't help. `effectiveMb` absent means no limit, or Docker couldn't be asked.
 */
export function memoryNotes(draft: LimitDraft, effectiveMb: number | undefined, daemonMb: number | undefined): string[] {
    const notes: string[] = [];
    if (effectiveMb != null) {
        const of = daemonMb != null ? ` of the ${memoryText(daemonMb)} Docker has` : "";
        notes.push(`In force now: ${memoryText(effectiveMb)}${of}.`);
    } else if (draft.mode === "none") {
        notes.push("No limit: a build can use all the memory Docker has.");
    }
    const asked = draft.mode === "custom" ? Number(draft.amount) * MB_PER_GB : NaN;
    if (daemonMb != null && Number.isFinite(asked) && asked > daemonMb) {
        notes.push(`Docker has ${memoryText(daemonMb)} in total, so more than that has no effect.`);
    }
    if (daemonMb != null && daemonMb < SMALL_DAEMON_MB) {
        notes.push(
            `Docker has ${memoryText(daemonMb)} in total, so builds that need more will fail whatever this limit says. ` +
                "Raise it in Docker Desktop, Settings, Resources."
        );
    }
    return notes;
}

/** The CPU box's line: what is in force. */
export function cpuNotes(draft: LimitDraft, effectiveCpus: number | undefined, daemonCpus: number | undefined): string[] {
    if (effectiveCpus != null) return [`In force now: ${cpuText(effectiveCpus, daemonCpus)}.`];
    if (draft.mode === "none") return ["No limit: a build can use every CPU Docker has."];
    return [];
}
