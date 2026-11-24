// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a tab does with text it read that CodeMirror can't take: a refusal in the
 * tab, back to the file's own viewer after "Open as text anyway", or, for a first
 * line too wide, the Large text viewer. Kept out of editor-model.ts, which holds
 * the tabs. docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.2.
 */

import { dispatch } from "@/app/store/editor-pane-state-store";
import { sniffRefusal } from "./editor-sniff";
import { chooseViewer } from "./viewers/viewer-choice";
import type { ViewerTabs } from "./viewers/viewer-tabs";

/** What the model lends: its block, its viewer state, its error report, and marking a tab's non-text viewer loaded. */
export interface RefusalHost {
    blockId: string;
    viewerTabs: ViewerTabs;
    report: (action: string, e: Error) => void;
    markViewerLoaded: (tabId: string, hash: string, opts?: { skipIfHashMatches?: string }) => string;
}

/**
 * Refuse content that looks binary or that would freeze the renderer in CodeMirror:
 *   1. Any NUL byte → binary (text files don't contain U+0000).
 *   2. Single line over 100K chars → would block the layout thread laying out a half-megabyte-wide line (real-world
 *      hit: Windows NTUSER.DAT regtrans-ms files come through as 512KB of one line).
 * Both are fast checks; the caller bails before stashing content or hashing.
 *
 * Returns undefined when the content is fine (the load goes on); else the load's result: null for a refusal, or the
 * hash of a tab moved to another viewer.
 */
export function refuseContent(
    host: RefusalHost,
    tabId: string,
    path: string,
    content: string,
    allowBinaryText: boolean,
    opts?: { skipIfHashMatches?: string }
): string | null | undefined {
    const sniffed = sniffRefusal(content, allowBinaryText);
    if (!sniffed) return undefined;
    const probe = host.viewerTabs.probe(tabId);
    // "Open as text anyway" that CodeMirror can't take after all: back to the file's own viewer, the choice dropped,
    // and say why.
    if (allowBinaryText && probe) {
        dispatch(host.blockId, { type: "SetTabViewer", tabId, viewer: chooseViewer(probe), clearOverride: true, source: "system" });
        host.report("Opening as text", new Error(sniffed.message));
        host.markViewerLoaded(tabId, `probe:${probe.size}:${probe.mtime}`);
        return null;
    }
    // A first line too wide for CodeMirror in a file the Large text viewer can show: it opens there.
    const wide = sniffed.kind === "wide" ? host.viewerTabs.markWide(tabId, path) : null;
    if (wide) {
        dispatch(host.blockId, { type: "SetTabViewer", tabId, viewer: "large", clearOverride: true, source: "system" });
        return host.markViewerLoaded(tabId, `probe:${wide.size}:${wide.mtime}`, opts);
    }
    dispatch(host.blockId, { type: "TabContentLoadFailed", tabId, error: sniffed.message, source: "system" });
    return null;
}
