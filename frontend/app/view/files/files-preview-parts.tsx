// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Files pane's preview panel's markup in parts (files-preview.tsx): the empty and loading lines, and one view
 * for each kind of preview (state in files-preview-state.ts): a folder or media card, an image, text (code or
 * Markdown) and the card for what has no preview.
 */

import { Markdown } from "@/app/element/markdown";
import { formatBytes } from "@/util/format-bytes";
import type { FsEntry } from "@/types/rpc/FsEntry";
import { Match, Show, Switch, type Accessor, type JSX } from "solid-js";
import { detectLanguage } from "../agent/components/detectLanguage";
import { HighlightedCode } from "../agent/components/HighlightedCode";
import { Button } from "@/app/element/ui";
import { PREVIEW_TEXT_BYTES, previewFacts, type Preview } from "./files-preview-state";

type Kind<K extends Preview["kind"]> = Accessor<Extract<Preview, { kind: K }>>;
type Open = { onOpen: (entry: FsEntry) => void };

/** A folder: its facts and a button that opens it. */
function FolderCard(props: { p: Kind<"folder"> } & Open): JSX.Element {
    const p = props.p;
    return (
        <div class="files-preview-card">
            <i class="fa fa-folder files-preview-icon" aria-hidden="true" />
            <div class="files-preview-name">{p().entry.name}</div>
            <div class="files-preview-facts">{previewFacts(p().entry)}</div>
            <Button class="files-button" onClick={() => props.onOpen(p().entry)}>
                Open folder
            </Button>
        </div>
    );
}

/** An image, opened by a double click. */
function ImageMedia(props: { p: Kind<"image"> } & Open): JSX.Element {
    const p = props.p;
    return (
        <div class="files-preview-media">
            <img src={p().url} alt={p().entry.name} onDblClick={() => props.onOpen(p().entry)} />
            <div class="files-preview-facts">{previewFacts(p().entry)}</div>
        </div>
    );
}

/** Text: highlighted code or rendered Markdown, and, when cut short, a line that offers the whole file. */
function TextPreview(props: { p: Kind<"text"> } & Open): JSX.Element {
    const p = props.p;
    return (
        <div class="files-preview-text">
            <Show
                when={p().markdown}
                fallback={
                    <HighlightedCode
                        code={p().text}
                        lang={detectLanguage(p().entry.name, p().text.split("\n", 1)[0])}
                        class="files-preview-code"
                    />
                }
            >
                <div class="files-preview-md">
                    <Markdown text={p().text} scrollable={false} />
                </div>
            </Show>
            <Show when={p().truncated}>
                <div class="files-preview-more">
                    Showing the first {formatBytes(PREVIEW_TEXT_BYTES)}.{" "}
                    <Button variant="link" tone="accent" onClick={() => props.onOpen(p().entry)}>
                        Open the whole file
                    </Button>
                </div>
            </Show>
        </div>
    );
}

/** Video or audio: played in a Media pane, not here. */
function AvCard(props: { p: Kind<"av"> } & Open): JSX.Element {
    const p = props.p;
    return (
        <div class="files-preview-card">
            <i class="fa fa-film files-preview-icon" aria-hidden="true" />
            <div class="files-preview-name">{p().entry.name}</div>
            <div class="files-preview-facts">{previewFacts(p().entry)}</div>
            <Button class="files-button" onClick={() => props.onOpen(p().entry)}>
                Play in a Media pane
            </Button>
        </div>
    );
}

/** A file with no preview (not text, or a read that failed), and a button that opens it with the default app. */
function NoPreviewCard(props: { p: Kind<"binary" | "other"> } & { onOpenWithOs: (entry: FsEntry) => void }): JSX.Element {
    const p = props.p;
    return (
        <div class="files-preview-card">
            <i class="fa fa-file files-preview-icon" aria-hidden="true" />
            <div class="files-preview-name">{p().entry.name}</div>
            <div class="files-preview-facts">{previewFacts(p().entry)}</div>
            <div class="files-preview-facts">
                {p().kind === "binary" ? "No preview: not a text file." : ((p() as { reason?: string }).reason ?? "No preview.")}
            </div>
            <Button class="files-button" onClick={() => props.onOpenWithOs(p().entry)}>
                Open with default app
            </Button>
        </div>
    );
}

/** What the panel shows for the current preview. */
export function PreviewBody(props: { preview: Accessor<Preview> } & Open & { onOpenWithOs: (entry: FsEntry) => void }): JSX.Element {
    const preview = props.preview;
    return (
        <Switch>
            <Match when={preview().kind === "none"}>
                <div class="files-preview-empty">Select a file to preview it.</div>
            </Match>
            <Match when={preview().kind === "loading"}>
                <div class="files-preview-empty">Loading…</div>
            </Match>
            <Match when={preview().kind === "folder" && (preview() as Extract<Preview, { kind: "folder" }>)}>
                {(p) => <FolderCard p={p} onOpen={props.onOpen} />}
            </Match>
            <Match when={preview().kind === "image" && (preview() as Extract<Preview, { kind: "image" }>)}>
                {(p) => <ImageMedia p={p} onOpen={props.onOpen} />}
            </Match>
            <Match when={preview().kind === "text" && (preview() as Extract<Preview, { kind: "text" }>)}>
                {(p) => <TextPreview p={p} onOpen={props.onOpen} />}
            </Match>
            <Match when={preview().kind === "av" && (preview() as Extract<Preview, { kind: "av" }>)}>
                {(p) => <AvCard p={p} onOpen={props.onOpen} />}
            </Match>
            <Match when={(preview().kind === "binary" || preview().kind === "other") && (preview() as Extract<Preview, { kind: "binary" | "other" }>)}>
                {(p) => <NoPreviewCard p={p} onOpenWithOs={props.onOpenWithOs} />}
            </Match>
        </Switch>
    );
}
