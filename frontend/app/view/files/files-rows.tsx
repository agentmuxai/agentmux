// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's rows: a details row, a grid tile and the rename box that sits in either, with the
 * sizes the windowed list lays them out at. Split out of files-view.tsx.
 */

import { tip } from "@/app/element/tip";
import type { Touch } from "@/app/store/touched-files";
import type { FsEntry } from "@/types/rpc/FsEntry";
import type { FsGitState } from "@/types/rpc/FsGitState";
import { formatBytes } from "@/util/format-bytes";
import { createEffect, createSignal, on, onCleanup, onMount, Show, type JSX } from "solid-js";
import { type FilesModel, windowsNames } from "./files-model";
import { nameProblem, stemLength } from "./files-path";
import { extensionOf } from "./files-sort";
import { cachedThumbnail, hasThumbnail, thumbnail } from "./files-thumbs";
import { TypeIcon } from "./files-type-icon";
export const ROW_HEIGHT = 24;
/** A grid tile's box (thumbnail and a two-line name). */
export const TILE_W = 112;
export const TILE_H = 132;
export const OVERSCAN = 8;


/** "just now", "5 min ago", "3 h ago", "yesterday", else the date. */
export function formatModified(ms: number | undefined, now: number = Date.now()): string {
    if (ms == null) return "";
    const diff = now - ms;
    if (diff < 0) return new Date(ms).toLocaleDateString();
    const min = Math.floor(diff / 60_000);
    if (min < 1) return "just now";
    if (min < 60) return `${min} min ago`;
    const h = Math.floor(min / 60);
    if (h < 24) return `${h} h ago`;
    if (h < 48) return "yesterday";
    return new Date(ms).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

function kindOf(e: FsEntry): string {
    if (e.is_dir) return e.is_symlink ? "Folder link" : "Folder";
    const ext = extensionOf(e.name);
    const base = ext ? `${ext.toUpperCase()} file` : "File";
    return e.is_symlink ? `${base} link` : base;
}

/** The letter beside a name and what it means (spec §12, git decorations).
 *  A folder shows the most pressing state of what's inside it. */
const GIT_MARKS: Record<Exclude<FsGitState, "ignored">, { letter: string; title: string }> = {
    untracked: { letter: "U", title: "Untracked: new, not yet added to git" },
    added: { letter: "A", title: "Added to git, not yet committed" },
    renamed: { letter: "R", title: "Renamed" },
    deleted: { letter: "D", title: "Deleted" },
    modified: { letter: "M", title: "Changed since the last commit" },
    conflicted: { letter: "!", title: "Merge conflict" },
};

export function FileRow(props: {
    model: FilesModel;
    entry: FsEntry;
    index: number;
    selected: boolean;
    focused: boolean;
    renaming: boolean;
    onClick: (e: MouseEvent) => void;
    onOpen: () => void;
    onContextMenu: (e: MouseEvent) => void;
    onRenameDone: () => void;
    onDragStart: (e: DragEvent) => void;
    onDragEnd: () => void;
    git?: FsGitState;
    touch?: Touch;
    onDragOver: () => void;
    dropTarget: boolean;
}): JSX.Element {
    return (
        <div
            id={`files-${props.model.blockId}-row-${props.index}`}
            class="files-row"
            classList={{
                "files-row-selected": props.selected,
                "files-row-focused": props.focused,
                "files-row-hidden": props.entry.hidden,
                "files-row-ignored": props.git === "ignored",
                "files-row-droptarget": props.dropTarget,
            }}
            role="row"
            aria-rowindex={props.index + 2}
            aria-selected={props.selected}
            style={{ top: `${props.index * ROW_HEIGHT}px` }}
            onClick={props.onClick}
            onDblClick={props.onOpen}
            onContextMenu={props.onContextMenu}
            draggable={!props.renaming}
            onDragStart={props.onDragStart}
            onDragOver={props.onDragOver}
            onDragEnd={props.onDragEnd}
            {...tip(props.entry.error ?? (props.entry.link_target ? `→ ${props.entry.link_target}` : undefined))}
        >
            <div class="files-cell files-col-name" role="gridcell">
                <span class="files-icon-box"><TypeIcon model={props.model} entry={props.entry} cssPx={16} glyphClass="files-icon" /></span>
                <Show when={props.entry.is_symlink}>
                    <i class="fa fa-share files-link-mark" aria-label="link" />
                </Show>
                <Show when={props.touch}>
                    {(t) => (
                        <span
                            class="files-touch"
                            style={t().color ? { "background-color": t().color } : undefined}
                            {...tip(`${props.entry.is_dir ? "Something inside was changed" : "Changed"} by ${t().agentName}, ${formatModified(t().at)} (${t().tool})`)}
                            aria-label={`changed by ${t().agentName}`}
                        />
                    )}
                </Show>
                <Show when={props.renaming} fallback={<span class="files-name">{props.entry.name}</span>}>
                    <RenameInput model={props.model} entry={props.entry} onDone={props.onRenameDone} />
                </Show>
                <Show when={props.entry.error}>
                    <i class="fa fa-triangle-exclamation files-entry-error" aria-label={props.entry.error} />
                </Show>
                <Show when={props.git && props.git !== "ignored" && GIT_MARKS[props.git]}>
                    {(mark) => (
                        <span class={`files-git files-git-${props.git}`} {...tip(mark().title)} aria-label={mark().title}>
                            {mark().letter}
                        </span>
                    )}
                </Show>
            </div>
            <div class="files-cell files-col-modified" role="gridcell">
                {formatModified(props.entry.mtime)}
            </div>
            <div class="files-cell files-col-size" role="gridcell">
                {props.entry.is_dir || props.entry.size == null ? "" : formatBytes(props.entry.size)}
            </div>
            <div class="files-cell files-col-kind" role="gridcell">
                {kindOf(props.entry)}
            </div>
        </div>
    );
}

/** One tile of the grid view: a thumbnail for an image, else a big icon,
 *  and the name below it. Same handlers as a details row. */
export function GridTile(props: {
    model: FilesModel;
    entry: FsEntry;
    index: number;
    left: number;
    top: number;
    selected: boolean;
    focused: boolean;
    renaming: boolean;
    git?: FsGitState;
    touch?: Touch;
    dropTarget: boolean;
    onClick: (e: MouseEvent) => void;
    onOpen: () => void;
    onContextMenu: (e: MouseEvent) => void;
    onDragStart: (e: DragEvent) => void;
    onDragOver: () => void;
    onDragEnd: () => void;
    onRenameDone: () => void;
}): JSX.Element {
    const path = () => props.model.pathOf(props.entry.name);
    // Thumbnails read through this computer's media stream: none for a host's
    // files, which would show whatever is at that path here.
    const local = () => !props.model.connection() && !props.model.root();
    const [thumb, setThumb] = createSignal<string | undefined>(local() ? cachedThumbnail(path(), props.entry.mtime) : undefined);
    createEffect(
        on(
            () => [path(), props.entry.mtime, props.entry.size] as const,
            ([p, mtime, size]) => {
                if (!local() || !hasThumbnail(props.entry.name, size)) {
                    setThumb(undefined);
                    return;
                }
                const hit = cachedThumbnail(p, mtime);
                setThumb(hit);
                if (hit) return;
                // Scrolled past before it was made: stop asking for it.
                const gone = new AbortController();
                onCleanup(() => gone.abort());
                void thumbnail(p, mtime, gone.signal).then((url) => {
                    if (!gone.signal.aborted && url) setThumb(url);
                });
            }
        )
    );
    return (
        <div
            id={`files-${props.model.blockId}-row-${props.index}`}
            class="files-tile"
            classList={{
                "files-row-selected": props.selected,
                "files-row-focused": props.focused,
                "files-row-hidden": props.entry.hidden,
                "files-row-ignored": props.git === "ignored",
                "files-row-droptarget": props.dropTarget,
            }}
            role="gridcell"
            aria-selected={props.selected}
            style={{ left: `${props.left}px`, top: `${props.top}px` }}
            onClick={props.onClick}
            onDblClick={props.onOpen}
            onContextMenu={props.onContextMenu}
            draggable={!props.renaming}
            onDragStart={props.onDragStart}
            onDragOver={props.onDragOver}
            onDragEnd={props.onDragEnd}
            {...tip(props.entry.name)}
        >
            <div class="files-tile-art">
                <Show
                    when={thumb()}
                    fallback={<TypeIcon model={props.model} entry={props.entry} cssPx={48} glyphClass="files-tile-icon" />}
                >
                    {(src) => <img class="files-tile-thumb" src={src()} alt="" draggable={false} />}
                </Show>
                <Show when={props.touch}>
                    {(t) => (
                        <span
                            class="files-touch files-tile-touch"
                            style={t().color ? { "background-color": t().color } : undefined}
                            {...tip(`Changed by ${t().agentName}, ${formatModified(t().at)} (${t().tool})`, { label: true })}
                        />
                    )}
                </Show>
                <Show when={props.git && props.git !== "ignored" && GIT_MARKS[props.git]}>
                    {(mark) => <span class={`files-git files-tile-git files-git-${props.git}`} {...tip(mark().title)}>{mark().letter}</span>}
                </Show>
            </div>
            <Show when={props.renaming} fallback={<div class="files-tile-name">{props.entry.name}</div>}>
                <RenameInput model={props.model} entry={props.entry} onDone={props.onRenameDone} />
            </Show>
        </div>
    );
}

function RenameInput(props: { model: FilesModel; entry: FsEntry; onDone: () => void }): JSX.Element {
    let input: HTMLInputElement | undefined;
    const [problem, setProblem] = createSignal<string | null>(null);
    let busy = false;
    // Enter or Escape unmounts the input, and its removal can fire blur,
    // which must not commit a second time (ReAgent on #4201).
    let finished = false;
    // Scrolled out of the window, the row (and this box) unmounts: leave
    // rename mode with it, or the list's keyboard would wait on a box that
    // isn't there.
    onCleanup(() => {
        if (!finished && props.model.renaming() === props.entry.name) props.model.setRenaming(null);
    });
    onMount(() => {
        input?.focus();
        input?.setSelectionRange(0, stemLength(props.entry.name, props.entry.is_dir));
    });
    /** Enter commits and stays in the box on a problem, so it can be fixed.
     *  Leaving the box (blur) commits too, but on a problem it cancels: with
     *  the focus gone, the list's keys would otherwise wait on a box nobody
     *  is typing in (ReAgent on #4201). */
    const finish = async (commit: boolean, leaving = false): Promise<void> => {
        if (busy || finished) return;
        const value = input?.value ?? props.entry.name;
        if (commit && value !== props.entry.name) {
            const why = nameProblem(value, windowsNames());
            if (why && !leaving) {
                setProblem(why);
                return;
            }
            if (!why) {
                busy = true;
                const ok = await props.model.rename(props.entry.name, value);
                busy = false;
                if (!ok && !leaving) return;
            }
        }
        finished = true;
        props.model.setRenaming(null);
        props.onDone();
    };
    return (
        <span class="files-rename">
            <input
                ref={input}
                class="files-rename-input" data-raw-text-field="the inline rename in a file row: it sits in the row's own line and commits or cancels on Enter, Escape or blur, not a form field"
                value={props.entry.name}
                spellcheck={false}
                aria-label={`New name for ${props.entry.name}`}
                aria-invalid={problem() != null}
                onClick={(e) => e.stopPropagation()}
                onDblClick={(e) => e.stopPropagation()}
                onInput={() => setProblem(null)}
                onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === "Enter") void finish(true);
                    else if (e.key === "Escape") void finish(false);
                }}
                onBlur={() => {
                    // The window losing focus (Alt+Tab) blurs the box but
                    // leaves it the active element: the user hasn't left the
                    // rename, so keep it open for when they come back.
                    if (document.activeElement === input) return;
                    void finish(true, true);
                }}
            />
            <Show when={problem()}>
                <span class="files-rename-problem" role="alert">
                    {problem()}
                </span>
            </Show>
        </span>
    );
}
