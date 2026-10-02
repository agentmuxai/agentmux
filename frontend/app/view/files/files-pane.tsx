// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A Hangar pane: its folders as document tabs, each a whole browser of its
 * own (folder, history, selection), with one strip, one set of tab keys and
 * one queue of copy and move jobs for the pane.
 * docs/specs/SPEC_DOCUMENT_TABS_2026_10_02.md §6.2.
 */

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";
import { DocTabsController, handleDocTabKey, type DocTabsSpec } from "@/app/doc-tabs/doc-tabs-controller";
import { DocTabStrip } from "@/app/doc-tabs/DocTabStrip";
import { createEffect, createRoot, For, on, Show, untrack, type JSX } from "solid-js";
import { FilesModel, META_PATH } from "./files-model";
import { FilesOps, opFinishedText } from "./files-ops";
import { baseName, samePath } from "./files-path";
import { FilesView } from "./files-view";

/** A Hangar document: a folder. */
export interface FilesDoc {
    path: string;
}

export const FILES_DOC_TABS: DocTabsSpec<FilesDoc> = {
    keyOf: (d) => d.path,
    titleOf: (d) => (d.path ? baseName(d.path) || d.path : "Hangar"),
    iconOf: () => "folder",
    serialize: (d) => d.path,
    deserialize: (st) => (typeof st === "string" ? { path: st } : null),
    // Ctrl+T: the folder in front, in a new tab.
    newDocument: (active) => ({ path: active?.path ?? "" }),
    keepOne: true,
};

export class FilesPaneModel {
    readonly tabs: DocTabsController<FilesDoc>;
    readonly ops: FilesOps;
    private readonly models = new Map<string, { model: FilesModel; dispose: () => void }>();

    constructor(private readonly ctx: PaneTabHostContext) {
        this.ops = new FilesOps(ctx.blockId);
        // One report for the pane, in the tab in front; every tab re-lists
        // (a move can take files out of a folder another tab shows).
        this.ops.onFinished = (op) => {
            this.activeModel()?.setStatus(opFinishedText(op), op.state === "done" && !op.failures?.length ? 4000 : 10000);
            for (const m of this.models.values()) m.model.refresh();
        };
        // A pane from before document tabs, or opened on a path (OpenFiles,
        // the widget bar), starts with one tab on its `files:path`.
        const start = ctx.meta()?.[META_PATH];
        this.tabs = new DocTabsController(
            FILES_DOC_TABS,
            { meta: () => ctx.meta() as Record<string, unknown> | undefined, setMeta: (p) => ctx.setMeta(p) },
            [{ path: typeof start === "string" ? start : "" }]
        );
    }

    /** The model for a tab, made the first time it is shown. */
    modelFor(tabId: string): FilesModel | undefined {
        const have = this.models.get(tabId);
        if (have) return have.model;
        const tab = untrack(() => this.tabs.tabs().find((t) => t.id === tabId));
        if (!tab) return undefined;
        // Its own root: the model's memos must outlive whatever computation
        // first asked for it (a tab's title changing re-runs that).
        const made = createRoot((dispose) => ({
            model: new FilesModel(this.ctx, {
                initialPath: tab.payload.path || undefined,
                ops: this.ops,
                domKey: tabId,
                isActive: () => this.tabs.activeId() === tabId,
                onPathChange: (path) => this.onTabPath(tabId, path),
            }),
            dispose,
        }));
        this.models.set(tabId, made);
        return made.model;
    }

    activeModel(): FilesModel | undefined {
        const id = this.tabs.activeId();
        return id ? this.models.get(id)?.model : undefined;
    }

    /** A tab went to another folder: its title and record follow, and the
     *  block's `files:path` keeps naming the folder in front (layout export,
     *  the pane's "+" and OpenFiles read it). */
    private onTabPath(tabId: string, path: string): void {
        const tab = this.tabs.tabs().find((t) => t.id === tabId);
        if (!tab || samePath(tab.payload.path, path)) {
            if (tab && tab.payload.path !== path) this.tabs.update(tabId, { payload: { path } });
        } else {
            this.tabs.update(tabId, { payload: { path }, key: path, title: FILES_DOC_TABS.titleOf({ path }) });
        }
        if (this.tabs.activeId() === tabId) void this.ctx.setMeta({ [META_PATH]: path });
    }

    /** Release the models of tabs that are gone. */
    prune(): void {
        const live = new Set(this.tabs.tabs().map((t) => t.id));
        for (const [id, m] of this.models) {
            if (!live.has(id)) {
                m.model.dispose();
                m.dispose();
                this.models.delete(id);
            }
        }
    }

    /** The pane's title: the folder in front. */
    title(): string {
        const t = this.tabs.active();
        return t ? FILES_DOC_TABS.titleOf(t.payload) : "Hangar";
    }

    dispose(): void {
        this.tabs.dispose();
        for (const m of this.models.values()) {
            m.model.dispose();
            m.dispose();
        }
        this.models.clear();
        this.ops.dispose();
    }
}

export function FilesPane(props: { pane: FilesPaneModel; ctx: PaneTabHostContext }): JSX.Element {
    const pane = props.pane;
    const tabs = pane.tabs;

    // Closed tabs drop their model; the tab in front names the pane's folder.
    createEffect(on(() => tabs.tabs().map((t) => t.id).join("\u0000"), () => pane.prune()));
    createEffect(
        on(tabs.activeId, () => {
            const t = tabs.active();
            if (t?.payload.path) void props.ctx.setMeta({ [META_PATH]: t.payload.path });
            // Keyboard focus follows the tab switched to.
            queueMicrotask(() => pane.activeModel()?.focusList?.());
        })
    );

    const onKeyDown = (e: KeyboardEvent): void => {
        // Inside a text box (rename, path, filter) the keys are the box's.
        if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
        handleDocTabKey(e, tabs, () =>
            pane.activeModel()?.setStatus({ text: "A Hangar pane always shows a folder. Use the pane's × to close it.", tone: "info" }, 3000)
        );
    };

    return (
        <div class="files-pane" onKeyDown={onKeyDown}>
            <DocTabStrip ctl={tabs} tooltipOf={(t) => t.payload.path} addTitle="New tab (Ctrl+T)" />
            <div class="files-pane-docs">
                {/* Keyed by tab id: a tab's title or folder changing never
                    remounts it. Every tab stays mounted (its scroll, filter
                    and rename state survive a switch); only the one in front
                    is shown. */}
                <For each={tabs.tabs().map((t) => t.id)}>
                    {(id) => (
                        <Show when={pane.modelFor(id)}>
                            {(model) => (
                                <div class="files-pane-doc" classList={{ "files-pane-doc-hidden": tabs.activeId() !== id }}>
                                    <FilesView
                                        model={model()}
                                        ctx={props.ctx}
                                        active={() => tabs.activeId() === id}
                                        openInNewTab={(dir) => void tabs.open({ path: dir })}
                                    />
                                </div>
                            )}
                        </Show>
                    )}
                </For>
            </div>
        </div>
    );
}
