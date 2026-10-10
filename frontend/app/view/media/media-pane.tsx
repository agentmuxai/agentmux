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
import { hostHas } from "@/app/host/host-caps";
import { getApi } from "@/app/store/app-api";
import { fireAndForget } from "@/util/util";
import { createEffect, on, Show, untrack, type JSX } from "solid-js";
import { MediaView } from "./media-view";
import { keyLabel } from "@/app/keybindings";

/** The file the pane shows (the tab in front's): the pane's title, layout
 *  export, and what an older build restores. */
export const META_PATH = "media:path" as const;
/** Files to open here as tabs, appended by `openInMediaPaneOnScreen`
 *  (media-open.ts) and drained by the pane. */
export const META_OPEN = "media:open" as const;

/** One request in `media:open`. Its `id` tells two requests for the same
 *  file apart (two clicks on one image), so draining removes exactly the
 *  ones handled. */
export interface MediaOpenRequest {
    id: string;
    path: string;
}

function asRequest(v: unknown): MediaOpenRequest | null {
    const r = v as Partial<MediaOpenRequest> | null;
    return r && typeof r.id === "string" && typeof r.path === "string" && r.path ? { id: r.id, path: r.path } : null;
}

/** A Media document: a file, or none yet ("Click to load media"). */
export interface MediaDoc {
    path: string;
    /** Tells apart tabs with no file on disk (each is its own document). */
    blank?: number;
    /** Dropped bytes with no host path: shown, kept while the pane lives,
     *  not saved (a restart drops the tab). */
    file?: File;
}

/** A tab showing nothing yet. */
const isEmpty = (d: MediaDoc | undefined): boolean => !!d && !d.path && !d.file;

let blanks = 0;

export function mediaIcon(path: string): string {
    const ext = extOf(path);
    if (IMAGE_EXTENSIONS.includes(ext)) return "image";
    if (VIDEO_EXTENSIONS.includes(ext)) return "film";
    if (AUDIO_EXTENSIONS.includes(ext)) return "music";
    return "photo-film";
}

/** A tab with no file yet is "New Tab", as in a browser: a place to load a
 *  file, not a document to save. It has no icon; a file's tab has its kind's. */
