// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What pane a destination opens and the meta it writes (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md R14).
 * A link's place goes into the same meta keys the pane's own openers and a saved layout already use
 * (`<view>:section` through `pane-sections.ts`, `kanban:open`, `vault:request`, `audit:agent`, `files:path`...), so a
 * pane opened by a link and one restored from a saved layout are the same pane. Pure: no store, no pane is touched.
 */

import { movedMeta, sectionMetaOf } from "@/app/store/pane-sections";
import type { ParsedDeepLink } from "./parse";
import { deepLinkPlace, deepLinkView, paneViewOf } from "./registry";

/** Meta keys the panes read. Each is the one the pane's own opener writes today. */
export const DEST_META = {
    settingsBoard: "settings:kanban-board",
    settingsBoardAsk: "settings:kanban-ask",
    /** One-shot: the setting row Settings scrolls to and marks, as its search does (settings-view.tsx). */
    settingsReveal: "settings:reveal",
    settingsRevealAsk: "settings:reveal-ask",
    vaultRequest: "vault:request",
    memoryAgent: "memory:agent",
    kanbanOpen: "kanban:open",
    kanbanBoard: "kanban:board",
    auditAgent: "audit:agent",
    auditSound: "audit:sound",
    auditMessage: "audit:message",
    editorFile: "file",
    editorLine: "editor:line",
    editorOffset: "editor:offset",
    editorFollowFolder: "editor:follow_folder",
    filesPath: "files:path",
    filesSelect: "files:select",
} as const;

/** What the executor opens for a destination. */
export interface PanePlan {
    /** The pane view (block `meta.view`). */
    view: string;
    /** The whole meta of a new pane, `view` included. */
    meta: Record<string, unknown>;
    /** What is merged into a pane of the view that is already open: the place, and nothing of the pane's own. */
    focusMeta: Record<string, unknown>;
    /** A setting row to reveal (Settings' anchor). */
    revealSetting?: string;
    /**
     * The place can't be shown by this build, so the view opens at its nearest place (§8): the words say which
     * part. The pane's own notice says it too where the pane has one.
     */
    missing?: string;
}

/** Why a link names no pane: a view that is not a pane (the status bar, build 2). */
export type PlanProblem = { code: "not_yet"; text: string };

function str(v: unknown): string {
    return String(v);
}

/**
 * The pane and meta `link` opens. `now` stamps a one-shot ask, so asking again for the same place is a new ask
 * (as `openSettingsAt` does).
 */
export function planDestination(link: ParsedDeepLink, now: number = Date.now()): PanePlan | PlanProblem {
    const view = deepLinkView(link.view);
    const paneView = view ? paneViewOf(view) : null;
    if (!view || paneView === null) return { code: "not_yet", text: `${link.view} is not a pane in this build` };
    const place = deepLinkPlace(link.view, link.place);
    const v = link.values;
    const p = link.params;
    let focus: Record<string, unknown> = {};
    let revealSetting: string | undefined;
    let missing: string | undefined;

    // A place whose one enum segment is the pane's section (R14).
    if (place?.sections) {
        const seg = place.segments.find((s) => typeof s !== "string");
        if (seg && typeof seg !== "string") focus = { ...sectionMetaOf(paneView, str(v[seg.var])) };
    }

    switch (link.view) {
        case "settings":
            if (p.board !== undefined) {
                focus[DEST_META.settingsBoard] = str(p.board);
                focus[DEST_META.settingsBoardAsk] = now;
            } else if (v.section === "kanban") {
                // As openSettingsAt: opened on the Kanban section with no board forgets the one asked for before.
                focus[DEST_META.settingsBoard] = null;
                focus[DEST_META.settingsBoardAsk] = null;
            }
            if (link.anchor) {
                revealSetting = link.anchor;
                focus[DEST_META.settingsReveal] = link.anchor;
                focus[DEST_META.settingsRevealAsk] = now;
            }
            break;
        case "vault":
            if (link.place === "request") focus[DEST_META.vaultRequest] = str(v.request);
            // Entries have no opaque id in this build (D7): the Vault opens and the caller is told the entry is not there.
            if (link.place === "entry") missing = "entry";
            break;
        case "memory":
            if (p.agent !== undefined) focus[DEST_META.memoryAgent] = str(p.agent);
            break;
        case "kanban":
            if (link.place === "task") focus[DEST_META.kanbanOpen] = str(v.task);
            if (link.place === "board") focus[DEST_META.kanbanBoard] = str(v.board);
            // The Kanban reads `<board>#<n>` as that board's task (kanban-section-state.ts).
            if (link.place === "board-task") focus[DEST_META.kanbanOpen] = `${str(v.board)}#${str(v.task)}`;
            break;
        case "swarm":
            if (link.place === "section") focus[`swarm:section`] = str(v.section);
            break;
        case "audit":
            if (link.place === "agent") focus[DEST_META.auditAgent] = str(v.agent);
            if (link.place === "sound") focus[DEST_META.auditSound] = str(v.sound);
            if (link.place === "message") focus[DEST_META.auditMessage] = str(v.message);
            break;
        case "editor":
        case "media":
            if (link.place === "file") {
                focus[DEST_META.editorFile] = str(v.path);
                if (p.line !== undefined) focus[DEST_META.editorLine] = Number(p.line);
                if (p.offset !== undefined) focus[DEST_META.editorOffset] = Number(p.offset);
                // The Media view is the Editor following newer files in the file's folder.
                if (link.view === "media") focus[DEST_META.editorFollowFolder] = true;
            }
            break;
        case "files":
            if (link.place === "folder") {
                focus[DEST_META.filesPath] = str(v.path);
                if (p.select !== undefined) focus[DEST_META.filesSelect] = [str(p.select)];
            }
            break;
    }

    // A moved or retired section follows pane-sections.ts, as a saved pane does.
    const meta = movedMeta({ view: paneView, ...focus });
    const finalView = typeof meta.view === "string" ? meta.view : paneView;
    const { view: _v, ...focusMeta } = meta;
    return { view: finalView, meta: { ...meta, view: finalView }, focusMeta, ...(revealSetting ? { revealSetting } : {}), ...(missing ? { missing } : {}) };
}

export function isPlanProblem(x: PanePlan | PlanProblem): x is PlanProblem {
    return (x as PlanProblem).code !== undefined;
}
