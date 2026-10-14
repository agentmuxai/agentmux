// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The default-shell picker's wording and selection, without the store, so it
// is tested on its own (default-shell.tsx renders it).

import type { ShellInfo, ShellList } from "@/app/store/rpc-api/shells";

/** The dropdown entry that reveals the path and arguments fields. */
export const CUSTOM = "custom";

/** "PowerShell 7.6.6", or just the name when it gave no version. */
export function shellLabel(s: ShellInfo): string {
    return s.version ? `${s.name} ${s.version}` : s.name;
}

/** Same file, as Windows compares paths: either slash, any case. */
function samePath(a: string, b: string): boolean {
    const norm = (p: string) => p.replace(/\//g, "\\").toLowerCase();
    return norm(a) === norm(b);
}

/** Which dropdown entry a `term:localshellpath` selects: "" (Automatic), a listed shell's path, or Custom. */
export function selectedEntry(setting: string | undefined, shells: ShellInfo[]): string {
    const value = (setting ?? "").trim();
    if (!value) return "";
    return shells.find((s) => samePath(s.path, value))?.path ?? CUSTOM;
}

/** The Automatic entry's text: what it resolves to on this machine now. */
export function automaticLabel(list: ShellList | null): string {
    if (!list) return "Automatic";
    const shell = list.shells.find((s) => samePath(s.path, list.automatic));
    return `Automatic: ${shell ? shellLabel(shell) : list.automatic}`;
}

/** The line under the dropdown: what new terminals open with, and why. */
export function describeEffective(list: ShellList): { text: string; warn: boolean } {
    const e = list.effective;
    const shell = list.shells.find((s) => samePath(s.path, e.path));
    const what = shell ? `${shellLabel(shell)} (${e.path})` : e.path;
    switch (e.source) {
        case "fallback":
            return { text: `${e.missing} wasn't found, so new terminals open with ${what}.`, warn: true };
        case "automatic":
            return { text: `New terminals open with ${what}, chosen automatically.`, warn: false };
        default:
            return { text: `New terminals open with ${what}.`, warn: false };
    }
}
