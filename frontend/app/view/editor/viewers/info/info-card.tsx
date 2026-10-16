// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's fallback for a file no viewer renders (a PDF until the PDF
 * viewer lands, an archive, a program, a database): what the file is, and
 * what can be done with it. Never a dead end.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.7.
 */

import { formatBytes } from "@/app/element/local-media";
import { Button } from "@/app/element/ui";
import { ConfirmDialog } from "@/app/components/confirm-dialog";
import { createSignal, Show, type JSX } from "solid-js";
import { ViewerBar, type ViewerBarProps } from "../viewer-bar";
import { isProgramPath, TEXT_MAX_BYTES, type FileProbe } from "../viewer-choice";

/** What the info card says a probe's kind is. */
export const KIND_LABELS: Record<string, string> = {
    image: "Image",
    image_file: "Image the editor can't show yet",
    svg: "SVG image",
    pdf: "PDF document",
    audio: "Audio",
    video: "Video",
    archive: "Archive",
    word: "Word document",
    excel: "Spreadsheet",
    powerpoint: "Presentation",
    sqlite: "SQLite database",
    font: "Font",
    text: "Text",
    other: "Binary file",
};

export interface InfoCardProps {
    path: string;
    /** Dropped bytes with no host path. */
    file?: File;
    probe: FileProbe | null;
    bar: Omit<ViewerBarProps, "start" | "facts">;
    /** Absent when the file can't be shown as text (too large). */
    onOpenAsText?: () => void;
    onOpenWithSystem?: () => void;
    onReveal?: () => void;
    onCopyPath?: () => void;
    /** Why the file isn't shown as text, when that's the reason. */
    note?: string;
}

export function InfoCard(props: InfoCardProps): JSX.Element {
    const name = () => props.file?.name || props.path.split(/[\\/]/).pop() || props.path;
    const kind = () => (props.probe ? (KIND_LABELS[props.probe.kind] ?? "File") : props.file?.type || "File");
    const size = () => props.probe?.size ?? props.file?.size;
    const modified = () => (props.probe?.mtime ? new Date(props.probe.mtime).toLocaleString() : "");
    // The open waiting on "Run this file?", for a program.
    const [confirmRun, setConfirmRun] = createSignal<(() => void) | null>(null);
    const askThenOpen = (open: (() => void) | undefined) => {
        if (!open) return;
        if (isProgramPath(props.path)) setConfirmRun(() => open);
        else open();
    };
    const barOpen = () => props.bar.onOpenWithSystem;

    return (
        <div class="editor-viewer info-card-viewer">
            {/* The toolbar's open button asks too: it runs a program as surely as this one. */}
            <ViewerBar {...props.bar} onOpenWithSystem={barOpen() && (() => askThenOpen(barOpen()))} />
            <div class="info-card" role="region" aria-label="File info">
                <div class="info-card-name">{name()}</div>
                <dl class="info-card-facts">
                    <dt>Kind</dt>
                    <dd>{kind()}</dd>
                    <Show when={size() != null}>
                        <dt>Size</dt>
                        <dd>
                            {formatBytes(size()!)} ({size()!.toLocaleString()} bytes)
                        </dd>
                    </Show>
                    <Show when={props.probe?.mime}>
                        <dt>Type</dt>
                        <dd>{props.probe!.mime}</dd>
                    </Show>
                    <Show when={modified()}>
                        <dt>Modified</dt>
                        <dd>{modified()}</dd>
                    </Show>
                    <Show when={props.path && !props.file}>
                        <dt>Path</dt>
                        <dd class="info-card-path">{props.path}</dd>
                    </Show>
                </dl>
                <Show when={props.note}>
                    <div class="info-card-note">{props.note}</div>
                </Show>
                <div class="info-card-actions">
                    <Show when={props.onOpenWithSystem}>
                        <Button tone="accent" icon="arrow-up-right-from-square" onClick={() => askThenOpen(props.onOpenWithSystem)}>
                            Open with the system's app
                        </Button>
                    </Show>
                    <Show when={props.onOpenAsText && (size() ?? 0) <= TEXT_MAX_BYTES}>
                        <Button icon="file-lines" onClick={() => props.onOpenAsText!()}>
                            Open as text anyway
                        </Button>
                    </Show>
                    <Show when={props.onReveal}>
                        <Button icon="folder-open" onClick={() => props.onReveal!()}>
                            Reveal in folder
                        </Button>
                    </Show>
                    <Show when={props.onCopyPath}>
                        <Button icon="copy" onClick={() => props.onCopyPath!()}>
                            Copy path
                        </Button>
                    </Show>
                </div>
            </div>
            <Show when={confirmRun()}>
                <ConfirmDialog
                    title="Run this file?"
                    message={`${name()} is a program or script. Opening it with the system runs it.`}
                    confirmLabel="Run"
                    onConfirm={() => {
                        const open = confirmRun();
                        setConfirmRun(null);
                        open?.();
                    }}
                    onCancel={() => setConfirmRun(null)}
                />
            </Show>
        </div>
    );
}
