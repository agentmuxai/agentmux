// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Toolchain pane's cards say, as plain functions of a row's facts:
 * the status dot, the line under the name, and the short notes in the footer.
 * Kept apart from the view so the wording can be tested without rendering.
 */

import type { StatusTone } from "@/app/element/ui";
import type { DriftResult } from "@/app/view/agent/providers/version-drift";

export interface ToolFacts {
    loading: boolean;
    found: boolean;
    optional?: boolean;
    version?: string;
    source?: string;
    kind: "core" | "local" | "provider";
}

export interface ToolStatus {
    tone: StatusTone;
    /** The dot's tooltip and spoken name. */
    label: string;
    /** The line under the card's name. */
    subtitle: string;
    /** A required tool that is missing, or one the user can update: the card asks for the user. */
    attention: boolean;
}

/** The card's dot and subtitle for a tool, given how its version compares with the pin. */
export function toolStatus(row: ToolFacts, drift: DriftResult): ToolStatus {
    if (row.loading) return { tone: "busy", label: "Checking", subtitle: "Checking…", attention: false };
    if (!row.found) {
        return row.optional
            ? { tone: "idle", label: "Not installed (optional)", subtitle: "Optional", attention: false }
            : { tone: "warn", label: "Not found", subtitle: "Not found", attention: true };
    }
    const subtitle = row.version ?? "Installed";
    if (drift.drift === "behind-pin") {
        return { tone: "warn", label: `Update available: ${drift.pinned}`, subtitle, attention: false };
    }
    return { tone: "ok", label: "Installed", subtitle, attention: false };
}

/** A footer note and, where it needs one, the longer explanation for its tooltip. */
export interface ToolNote {
    text: string;
    title?: string;
}

/**
 * The footer notes of an installed tool: where it came from, and how it sits
 * against the version AgentMux validated. A pin that trails upstream is ours to
 * bump, so it is shown quietly and never as something to do.
 */
export function toolNotes(row: ToolFacts, drift: DriftResult): ToolNote[] {
    if (row.loading || !row.found) return [];
    const notes: ToolNote[] = [];
    if (row.source === "local_install") notes.push({ text: "managed" });
    if (row.kind === "provider" && row.source === "system_path") {
        notes.push({
            text: "your install",
            title: "AgentMux doesn't install this CLI: it runs the copy you installed",
        });
    }
    if (drift.drift === "behind-pin") {
        notes.push({
            text: `${drift.pinned} available`,
            title: `AgentMux is validated against ${drift.pinned}; you have ${drift.installed}`,
        });
    } else if (drift.drift === "current") {
        notes.push({ text: "up to date" });
    } else if (drift.drift === "ahead-of-pin") {
        notes.push({
            text: "ahead of pin",
            title: `Newer than the ${drift.pinned} AgentMux validates against — not a problem, just untested here`,
        });
    }
    if (drift.currency === "pin-behind-upstream") {
        notes.push({
            text: `pin ${drift.pinned} · upstream ${drift.latest}`,
            title: `AgentMux pins ${drift.pinned}; upstream has ${drift.latest}. Nothing for you to do — this is ours to bump.`,
        });
    }
    return notes;
}

export interface WidgetFacts {
    cliLoading: boolean;
    cliFound: boolean;
    healthLoading: boolean;
    running: boolean;
    installKind: "pip" | "npm" | "manual";
}

/** The card's dot and subtitle for an external widget. */
export function widgetStatus(w: WidgetFacts, port: number): { tone: StatusTone; label: string; subtitle: string } {
    if (w.running) return { tone: "ok", label: "Running", subtitle: `Running on port ${port}` };
    if (w.cliLoading || w.healthLoading) return { tone: "busy", label: "Checking", subtitle: "Checking…" };
    if (w.cliFound) return { tone: "idle", label: "Installed, not running", subtitle: "Installed, not running" };
    const text = w.installKind === "manual" ? "Not detected" : "Not installed";
    return { tone: "idle", label: text, subtitle: text };
}

/** A port as typed; null when it is not a whole number from 1 to 65535. */
export function parsePort(raw: string): number | null {
    if (!/^\d{1,5}$/.test(raw.trim())) return null;
    const n = parseInt(raw, 10);
    return n >= 1 && n <= 65535 ? n : null;
}