export const MEDIA_DOC_TABS: DocTabsSpec<MediaDoc> = {
    keyOf: (d) => d.path || `blank:${d.blank ?? 0}`,
    titleOf: (d) => (d.path ? basenameOf(d.path) : (d.file?.name ?? "New Tab")),
    iconOf: (d) => (isEmpty(d) ? undefined : mediaIcon(d.path || d.file?.name || "")),
    serialize: (d) => d.path,
    deserialize: (st) => (typeof st === "string" && st ? { path: st } : null),
    // Ctrl+T, "+": a tab to pick a file into.
    newDocument: () => ({ path: "", blank: ++blanks }),
    // The strip is there with one file too, as the Editor's is: its "+" and
    // a tab to drag to another Media pane are where people look for them.
    alwaysShowStrip: true,
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

    /** The pane's title: the file in front, or "Media" while it shows none. */
    title(): string {
        const t = this.tabs.active();
        return t && !isEmpty(t.payload) ? MEDIA_DOC_TABS.titleOf(t.payload) : "Media";
    }

    /** A tab now shows `path`: its title follows, and the pane's
     *  `media:path` when it is the tab in front. One file has one tab: if
     *  another tab already shows `path`, an empty tab gives way to it (that
     *  tab comes to the front; false: this tab is gone), and a tab that
     *  showed a file keeps it, closing the other (a newer render landing on
     *  a file another tab had open). */
    setTabPath(tabId: string, path: string): boolean {
        const tab = this.tabs.tabs().find((t) => t.id === tabId);
        if (!tab) return false;
        const other = path ? this.tabs.tabs().find((t) => t.id !== tabId && t.payload.path === path) : undefined;
        if (other && isEmpty(tab.payload)) {
            this.tabs.activate(other.id);
            this.tabs.close(tabId);
            return false;
        }
        if (other) this.tabs.close(other.id);
        const payload: MediaDoc = path ? { path } : { path: "", blank: ++blanks };
        this.tabs.update(tabId, {
            payload,
            key: MEDIA_DOC_TABS.keyOf(payload),
            title: MEDIA_DOC_TABS.titleOf(payload),
            icon: MEDIA_DOC_TABS.iconOf!(payload),
        });
        if (this.tabs.activeId() === tabId) fireAndForget(() => this.ctx.setMeta({ [META_PATH]: path }));
        return true;
    }

    /** Open `path` as a tab here (one already open on it comes to the front). */
    open(path: string): void {
        // The tab in front with no file yet takes it, unless the file is
        // open already (then that tab comes to the front).
        const active = this.tabs.active();
        if (active && isEmpty(active.payload) && !this.tabs.tabs().some((t) => t.payload.path === path)) {
            this.setTabPath(active.id, path);
            return;
        }
        this.tabs.open({ path });
    }

    /** Dropped bytes with no host path: on the tab in front when it shows
     *  nothing, else on a new tab (a drop never replaces what a tab shows). */
    openFile(file: File): void {
        const active = this.tabs.active();
        if (active && isEmpty(active.payload)) {
            const payload: MediaDoc = { path: "", blank: active.payload.blank, file };
            this.tabs.update(active.id, { payload, title: MEDIA_DOC_TABS.titleOf(payload), icon: MEDIA_DOC_TABS.iconOf!(payload) });
            fireAndForget(() => this.ctx.setMeta({ [META_PATH]: "" }));
            return;
        }
        this.tabs.open({ path: "", blank: ++blanks, file });
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
                if ((untrack(() => ctx.meta()?.[META_PATH]) ?? "") !== path) fireAndForget(() => ctx.setMeta({ [META_PATH]: path }));
            },
            { defer: true }
        )
    );

    // A Media pane always has a tab to pick or drop a file into: closing
    // the last one leaves an empty one ("Click to load media").
    createEffect(() => {
        if (tabs.tabs().length === 0) tabs.newDocument();
    });

    // Files sent here from elsewhere in the app, in order. Each request is
    // handled once, by id: the queue is seen again whenever the block's
    // meta changes, and its clearing takes a round trip to land. Draining
    // removes exactly the requests handled; ones appended meanwhile stay.
    // Tracks the queue alone: opening reads the tabs.
    const handledIds = new Set<string>();
    createEffect(
        on(
            () => ctx.meta()?.[META_OPEN],
            (queued) => {
                const requests = (Array.isArray(queued) ? queued : []).map(asRequest).filter((r): r is MediaOpenRequest => r != null);
                // Forget ids the block no longer holds: their clearing landed.
                const present = new Set(requests.map((r) => r.id));
                for (const id of handledIds) if (!present.has(id)) handledIds.delete(id);
                const fresh = requests.filter((r) => !handledIds.has(r.id));
                if (fresh.length === 0) return;
                for (const r of fresh) handledIds.add(r.id);
                untrack(() => {
                    for (const r of fresh) pane.open(r.path);
                });
                fireAndForget(() => {
                    const now = untrack(() => ctx.meta()?.[META_OPEN]);
                    const rest = (Array.isArray(now) ? now : []).filter((v) => {
                        const r = asRequest(v);
                        return r != null && !handledIds.has(r.id);
                    });
                    return ctx.setMeta({ [META_OPEN]: rest.length ? rest : null });
                });
            }
        )
    );

    const onKeyDown = (e: KeyboardEvent): void => {
        if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
        handleDocTabKey(e, tabs);
    };

    const pickInto = async (): Promise<void> => {
        if (!hostHas("nativeDialogs")) return;
        const path = await getApi()?.showOpenFileDialog?.();
        if (path) pane.open(path);
    };

    return (
        <div class="media-pane flex flex-col w-full h-full" tabIndex={-1} onKeyDown={onKeyDown}>
            <DocTabStrip
                ctl={tabs}
                docType="media"
                blockId={ctx.blockId}
                canDrag={(t) => !isEmpty(t.payload)}
                tooltipOf={(t) => t.payload.path || "No file yet"}
                addTitle={`New tab (${keyLabel("ctrl+t")})`}
            />
            <div class="media-pane-doc flex-1" style={{ position: "relative", "min-height": 0 }}>
                {/* Keyed by tab: switching tabs mounts that tab's file. */}
                <Show
                    when={tabs.active()?.id}
                    keyed
                    fallback={
                        <div class="media-pane-empty" onClick={() => void pickInto()}>
                            <div>No files open</div>
                            <div class="media-pane-empty-hint">
                                {hostHas("nativeDialogs") ? "Click to pick one, or drop a file here." : "Drop a file here."}
                            </div>
                        </div>
                    }
                >
                    {(id) => (
                        <MediaView
                            blockId={ctx.blockId}
                            path={tabs.tabs().find((t) => t.id === id)?.payload.path ?? ""}
                            file={tabs.tabs().find((t) => t.id === id)?.payload.file}
                            openDroppedFile={(file) => {
                                pane.openFile(file);
                                return true;
                            }}
                            onPathChange={(path) => pane.setTabPath(id, path)}
                            openDropped={(path) => {
                                // Onto a tab showing a file: a new tab. Onto
                                // an empty one: that tab.
                                if (isEmpty(tabs.active()?.payload)) return false;
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
