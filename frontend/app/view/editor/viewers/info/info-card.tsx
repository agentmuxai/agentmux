// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Editor's fallback for a file no viewer renders (a PDF until the PDF
 * viewer lands, an archive, a program, a database): what the file is, and
 * what can be done with it. Never a dead end.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.4.7.
 */

import { revealInFileManagerLabel } from "@/app/util/file-manager-label";
import { formatBytes } from "@/util/format-bytes";
import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { ConfirmModal } from "@/app/element/confirm-modal";
import { createSignal, Show, type JSX } from "solid-js";
import { ViewerBar, type ViewerBarProps } from "../viewer-bar";
import { isProgramPath, type FileProbe } from "../viewer-choice";

const KIND_KEYS = new Set([
    "image",
    "imageFile",
    "svg",
    "pdf",
    "audio",
    "video",
    "archive",
    "word",
    "excel",
    "powerpoint",
    "sqlite",
    "font",
    "text",
    "other",
]);

/** What the info card says a probe's kind is; a kind it has no words for reads as "File". */
function kindLabel(kind: string): string {
    const key = kind === "image_file" ? "imageFile" : kind;
    return KIND_KEYS.has(key) ? t(`infoCard.kind.${key}`) : t("infoCard.file");
}

export interface InfoCardProps {
    path: string;
    /** Dropped bytes with no host path. */
    file?: File;
    probe: FileProbe | null;
    bar: Omit<ViewerBarProps, "start" | "facts">;
    /** Absent when the file can't be shown as text (too large). */
    onOpenAsText?: () => void;
    /** Show the file's bytes: any file, whatever its size. */
    onOpenAsHex?: () => void;
    onOpenWithSystem?: () => void;
    onReveal?: () => void;
    onCopyPath?: () => void;
    /** Why the file isn't shown as text, when that's the reason. */
    note?: string;
}

export function InfoCard(props: InfoCardProps): JSX.Element {
    const name = () => props.file?.name || props.path.split(/[\\/]/).pop() || props.path;
    const kind = () => (props.probe ? kindLabel(props.probe.kind) : props.file?.type || t("infoCard.file"));
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
            <div class="info-card" role="region" aria-label={t("infoCard.aria")}>
                <div class="info-card-name">{name()}</div>
                <dl class="info-card-facts">
                    <dt>{t("infoCard.kindLabel")}</dt>
                    <dd>{kind()}</dd>
                    <Show when={size() != null}>
                        <dt>{t("infoCard.size")}</dt>
                        <dd>
                            {t("infoCard.sizeValue", { size: formatBytes(size()!), bytes: size()!.toLocaleString() })}
                        </dd>
                    </Show>
                    <Show when={props.probe?.mime}>
                        <dt>{t("infoCard.type")}</dt>
                        <dd>{props.probe!.mime}</dd>
                    </Show>
                    <Show when={modified()}>
                        <dt>{t("infoCard.modified")}</dt>
                        <dd>{modified()}</dd>
                    </Show>
                    <Show when={props.path && !props.file}>
                        <dt>{t("infoCard.path")}</dt>
                        <dd class="info-card-path">{props.path}</dd>
                    </Show>
                </dl>
                <Show when={props.note}>
                    <div class="info-card-note">{props.note}</div>
                </Show>
                <div class="info-card-actions">
                    <Show when={props.onOpenWithSystem}>
                        <Button tone="accent" icon="arrow-up-right-from-square" onClick={() => askThenOpen(props.onOpenWithSystem)}>
                            {t("infoCard.openSystem")}
                        </Button>
                    </Show>
                    <Show when={props.onOpenAsText}>
                        <Button icon="file-lines" onClick={() => props.onOpenAsText!()}>
                            {t("infoCard.openText")}
                        </Button>
                    </Show>
                    <Show when={props.onOpenAsHex}>
                        <Button icon="table-cells" onClick={() => props.onOpenAsHex!()}>
                            {t("infoCard.openHex")}
                        </Button>
                    </Show>
                    <Show when={props.onReveal}>
                        <Button icon="folder-open" onClick={() => props.onReveal!()}>
                            {revealInFileManagerLabel()}
                        </Button>
                    </Show>
                    <Show when={props.onCopyPath}>
                        <Button icon="copy" onClick={() => props.onCopyPath!()}>
                            {t("infoCard.copyPath")}
                        </Button>
                    </Show>
                </div>
            </div>
            <Show when={confirmRun()}>
                <ConfirmModal
                    open={true}
                    scope="pane"
                    title={t("infoCard.runTitle")}
                    description={t("infoCard.runBody", { name: name() })}
                    destructive
                    confirmLabel={t("infoCard.run")}
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
