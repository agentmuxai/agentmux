// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How the Remotes pane groups and labels its records
// (docs/specs/SPEC_REMOTES_PANE_2026_10_05.md §4.2). Pure: no Solid, no RPC.

import type { RemoteHelper, RemotePlatform, RemoteRecord, RemoteStatus } from "@/app/store/rpc-api/remotes";

export type RemoteSection = "pinned" | "ssh" | "recent" | "wsl" | "hidden";

export const SECTION_ORDER: RemoteSection[] = ["pinned", "ssh", "recent", "wsl", "hidden"];

export const SECTION_LABELS: Record<RemoteSection, string> = {
    pinned: "Pinned",
    ssh: "SSH hosts",
    recent: "Recent",
    wsl: "WSL",
    hidden: "Hidden",
};

/** The six colours a remote can carry (plus none). */
export const REMOTE_COLORS: { value: string; name: string }[] = [
    { value: "#e5484d", name: "Red" },
    { value: "#f76b15", name: "Orange" },
    { value: "#ffc53d", name: "Yellow" },
    { value: "#46a758", name: "Green" },
    { value: "#0090ff", name: "Blue" },
    { value: "#8e4ec6", name: "Purple" },
];

function setting<T>(r: RemoteRecord, key: string): T | undefined {
    return r.settings?.[key] as T | undefined;
}

export function isPinned(r: RemoteRecord): boolean {
    return setting<boolean>(r, "display:pinned") === true;
}

export function isHidden(r: RemoteRecord): boolean {
    return setting<boolean>(r, "display:hidden") === true;
}

/** The nickname if set, else the connection name. */
export function displayName(r: RemoteRecord): string {
    const nick = setting<string>(r, "display:name");
    return nick && nick.trim() ? nick.trim() : r.name;
}

export function remoteColor(r: RemoteRecord): string | undefined {
    const c = setting<string>(r, "display:color");
    return c && /^#[0-9a-fA-F]{6}$/.test(c) ? c : undefined;
}

/** Exactly one section per remote: Pinned wins, then Hidden, then by kind and source. */
export function sectionOf(r: RemoteRecord): RemoteSection {
    if (isPinned(r)) return "pinned";
    if (isHidden(r)) return "hidden";
    if (r.kind === "wsl") return "wsl";
    if (r.sources.includes("ssh_config") || r.sources.includes("settings")) return "ssh";
    return "recent";
}

function byName(a: RemoteRecord, b: RemoteRecord): number {
    return displayName(a).localeCompare(displayName(b), undefined, { sensitivity: "base" });
}

function sortIn(section: RemoteSection, records: RemoteRecord[]): RemoteRecord[] {
    const out = [...records];
    if (section === "pinned") {
        out.sort((a, b) => {
            const oa = setting<number>(a, "display:order") ?? 0;
            const ob = setting<number>(b, "display:order") ?? 0;
            return oa !== ob ? oa - ob : byName(a, b);
        });
    } else if (section === "recent") {
        out.sort((a, b) => (b.last_used_ms ?? 0) - (a.last_used_ms ?? 0));
    } else {
        out.sort(byName);
    }
    return out;
}

export function matchesFilter(r: RemoteRecord, filter: string): boolean {
    const f = filter.trim().toLowerCase();
    if (!f) return true;
    return r.name.toLowerCase().includes(f) || displayName(r).toLowerCase().includes(f);
}

export interface RemoteGroup {
    section: RemoteSection;
    label: string;
    records: RemoteRecord[];
}

/** The sections to show, in order, each sorted; empty sections are left out. */
export function groupRemotes(records: RemoteRecord[], filter = ""): RemoteGroup[] {
    const bySection = new Map<RemoteSection, RemoteRecord[]>();
    for (const r of records) {
        if (!matchesFilter(r, filter)) continue;
        const s = sectionOf(r);
        bySection.set(s, [...(bySection.get(s) ?? []), r]);
    }
    return SECTION_ORDER.filter((s) => bySection.has(s)).map((s) => ({
        section: s,
        label: SECTION_LABELS[s],
        records: sortIn(s, bySection.get(s)!),
    }));
}

/** `Linux · x86_64`, `macOS · arm64`; empty when unknown. */
export function platformLabel(p: RemotePlatform | null | undefined): string {
    if (!p) return "";
    const os = p.os === "macos" ? "macOS" : p.os === "linux" ? "Linux" : p.os;
    return p.arch ? `${os} · ${p.arch}` : os;
}

export function helperLabel(h: RemoteHelper): string {
    switch (h.state) {
        case "installed":
            return h.version ? `Helper ${h.version}` : "Helper installed";
        case "absent":
            return "No helper";
        case "never":
            return "Helper: never";
        case "unsupported":
            return "No helper for this platform";
        default:
            return "";
    }
}

export function statusLabel(s: RemoteStatus): string {
    switch (s.state) {
        case "connected":
            return "Connected";
        case "connecting":
            return "Connecting…";
        case "error":
            return s.error ? `Error: ${s.error}` : "Error";
        default:
            return "Not connected";
    }
}

export function plural(n: number, one: string, many = `${one}s`): string {
    return `${n} ${n === 1 ? one : many}`;
}
