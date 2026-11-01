// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What makes two panes of an app-scope view the same thing, and which of them hold a live process
 * (docs/reports/REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md §4): an agent pane is
 * one agent's session, an Editor pane one document, a browser pane one named profile's tab. A pane
 * with none of these is only "a pane of that view", and is looked for in this window alone.
 */

import { DOC_TABS_META } from "@/app/doc-tabs/doc-tabs";
import { canonicalizePath } from "@/app/store/editor-pane-state-store";
import { IDENTITY_META_KEY, parseIdentity } from "@/app/util/browser-identity";
import { isSshConnection } from "@/app/view/term/ssh-connection";

/**
 * The document an Editor pane shows now: its front document tab (persisted in `doctabs`), else the pre-tabs
 * `file` key. `file` alone goes stale once a tab is switched or closed, so it is only the fallback.
 */
function shownDocument(meta: Record<string, unknown> | undefined): string | undefined {
    const record = meta?.[DOC_TABS_META] as { active?: unknown; tabs?: { state?: { path?: unknown } }[] } | undefined;
    if (Array.isArray(record?.tabs) && record.tabs.length > 0) {
        const path = record.tabs[typeof record.active === "number" ? record.active : 0]?.state?.path;
        return typeof path === "string" && path.trim() ? canonicalizePath(path.trim()) : undefined;
    }
    const file = meta?.["file"];
    return typeof file === "string" && file.trim() ? canonicalizePath(file.trim()) : undefined;
}

/** The pane's identity, or `undefined` when the view has none (or this pane has not got it set). */
export function paneIdentity(view: string | undefined, meta: Record<string, unknown> | undefined): string | undefined {
    const text = (v: unknown): string | undefined => (typeof v === "string" && v.trim() ? v.trim() : undefined);
    switch (view) {
        case "agent": {
            const id = text(meta?.["agentId"]);
            return id && `agent:${id}`;
        }
        case "editor": {
            const file = shownDocument(meta);
            // The same path on another host is another document.
            const host = isSshConnection(meta?.["connection"]) ? text(meta?.["connection"]) : undefined;
            return file && (host ? `file@${host}:${file}` : `file:${file}`);
        }
        case "browser": {
            const identity = parseIdentity(meta?.[IDENTITY_META_KEY]);
            return identity.kind === "profile" ? `profile:${identity.id}` : undefined;
        }
        default:
            return undefined;
    }
}

/** Views whose pane runs a process of its own: moving one between windows would cut the session off. */
const LIVE_PROCESS_VIEWS: ReadonlySet<string> = new Set(["term", "agent"]);

export const holdsLiveProcess = (view: string | undefined): boolean => !!view && LIVE_PROCESS_VIEWS.has(view);

/**
 * Panes that can be gone to but not moved to another window: a live process would be cut off, and an Editor
 * keeps its unsaved text in its own window, which a move would drop.
 */
export const staysInItsWindow = (view: string | undefined): boolean => holdsLiveProcess(view) || view === "editor";
