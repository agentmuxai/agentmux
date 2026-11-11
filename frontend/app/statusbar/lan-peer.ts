// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the network pill's peer rows say about a peer, apart from drawing it.

import type { LanInstance } from "@/types/rpc/LanInstance";
import type { LanPinInfo } from "@/types/rpc/LanPinInfo";
import type { RefusedKeyInfo } from "@/types/rpc/RefusedKeyInfo";
import { formatRelativeTime } from "@/util/format-time";

export interface PeerKeyRefusal {
    agent: string;
    refused: RefusedKeyInfo;
}

/**
 * The agents of `peer` for whose name a message was refused because it came with a different key
 * than the one pinned. A pin is by name, and a peer lists the names it hosts, so the match is the
 * lower-cased name. Newest refusal first.
 */
export function peerKeyRefusals(peer: LanInstance, pins: readonly LanPinInfo[]): PeerKeyRefusal[] {
    const hosted = new Set(peer.agents.map((a) => a.toLowerCase()));
    const out: PeerKeyRefusal[] = [];
    for (const pin of pins) {
        if (pin.refused && hosted.has(pin.agent_id.toLowerCase())) {
            out.push({ agent: pin.agent_id, refused: pin.refused });
        }
    }
    return out.sort((a, b) => b.refused.last_seen_at - a.refused.last_seen_at);
}

/** "now", "42 sec. ago", "5 min. ago", "3 hr. ago", "2 days ago" for a time in epoch seconds. */
export function lastSeenText(seenSecs: number, nowSecs: number, locale?: string): string {
    return formatRelativeTime(seenSecs * 1000, nowSecs * 1000, { locale });
}

const OS_LABELS: Record<string, string> = { windows: "Windows", macos: "macOS", linux: "Linux" };

const INSTALL_LABELS: Record<string, string> = {
    "windows-store": "Microsoft Store",
    "windows-setup": "installed",
    "windows-portable": "portable",
    "macos-app": "app",
    "linux-appimage": "AppImage",
    "linux-portable": "portable",
    "linux-package": "package",
};

/** How a peer is installed, in words ("Microsoft Store", "installed", "portable"); empty when it did not say. */
export function installLabel(install: string | undefined): string {
    return (install && INSTALL_LABELS[install]) || "";
}

const BUILD_LABELS: Record<string, string> = { release: "release", local: "local build", dev: "dev build" };

/**
 * What kind of build a peer says it is ("release", "local build", "dev build"); empty when it did not say.
 * The peer reads it off its own binary. It is never worked out from the channel's name: a release that a
 * local build was upgraded to keeps that build's channel.
 */
export function buildLabel(build: string | undefined): string {
    return (build && BUILD_LABELS[build]) || "";
}

/** "Windows · release · Microsoft Store": the peer's OS, kind of build and install, leaving out what it did not advertise. */
export function peerRuns(peer: Pick<LanInstance, "os"> & { build?: string; install?: string }): string {
    const os = OS_LABELS[peer.os] ?? peer.os;
    return [os, buildLabel(peer.build), installLabel(peer.install)].filter((p) => p).join(" · ");
}

/** Whether a peer that has been asked who it runs says it runs nobody (an install with no agents). */
export function hasNoAgents(peer: Pick<LanInstance, "agents" | "last_polled_ok">): boolean {
    return peer.last_polled_ok > 0 && peer.agents.length === 0;
}

const hostKey = (hostname: string): string => hostname.trim().toLowerCase();

/**
 * The hosts (lower-cased) the list shows more than once, so their rows say which channel each is.
 * `selfHost` is this machine: its own instance is not a row, but a peer on the same host is its
 * second instance, so that peer is told apart too.
 */
export function hostsWithSeveralInstances(peers: readonly Pick<LanInstance, "hostname">[], selfHost?: string): Set<string> {
    const counts = new Map<string, number>();
    const self = selfHost ? hostKey(selfHost) : "";
    if (self) counts.set(self, 1);
    for (const peer of peers) {
        const key = hostKey(peer.hostname);
        if (key) counts.set(key, (counts.get(key) ?? 0) + 1);
    }
    return new Set([...counts].filter(([, n]) => n > 1).map(([host]) => host));
}

/** Whether `peer`'s row names its channel: only when its host runs more than one instance. */
export function showsChannel(peer: Pick<LanInstance, "hostname" | "channel">, shared: ReadonlySet<string>): boolean {
    return !!peer.channel && shared.has(hostKey(peer.hostname));
}

/** `text` cut in the middle to at most `max` characters ("local-39…8424"); the whole of it is in the tooltip. */
export function middleTruncate(text: string, max: number): string {
    if (text.length <= max || max < 3) return text;
    const keep = max - 1;
    const head = Math.ceil(keep / 2);
    return `${text.slice(0, head)}…${text.slice(text.length - (keep - head))}`;
}
