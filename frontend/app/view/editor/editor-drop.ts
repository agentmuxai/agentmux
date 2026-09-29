// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dropping files onto an editor pane: text files open as tabs; a known binary
 * is refused while hovering; an unknown type is let through and sniffed.
 * docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.3 ("Later panes").
 */

import type { DragFiles, DropVerdict, FileDropHook } from "@/app/drag/file-drop";
import { fileKind } from "@/app/view/agent/attachments/file-kind";

const BINARY_KINDS = new Set(["image", "image_file", "pdf", "word", "excel", "powerpoint", "archive", "audio", "video"]);
const TEXT_MIME = /^(text\/|application\/(json|xml|javascript|x-sh|x-yaml|toml|sql)|image\/svg\+xml$)/;
const SNIFF_BYTES = 8192;
/** The same cap srv's readeditorfile enforces on a path (editor_handlers.rs),
 *  checked before a pathless file's bytes are read into memory. */
export const MAX_DROPPED_TEXT_BYTES = 10_000_000;

const baseName = (path: string) => path.slice(Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\")) + 1);

const isKnownBinaryName = (name: string) => BINARY_KINDS.has(fileKind(undefined, name));

/** Unknown MIME ("") and text types pass; anything else is a known non-text type. */
const isKnownBinaryType = (type: string) => type !== "" && !TEXT_MIME.test(type);

export function editorDropVerdict(drag: DragFiles): DropVerdict {
    if (drag.names) {
        const binary = drag.names.find(isKnownBinaryName);
        if (binary) return { ok: false, reason: `The editor opens text files; ${baseName(binary)} isn't one` };
    } else if (drag.types.some(isKnownBinaryType)) {
        return { ok: false, reason: "The editor opens text files" };
    }
    const message =
        drag.count === 1 && drag.names?.[0] ? `Open ${baseName(drag.names[0])}` : `Open ${drag.count} files`;
    return { ok: true, message, icon: "fa-file-lines" };
}

export interface EditorDropTarget {
    openFile(path: string): Promise<void>;
    /** A file with no host path: its text in a new untitled tab. */
    openText(name: string, content: string): Promise<void>;
    cantOpen(name: string): void;
}

async function looksBinary(file: File): Promise<boolean> {
    const head = new Uint8Array(await file.slice(0, SNIFF_BYTES).arrayBuffer());
    return head.includes(0);
}

export function createEditorDropHook(target: EditorDropTarget): FileDropHook {
    return {
        accept: editorDropVerdict,
        async drop({ paths, files }) {
            if (paths.length > 0) {
                // openFile refuses binary or oversized content itself, on the tab.
                for (const path of paths) await target.openFile(path);
                return;
            }
            for (const file of files) {
                const name = file.name || "file";
                if (file.size > MAX_DROPPED_TEXT_BYTES || isKnownBinaryName(name) || (await looksBinary(file))) {
                    target.cantOpen(name);
                    continue;
                }
                await target.openText(name, await file.text());
            }
        },
    };
}

/**
 * A fresh, empty untitled tab for dropped text. srv may hand back an
 * abandoned scratch that still holds unsaved notes; it's left open (the user
 * gets their notes back) and never overwritten. Each request excludes the
 * scratches already open, so the next one is a different candidate.
 */
export async function openEmptyScratch(
    open: () => Promise<string | undefined>,
    contentOf: (tabId: string) => string | undefined,
    attempts = 5,
): Promise<string | undefined> {
    for (let i = 0; i < attempts; i++) {
        const tabId = await open();
        if (!tabId) return undefined;
        if (!contentOf(tabId)) return tabId;
    }
    return undefined;
}
