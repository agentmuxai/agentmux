// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pure parts of Settings, Agents on sites, page permissions: what a kind is called beside its checkbox,
// and a list of kinds with one switched.

/** What a kind is called beside its checkbox. */
export const PERMISSION_LABELS: Record<string, string> = {
    camera: "Camera",
    microphone: "Microphone",
    location: "Location",
    notifications: "Notifications",
};

/** `kinds` with `kind` on or off, each listed once and in the order srv lists them. */
export function withKind(kinds: readonly string[], kind: string, on: boolean): string[] {
    const next = new Set(kinds);
    if (on) next.add(kind);
    else next.delete(kind);
    return Object.keys(PERMISSION_LABELS).filter((k) => next.has(k));
}
