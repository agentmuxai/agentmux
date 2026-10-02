// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A Media pane: its files as document tabs. Only the tab in front is
 * mounted (a video in a tab behind it must not keep playing), so a tab is
 * just its file until it is shown. Files opened from elsewhere in the app
 * (an image in an agent's reply, Hangar) arrive through `media:open` and
 * become tabs here instead of new panes.
 * docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §6.3, §5.7.
 */

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { DocTabsController, handleDocTabKey, type DocTabsSpec } from "@/app/doc-tabs/doc-tabs-controller";
import { DocTabStrip } from "@/app/doc-tabs/DocTabStrip";
import { AUDIO_EXTENSIONS, basenameOf, extOf, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS } from "@/app/element/local-media";
import { getApi } from "@/app/store/app-api";
import { fireAndForget } from "@/util/util";
import { createEffect, on, Show, type JSX } from "solid-js";
import { MediaView } from "./media-view";

/** The file the pane shows (the tab in front's): the pane's title, layout
 *  export, and what an older build restores. */
export const META_PATH = "media:path" as const;
/** Files to open here as tabs, appended by `openMedia` (media-open.ts) and
 *  drained by the pane. */
export const META_OPEN = "media:open" as const;

/** A Media document: a file, or none yet ("Click to load media"). */
export interface MediaDoc {
    path: string;
    /** Tells apart tabs with no file yet (each is its own document). */
    blank?: number;
}

let blanks = 0;

export function mediaIcon(path: string): string {
    const ext = extOf(path);
    if (IMAGE_EXTENSIONS.includes(ext)) return "image";
    if (VIDEO_EXTENSIONS.includes(ext)) return "film";
    if (AUDIO_EXTENSIONS.includes(ext)) return "music";
    return "photo-film";
}

export const MEDIA_DOC_TABS: DocTabsSpec<MediaDoc> = {
    keyOf: (d) => d.path || `blank:${d.blank ?? 0}`,
    titleOf: (d) => (d.path ? basenameOf(d.path) : "Media"),
    iconOf: (d) => mediaIcon(d.path),
    serialize: (d) => d.path,
    deserialize: (st) => (typeof st === "string" && st ? { path: st } : null),
    // Ctrl+T, "+": a tab to pick a file into.
    newDocument: () => ({ path: "", blank: ++blanks }),
};

export class MediaPaneModel {
    readonly tabs: DocTabsController<MediaDoc>;

    constructor(private readonly ctx: PaneTabHostContext) {
        // A pane from before tabs, or opened on a file (OpenMedia, a click
        // on an image), starts with one tab on its `media:path`.
        const start = ctx.meta()?.[META_PATH];
        this.tabs = new DocTabsController(
            MEDIA_DOC_TABS,
            { meta: () => ctx.meta() as Record<string, unknown> | undefined, setMeta: (p) => ctx.setMeta(p) },
            [typeof start === "string" && start ? { path: start } : { path: "", blank: ++blanks }]
        );
    }

    /** The pane's title: the file in front. */
    title(): string {
        const t = this.tabs.active();
        return t ? MEDIA_DOC_TABS.titleOf(t.payload) : "Media";
    }

    /** A tab now shows `path`: its title follows, and the pane's
     *  `media:path` when it is the tab in front. */
    setTabPath(tabId: string, path: string): void {
        const tab = this.tabs.tabs().find((t) => t.id === tabId);
        if (!tab) return;
        const payload: MediaDoc = path ? { path } : { path: "", blank: ++blanks };
        this.tabs.update(tabId, {
            payload,
            key: MEDIA_DOC_TABS.keyOf(payload),
            title: MEDIA_DOC_TABS.titleOf(payload),
            icon: MEDIA_DOC_TABS.iconOf!(payload),
        });
        if (this.tabs.activeId() === tabId) fireAndForget(() => this.ctx.setMeta({ [META_PATH]: path }));
    }

    /** Open `path` as a tab here (one already open on it comes to the front). */
    open(path: string): void {
        // The tab in front with no file yet takes it.
        const active = this.tabs.active();
        if (active && !active.payload.path) {
            this.setTabPath(active.id, path);
            return;
        }
        this.tabs.open({ path });
    }

    dispose(): void {
        this.tabs.dispose();
    }
}

export function MediaPane(props: { pane: MediaPaneModel; ctx: PaneTabHostContext }): JSX.Element {
    const pane = props.pane;
    const tabs = pane.tabs;
    const ctx = props.ctx;

    // The tab in front names the pane's file.
    createEffect(
        on(
            tabs.activeId,
            () => {
                const path = tabs.active()?.payload.path ?? "";
                if ((ctx.meta()?.[META_PATH] ?? "") !== path) fireAndForget(() => ctx.setMeta({ [META_PATH]: path }));
            },
            { defer: true }
        )
    );

    // A Media pane always has a tab to pick or drop a file into: closing
    // the last one leaves an empty one ("Click to load media").
    createEffect(() => {
        if (tabs.tabs().length === 0) tabs.newDocument();
    });

    // Files sent here from elsewhere in the app, in order; only the entries
    // handled are removed (more may have been appended meanwhile).
    createEffect(() => {
        const queued = ctx.meta()?.[META_OPEN];
        if (!Array.isArray(queued) || queued.length === 0) return;
        const handled = queued.filter((p): p is string => typeof p === "string" && p.length > 0);
        for (const path of handled) pane.open(path);
        fireAndForget(() => {
            const now = ctx.meta()?.[META_OPEN];
            const rest = Array.isArray(now) ? now.filter((p) => !handled.includes(p as string)) : [];
            return ctx.setMeta({ [META_OPEN]: rest.length ? rest : null });
        });
    });

    const onKeyDown = (e: KeyboardEvent): void => {
        if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
        handleDocTabKey(e, tabs);
    };

    const pickInto = async (): Promise<void> => {
        const path = await getApi()?.showOpenFileDialog?.();
        if (path) pane.open(path);
    };

    return (
        <div class="media-pane flex flex-col w-full h-full" tabIndex={-1} onKeyDown={onKeyDown}>
            <DocTabStrip ctl={tabs} tooltipOf={(t) => t.payload.path || "No file yet"} addTitle="New tab (Ctrl+T)" />
            <div class="media-pane-doc flex-1" style={{ position: "relative", "min-height": 0 }}>
                {/* Keyed by tab: switching tabs mounts that tab's file. */}
                <Show
                    when={tabs.active()?.id}
                    keyed
                    fallback={
                        <div class="media-pane-empty" onClick={() => void pickInto()}>
                            <div>No files open</div>
                            <div class="media-pane-empty-hint">Click to pick one, or drop a file here.</div>
                        </div>
                    }
                >
                    {(id) => (
                        <MediaView
                            blockId={ctx.blockId}
                            path={tabs.tabs().find((t) => t.id === id)?.payload.path ?? ""}
                            onPathChange={(path) => pane.setTabPath(id, path)}
                            openDropped={(path) => {
                                // Onto a tab showing a file: a new tab. Onto
                                // an empty one: that tab.
                                if (!tabs.active()?.payload.path) return false;
                                tabs.open({ path });
                                return true;
                            }}
                        />
                    )}
                </Show>
            </div>
        </div>
    );
}
